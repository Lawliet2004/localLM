//! OS-level execution isolation on Windows: AppContainer (LPAC when the
//! system allows it) + a Job Object. 'enforced' means a process that leaves
//! the app's boundary never runs at all — there is no silent fallback.
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IsolationMode {
    #[default]
    Unsandboxed,
    Enforced,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IsolationSettings {
    #[serde(default)]
    pub mode: IsolationMode,
    /// Spawned processes get the `internetClient` capability. Off means the
    /// AppContainer has no network access at all (including loopback).
    #[serde(default)]
    pub network: bool,
    /// LAN reach — requires `network` and an explicit opt-in.
    #[serde(default)]
    pub lan: bool,
    /// Opt into LPAC (Less Privileged AppContainer). Interpreters typically
    /// cannot read their HKLM registry keys under LPAC, so it is off by
    /// default and the container runs as a regular AppContainer.
    #[serde(default)]
    pub lpac: bool,
    /// Extra directories (interpreters, toolchains) the container can read
    /// and execute. The app also auto-grants the spawn target's own folder.
    #[serde(default)]
    pub toolchain_dirs: Vec<String>,
    #[serde(default = "default_process_limit")]
    pub process_limit: u32,
    #[serde(default = "default_memory_mb")]
    pub memory_mb: u64,
}
fn default_process_limit() -> u32 {
    32
}
fn default_memory_mb() -> u64 {
    2048
}

impl Default for IsolationSettings {
    fn default() -> Self {
        Self {
            mode: IsolationMode::Unsandboxed,
            network: false,
            lan: false,
            lpac: false,
            toolchain_dirs: Vec::new(),
            process_limit: default_process_limit(),
            memory_mb: default_memory_mb(),
        }
    }
}

pub fn settings(store: &crate::store::Store) -> IsolationSettings {
    store.setting("execution.isolation").unwrap_or_default()
}

/// Persist the policy. Leaving 'enforced' revokes the ACEs this module granted.
pub fn save(store: &crate::store::Store, next: &IsolationSettings) -> Result<(), String> {
    #[cfg(windows)]
    if settings(store).mode == IsolationMode::Enforced && next.mode == IsolationMode::Unsandboxed {
        revoke_all(store);
    }
    store.save_setting("execution.isolation", next)
}

/// Everything a spawn needs — built once per call site so the sandboxed and
/// regular paths share the same program/args/env.
pub struct SpawnSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// The env vars to carry into the child (it gets nothing else).
    pub env: Vec<(OsString, OsString)>,
    /// Pipe stdin into the child (false -> NUL).
    pub stdin: bool,
    /// Keep the child's stderr open (false -> NUL, e.g. MCP creds policy).
    pub stderr: bool,
    /// Roots the container may write; the working directory must be inside one.
    pub writable: Vec<PathBuf>,
    /// Job CPU time limit (0 = none). Mirrors the caller's command timeout.
    pub cpu_seconds: u32,
}

impl SpawnSpec {
    /// The standard scrubbed environment every spawn site already used.
    pub fn standard_env(extra: &[(OsString, OsString)]) -> Vec<(OsString, OsString)> {
        // Profile vars included so unsandboxed spawns match the old env list;
        // the enforced path overrides them with a private directory.
        let mut env: Vec<(OsString, OsString)> = [
            "SystemRoot",
            "WINDIR",
            "TEMP",
            "TMP",
            "PATH",
            "PATHEXT",
            "COMSPEC",
            "USERPROFILE",
            "HOME",
            "APPDATA",
            "LOCALAPPDATA",
            "TERM",
            "COLORTERM",
        ]
        .iter()
        .filter_map(|name| std::env::var_os(name).map(|value| (OsString::from(name), value)))
        .collect();
        env.extend(extra.iter().cloned());
        env.push(("PYTHONIOENCODING".into(), "utf-8".into()));
        env.push(("PYTHONUTF8".into(), "1".into()));
        env
    }
    /// Unsandboxed (and non-Windows) spawn — the existing CommandWrap path.
    pub fn command(&self) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(&self.program);
        command
            .args(&self.args)
            .current_dir(&self.cwd)
            .env_clear()
            .stdin(if self.stdin {
                std::process::Stdio::piped()
            } else {
                std::process::Stdio::null()
            })
            .stdout(std::process::Stdio::piped())
            .stderr(if self.stderr {
                std::process::Stdio::piped()
            } else {
                std::process::Stdio::null()
            });
        command.envs(self.env.iter().cloned());
        command
    }
}

/// Spawn under the current policy. `writable` roots are only ACE-granted (and
/// required) when enforced; `db_path` records grants for later revocation.
pub fn spawn(
    spec: &SpawnSpec,
    settings: &IsolationSettings,
    db_path: Option<&Path>,
) -> Result<Box<dyn ChildWrapper>, String> {
    #[cfg(windows)]
    if settings.mode == IsolationMode::Enforced {
        return win::spawn_appcontainer(spec, settings, db_path);
    }
    let _ = db_path;
    let _ = settings;
    let mut command = CommandWrap::from(spec.command());
    command.wrap(KillOnDrop);
    #[cfg(windows)]
    {
        command.wrap(process_wrap::tokio::CreationFlags(
            windows::Win32::System::Threading::CREATE_NO_WINDOW,
        ));
        command.wrap(process_wrap::tokio::JobObject);
    }
    #[cfg(unix)]
    {
        command.wrap(process_wrap::tokio::ProcessGroup::leader());
    }
    command
        .spawn()
        .map_err(|error| format!("Process launch failed: {error}"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IsolationStatus {
    pub available: bool,
    /// 'lpac' | 'appcontainer' once a spawn has been probed.
    pub kind: Option<String>,
    pub reason: String,
    pub policy: IsolationSettings,
    /// ACE grants recorded so far (paths the container SID was given).
    pub grants: Vec<String>,
}

#[tauri::command]
pub fn isolation_status(
    state: tauri::State<'_, crate::AppState>,
) -> Result<IsolationStatus, String> {
    let store = state.database()?;
    Ok(status(&store))
}

fn status(store: &crate::store::Store) -> IsolationStatus {
    let (available, reason) = probe();
    IsolationStatus {
        available,
        kind: last_kind(store),
        reason,
        policy: settings(store),
        grants: store
            .isolation_grants()
            .unwrap_or_default()
            .into_iter()
            .map(|(path, _)| path)
            .collect(),
    }
}

#[tauri::command]
pub fn isolation_get(
    state: tauri::State<'_, crate::AppState>,
) -> Result<IsolationSettings, String> {
    let store = state.database()?;
    Ok(settings(&store))
}

#[tauri::command]
pub fn isolation_save(
    state: tauri::State<'_, crate::AppState>,
    settings: IsolationSettings,
) -> Result<(), String> {
    let store = state.database()?;
    save(&store, &settings)
}

fn last_kind(store: &crate::store::Store) -> Option<String> {
    store
        .setting::<Option<String>>("execution.isolation.kind")
        .ok()
        .flatten()
}

#[cfg(windows)]
fn probe() -> (bool, String) {
    match win::probe_appcontainer() {
        Ok(()) => (true, "AppContainer isolation is available.".into()),
        Err(error) => (
            false,
            format!("AppContainer is unavailable on this system ({error}); enforced mode will refuse to run."),
        ),
    }
}
#[cfg(not(windows))]
fn probe() -> (bool, String) {
    (
        false,
        "AppContainer isolation is only implemented on Windows.".into(),
    )
}

// ---------------------------------------------------------------------------
// In-process file tools: same allowed roots, enforced at the app level.
// cap-std already refuses `..`/device paths and never follows reparse points
// out of the root (see workspace.rs junction tests); this adds the root check
// that pairs a tool call with a task's active workspace.
pub fn canonical_root(path: &Path) -> Result<PathBuf, String> {
    path.canonicalize()
        .map_err(|error| format!("Path does not resolve ({path:?}): {error}"))
}

/// Reject a requested path that canonicalizes outside `root` — the check each
/// in-process tool applies before touching the filesystem. Existing
/// components must not be reparse points whose target leaves the root.
pub fn inside_root(root: &Path, target: &Path) -> bool {
    match (root.canonicalize(), target.canonicalize()) {
        (Ok(root), Ok(target)) => target.starts_with(root),
        // A target that does not exist yet is allowed only when its nearest
        // existing ancestor stays inside the root.
        (Ok(root), Err(_)) => nearest_existing(target).is_some_and(|p| p.starts_with(&root)),
        _ => false,
    }
}

fn nearest_existing(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|p| p.exists())
        .and_then(|p| p.canonicalize().ok())
}

/// Network tools the execution policy disables when enforced without network
/// (they run in this process, so the OS boundary cannot cover them — the app
/// blocks them instead and says so).
pub fn network_blocked_alias(alias: &str) -> bool {
    matches!(
        alias,
        "browser" | "web_search" | "web_fetch" | "daytona_exec" | "daytona_run"
    )
}

pub const NETWORK_BLOCKED_REASON: &str = "Network blocked by execution policy";

/// True when in-process network tooling must be disabled.
pub fn agent_network_blocked(settings: &IsolationSettings) -> bool {
    settings.mode == IsolationMode::Enforced && !settings.network
}

#[cfg(windows)]
pub fn revoke_all(store: &crate::store::Store) {
    // Deleting the sandbox profiles (mappings + package folders) also makes
    // their SIDs useless for any leftover ACE a revoke could not reach.
    let removed = win::purge_stale_profiles();
    if removed > 0 {
        eprintln!("[isolation] removed {removed} stale sandbox profiles");
    }
    for (path, name) in store.isolation_grants().unwrap_or_default() {
        if let Ok(sid) = win::profile_sid(&name) {
            let _ = win::remove_grant(Path::new(&path), sid);
            unsafe {
                let _ = windows::Win32::Security::FreeSid(sid);
            }
        }
    }
    let _ = store.clear_isolation_grants();
}
#[cfg(not(windows))]
pub fn revoke_all(_store: &crate::store::Store) {}

#[cfg(windows)]
mod win {
    use super::*;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::{
        CloseHandle, LocalFree, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT, HLOCAL,
        WAIT_OBJECT_0,
    };
    use windows::Win32::Security::Authorization::{
        BuildTrusteeWithSidW, GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW,
        EXPLICIT_ACCESS_W, GRANT_ACCESS, REVOKE_ACCESS, SE_FILE_OBJECT, TRUSTEE_W,
    };
    use windows::Win32::Security::Isolation::DeriveAppContainerSidFromAppContainerName;
    use windows::Win32::Security::{
        DeriveCapabilitySidsFromName, FreeSid, IsValidSid, ACL, CONTAINER_INHERIT_ACE,
        DACL_SECURITY_INFORMATION, OBJECT_INHERIT_ACE, PSID, SECURITY_ATTRIBUTES,
        SECURITY_CAPABILITIES, SID_AND_ATTRIBUTES,
    };
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ALL_ACCESS, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicUIRestrictions,
        JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject,
        JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_JOB_MEMORY,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows::Win32::System::Pipes::CreatePipe;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER,
        KEY_READ,
    };
    use windows::Win32::System::SystemServices::JOB_OBJECT_UILIMIT_ALL;
    use windows::Win32::System::Threading::{
        CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
        InitializeProcThreadAttributeList, UpdateProcThreadAttribute, WaitForSingleObject,
        CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT,
        PROCESS_CREATION_FLAGS, PROCESS_INFORMATION,
        PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
        PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    };
    use windows::Win32::System::WindowsProgramming::PROCESS_CREATION_ALL_APPLICATION_PACKAGES_OPT_OUT;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    // -- SID plumbing -------------------------------------------------------

    /// Deterministic profile per workspace root so granted ACEs survive restarts.
    pub fn profile_name(root: &Path) -> String {
        let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let hash = crate::task_workspace::sha256_bytes(canonical.to_string_lossy().as_bytes());
        format!("LocalLM.Sandbox.{}", &hash[..8])
    }

    /// Create (or reuse) the AppContainer profile and return its SID. A bare
    /// Derive without a registered profile yields a token Windows treats as
    /// nonfunctional — the profile registration is required.
    pub fn profile_sid(name: &str) -> Result<PSID, String> {
        unsafe {
            use windows::Win32::Security::Isolation::CreateAppContainerProfile;
            let name_w = wide(name);
            match CreateAppContainerProfile(
                PCWSTR(name_w.as_ptr()),
                PCWSTR(wide("LocalLM sandbox").as_ptr()),
                PCWSTR(wide("Isolated LocalLM process").as_ptr()),
                None,
            ) {
                Ok(sid) => Ok(sid),
                // Profile already exists -> derive the same SID.
                Err(_) => DeriveAppContainerSidFromAppContainerName(PCWSTR(name_w.as_ptr()))
                    .map_err(|error| {
                        format!("Could not create or derive the AppContainer profile: {error}")
                    }),
            }
        }
    }

    /// Delete a registered profile (removes its registry mapping and package
    /// folder). The profile name is deterministic per workspace root, so a
    /// deleted profile simply gets recreated on the next sandboxed spawn.
    pub fn delete_profile(name: &str) -> Result<(), String> {
        use windows::Win32::Security::Isolation::DeleteAppContainerProfile;
        unsafe {
            DeleteAppContainerProfile(PCWSTR(wide(name).as_ptr()))
                .map_err(|error| format!("Could not delete profile {name}: {error}"))
        }
    }

    /// Remove every registered `locallm.sandbox` profile — stale leftovers
    /// from tests or deleted workspaces, plus the matching package folders
    /// under AppData\Local\Packages (DeleteAppContainerProfile removes both).
    /// Returns the number of profiles removed.
    pub fn purge_stale_profiles() -> u32 {
        const BASE: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppContainer\Mappings";
        let base_w = wide(BASE);
        let mut key = HKEY::default();
        if unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(base_w.as_ptr()),
                None,
                KEY_READ,
                &mut key,
            )
        }
        .is_err()
        {
            return 0;
        }
        let mut removed = 0;
        let mut index = 0u32;
        loop {
            let mut name = [0u16; 128];
            let mut len = name.len() as u32;
            let status = unsafe {
                RegEnumKeyExW(
                    key,
                    index,
                    Some(PWSTR(name.as_mut_ptr())),
                    &mut len,
                    None,
                    None,
                    None,
                    None,
                )
            };
            if status.is_err() {
                break;
            }
            index += 1;
            let sid_w = wide(&String::from_utf16_lossy(&name[..len as usize]));
            let mut subkey = HKEY::default();
            if unsafe { RegOpenKeyExW(key, PCWSTR(sid_w.as_ptr()), None, KEY_READ, &mut subkey) }
                .is_err()
            {
                continue;
            }
            let mut value = [0u8; 512];
            let mut value_len = value.len() as u32;
            let queried = unsafe {
                RegQueryValueExW(
                    subkey,
                    PCWSTR(wide("Moniker").as_ptr()),
                    None,
                    None,
                    Some(value.as_mut_ptr()),
                    Some(&mut value_len),
                )
            };
            let _ = unsafe { RegCloseKey(subkey) };
            if queried.is_err() || value_len < 2 {
                continue;
            }
            let units = (value_len as usize / 2).saturating_sub(1);
            let moniker: String = String::from_utf16_lossy(unsafe {
                std::slice::from_raw_parts(value.as_ptr() as *const u16, units)
            });
            if moniker.to_ascii_lowercase().starts_with("locallm.sandbox")
                && delete_profile(&moniker).is_ok()
            {
                removed += 1;
            }
        }
        let _ = unsafe { RegCloseKey(key) };
        removed
    }

    /// internetClient / privateNetworkClientServer capability SID.
    fn capability_sid(name: &str) -> Result<PSID, String> {
        unsafe {
            let name_w = wide(name);
            let mut group_sids: *mut PSID = std::ptr::null_mut();
            let mut group_count = 0u32;
            let mut name_sids: *mut PSID = std::ptr::null_mut();
            let mut name_count = 0u32;
            DeriveCapabilitySidsFromName(
                PCWSTR(name_w.as_ptr()),
                &mut group_sids,
                &mut group_count,
                &mut name_sids,
                &mut name_count,
            )
            .map_err(|error| format!("Could not derive the {name} capability: {error}"))?;
            if name_count == 0 || name_sids.is_null() {
                return Err(format!("The {name} capability is not available."));
            }
            // Own the first name SID; free the remainder + group SIDs.
            let sid = *name_sids;
            for index in 1..name_count {
                let _ = FreeSid(*name_sids.add(index as usize));
            }
            for index in 0..group_count {
                let _ = FreeSid(*group_sids.add(index as usize));
            }
            LocalFree(Some(HLOCAL(name_sids as _)));
            if !group_sids.is_null() {
                LocalFree(Some(HLOCAL(group_sids as _)));
            }
            Ok(sid)
        }
    }

    // -- ACE grants -----------------------------------------------------------

    /// Add (GRANT_ACCESS) or remove (REVOKE_ACCESS) an inheritable ACE for the
    /// container SID on a directory. Returns the canonical granted path.
    pub fn update_grant(
        root: &Path,
        sid: PSID,
        access: u32,
        revoke: bool,
    ) -> Result<String, String> {
        update_grant_ex(root, sid, access, revoke, true)
    }

    /// `inheritable` controls OI|CI on the ACE — ancestor traverse grants use
    /// `false` so the grant cannot leak onto unrelated children.
    pub fn update_grant_ex(
        root: &Path,
        sid: PSID,
        access: u32,
        revoke: bool,
        inheritable: bool,
    ) -> Result<String, String> {
        let path = root.canonicalize().map_err(|error| error.to_string())?;
        // SetNamedSecurityInfoW can stall on verbatim (\\?\) paths.
        let path_text = path.to_string_lossy().replace("\\\\?\\", "");
        unsafe {
            let mut trustee = TRUSTEE_W::default();
            BuildTrusteeWithSidW(&mut trustee, Some(sid));
            let entry = EXPLICIT_ACCESS_W {
                grfAccessMode: if revoke { REVOKE_ACCESS } else { GRANT_ACCESS },
                grfAccessPermissions: access,
                grfInheritance: if inheritable {
                    OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE
                } else {
                    windows::Win32::Security::NO_INHERITANCE
                },
                Trustee: trustee,
            };

            let path_w = wide(&path_text);
            let mut sd = windows::Win32::Security::PSECURITY_DESCRIPTOR(std::ptr::null_mut());
            let mut old_dacl: *mut ACL = std::ptr::null_mut();
            let mut error = GetNamedSecurityInfoW(
                PCWSTR(path_w.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(&mut old_dacl),
                None,
                &mut sd,
            );
            let mut new_dacl: *mut ACL = std::ptr::null_mut();
            if error.is_ok() {
                error = SetEntriesInAclW(
                    Some(&[entry]),
                    if old_dacl.is_null() {
                        None
                    } else {
                        Some(old_dacl)
                    },
                    &mut new_dacl,
                );
            }
            if error.is_ok() {
                error = SetNamedSecurityInfoW(
                    PCWSTR(path_w.as_ptr()),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    None,
                    None,
                    Some(new_dacl),
                    None,
                );
            }
            if !new_dacl.is_null() {
                LocalFree(Some(HLOCAL(new_dacl as _)));
            }
            if !sd.0.is_null() {
                LocalFree(Some(HLOCAL(sd.0)));
            }
            if error.is_err() {
                return Err(format!(
                    "Could not update file permissions on {}: {}",
                    path.display(),
                    error.to_hresult().message()
                ));
            }
        }
        Ok(path.to_string_lossy().into_owned())
    }

    /// Grant (or revoke) `access` on exactly one object — SetFileSecurityW
    /// writes the DACL verbatim and never walks the tree. Ancestor traverse
    /// grants must use this: SetNamedSecurityInfoW re-propagates inherited
    /// ACEs down the whole subtree even when the new ACE is non-inheritable
    /// (minutes on directories like AppData).
    pub fn update_grant_no_propagate(
        path: &Path,
        sid: PSID,
        access: u32,
        revoke: bool,
    ) -> Result<String, String> {
        use windows::Win32::Security::SetFileSecurityW;
        use windows::Win32::Security::{
            InitializeSecurityDescriptor, SetSecurityDescriptorDacl, SECURITY_DESCRIPTOR,
        };
        use windows::Win32::System::SystemServices::SECURITY_DESCRIPTOR_REVISION;
        let canon = path.canonicalize().map_err(|error| error.to_string())?;
        let path_text = canon.to_string_lossy().replace("\\\\?\\", "");
        unsafe {
            let mut trustee = TRUSTEE_W::default();
            BuildTrusteeWithSidW(&mut trustee, Some(sid));
            let entry = EXPLICIT_ACCESS_W {
                grfAccessMode: if revoke { REVOKE_ACCESS } else { GRANT_ACCESS },
                grfAccessPermissions: access,
                grfInheritance: windows::Win32::Security::NO_INHERITANCE,
                Trustee: trustee,
            };

            let path_w = wide(&path_text);
            let mut sd = windows::Win32::Security::PSECURITY_DESCRIPTOR(std::ptr::null_mut());
            let mut old_dacl: *mut ACL = std::ptr::null_mut();
            let mut error = GetNamedSecurityInfoW(
                PCWSTR(path_w.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(&mut old_dacl),
                None,
                &mut sd,
            );
            let mut new_dacl: *mut ACL = std::ptr::null_mut();
            if error.is_ok() {
                error = SetEntriesInAclW(
                    Some(&[entry]),
                    if old_dacl.is_null() {
                        None
                    } else {
                        Some(old_dacl)
                    },
                    &mut new_dacl,
                );
            }
            if error.is_ok() {
                let mut descriptor = SECURITY_DESCRIPTOR::default();
                let descriptor_ptr = windows::Win32::Security::PSECURITY_DESCRIPTOR(
                    &mut descriptor as *mut _ as *mut _,
                );
                if InitializeSecurityDescriptor(descriptor_ptr, SECURITY_DESCRIPTOR_REVISION)
                    .is_err()
                    || SetSecurityDescriptorDacl(descriptor_ptr, true, Some(new_dacl), false)
                        .is_err()
                    || !SetFileSecurityW(
                        PCWSTR(path_w.as_ptr()),
                        DACL_SECURITY_INFORMATION,
                        descriptor_ptr,
                    )
                    .as_bool()
                {
                    error = windows::Win32::Foundation::WIN32_ERROR(
                        windows::Win32::Foundation::GetLastError().0,
                    );
                }
            }
            if !new_dacl.is_null() {
                LocalFree(Some(HLOCAL(new_dacl as _)));
            }
            if !sd.0.is_null() {
                LocalFree(Some(HLOCAL(sd.0)));
            }
            if error.is_err() {
                return Err(format!(
                    "Could not update file permissions on {}: {}",
                    canon.display(),
                    error.to_hresult().message()
                ));
            }
        }
        Ok(canon.to_string_lossy().into_owned())
    }

    pub fn remove_grant(root: &Path, sid: PSID) -> Result<(), String> {
        update_grant(root, sid, FILE_ALL_ACCESS.0, true).map(|_| ())
    }

    pub fn probe_appcontainer() -> Result<(), String> {
        let sid = profile_sid(&profile_name(Path::new("probe")))?;
        unsafe {
            if !IsValidSid(sid).as_bool() {
                return Err("the derived AppContainer SID is invalid".into());
            }
            let _ = FreeSid(sid);
        }
        Ok(())
    }

    // -- Spawn ----------------------------------------------------------------

    /// A stdio pipe whose `child` end is inheritable (the HANDLE_LIST passes
    /// it in) and `parent` is not. `writable` = the child writes, parent reads.
    struct Pipe {
        parent: OwnedHandle,
        child: HANDLE,
    }

    fn pipe(writable: bool) -> Result<Pipe, String> {
        unsafe {
            let attributes = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                bInheritHandle: true.into(),
                lpSecurityDescriptor: std::ptr::null_mut(),
            };
            let mut read = HANDLE::default();
            let mut write = HANDLE::default();
            CreatePipe(&mut read, &mut write, Some(&attributes as *const _), 0)
                .map_err(|error| format!("Could not create a stdio pipe: {error}"))?;
            let (parent, child) = if writable {
                (read, write)
            } else {
                (write, read)
            };
            // The parent end must not be inherited, or the pipe never reaches EOF.
            SetHandleInformation(
                parent,
                HANDLE_FLAG_INHERIT.0,
                windows::Win32::Foundation::HANDLE_FLAGS(0),
            )
            .map_err(|error| format!("Could not mark the pipe end: {error}"))?;
            Ok(Pipe {
                parent: OwnedHandle::from_raw_handle(parent.0),
                child,
            })
        }
    }

    fn nul_handle() -> Result<HANDLE, String> {
        unsafe {
            CreateFileW(
                windows::core::w!("NUL"),
                (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                Some(&SECURITY_ATTRIBUTES {
                    nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                    bInheritHandle: true.into(),
                    lpSecurityDescriptor: std::ptr::null_mut(),
                } as *const _),
                OPEN_EXISTING,
                windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
            .map_err(|error| format!("Could not open the NUL device: {error}"))
        }
    }

    /// Command-line quoting the way `CommandLineToArgvW` reads it.
    fn is_powershell(program: &str) -> bool {
        let name = Path::new(program)
            .file_name()
            .map(|name| name.to_string_lossy().to_ascii_lowercase());
        matches!(name.as_deref(), Some("powershell.exe" | "pwsh.exe"))
    }

    /// AppContainer processes cannot change their current directory — the OS
    /// denies SetCurrentDirectory outright — so Set-Location to a DOS path
    /// always fails. A PSDrive rooted at the workspace bypasses ancestor
    /// resolution: `LL:\file` opens the granted root directly. `-Command -`
    /// (stdin) gets the same lines as its first stdin input instead.
    fn set_location(cwd_text: &str) -> String {
        format!(
            "New-PSDrive -Name LL -PSProvider FileSystem -Root '{}' | Out-Null; Set-Location LL:\\",
            cwd_text.replace('\'', "''")
        )
    }

    fn effective_args(spec: &SpawnSpec, cwd_text: &str) -> Vec<String> {
        let mut args = spec.args.clone();
        if is_powershell(&spec.program) {
            if let Some(index) = args
                .iter()
                .position(|arg| arg.eq_ignore_ascii_case("-Command"))
            {
                if let Some(script) = args
                    .get_mut(index + 1)
                    .filter(|script| script.as_str() != "-")
                {
                    *script = format!("{}; {}", set_location(cwd_text), script);
                }
            }
        }
        args
    }

    fn command_line(spec: &SpawnSpec, cwd_text: &str) -> String {
        let mut line = String::new();
        let args = effective_args(spec, cwd_text);
        for (index, part) in std::iter::once(&spec.program)
            .chain(args.iter())
            .enumerate()
        {
            if index > 0 {
                line.push(' ');
            }
            if part.is_empty() || part.contains(|c: char| c.is_whitespace() || c == '"') {
                line.push('"');
                let mut backslashes = 0usize;
                for ch in part.chars() {
                    match ch {
                        '\\' => backslashes += 1,
                        '"' => {
                            line.push_str(&"\\".repeat(backslashes * 2 + 1));
                            line.push('"');
                            backslashes = 0;
                        }
                        _ => {
                            line.push_str(&"\\".repeat(backslashes));
                            line.push(ch);
                            backslashes = 0;
                        }
                    }
                }
                line.push_str(&"\\".repeat(backslashes * 2));
                line.push('"');
            } else {
                line.push_str(part);
            }
        }
        line
    }

    /// STARTUPINFOEX + SECURITY_CAPABILITIES + an explicit stdio HANDLE_LIST.
    /// `lpac` selects the LPAC policy (0) vs. a regular AppContainer (OPT_OUT).
    unsafe fn create_process(
        spec: &SpawnSpec,
        env_block: &[u16],
        handles: &[HANDLE; 3],
        capabilities: &[SID_AND_ATTRIBUTES],
        sid: PSID,
        lpac: bool,
    ) -> Result<PROCESS_INFORMATION, String> {
        let mut security = SECURITY_CAPABILITIES {
            AppContainerSid: sid,
            // Empty Vec::as_ptr is a non-null dangling address — CreateProcessW
            // rejects it; NULL is the "no capabilities" representation.
            Capabilities: if capabilities.is_empty() {
                std::ptr::null_mut()
            } else {
                capabilities.as_ptr() as *mut _
            },
            CapabilityCount: capabilities.len() as u32,
            Reserved: 0,
        };
        let mut size = 0usize;
        let _ = InitializeProcThreadAttributeList(None, 3, None, &mut size);
        // Aligned storage — the list contains pointers; a Vec<u8> is only
        // byte-aligned and CreateProcessW rejects it with EINVAL.
        let mut buffer = vec![0u64; size.div_ceil(8)];
        let list = windows::Win32::System::Threading::LPPROC_THREAD_ATTRIBUTE_LIST(
            buffer.as_mut_ptr() as _,
        );
        InitializeProcThreadAttributeList(Some(list), 3, None, &mut size)
            .map_err(|error| format!("Could not build process attributes: {error}"))?;
        let spawned = (|| -> Result<PROCESS_INFORMATION, String> {
            UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
                Some(&security as *const _ as *const _),
                std::mem::size_of::<SECURITY_CAPABILITIES>(),
                None,
                None,
            )
            .map_err(|error| format!("Could not set the container attribute: {error}"))?;
            // PROCESS_CREATION_ALL_APPLICATION_PACKAGES_OPT_OUT opts the
            // child OUT of ALL_APPLICATION_PACKAGES — that IS the LPAC flag.
            // Plain value 0 keeps the all-app-packages group (regular AC).
            let policy = if lpac {
                PROCESS_CREATION_ALL_APPLICATION_PACKAGES_OPT_OUT
            } else {
                0u32
            };
            UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY as usize,
                Some(&policy as *const _ as *const _),
                std::mem::size_of::<u32>(),
                None,
                None,
            )
            .map_err(|error| format!("Could not set the container policy: {error}"))?;
            UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                Some(handles.as_ptr() as *const _),
                handles.len() * std::mem::size_of::<HANDLE>(),
                None,
                None,
            )
            .map_err(|error| format!("Could not set the handle list: {error}"))?;
            let _ = &mut security;
            let mut info = STARTUPINFOEXW::default();
            info.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
            info.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
            info.StartupInfo.hStdInput = handles[0];
            info.StartupInfo.hStdOutput = handles[1];
            info.StartupInfo.hStdError = handles[2];
            info.lpAttributeList = list;
            // CreateProcessW rejects verbatim (\\?\) current directories, and
            // 8.3 short-name components force a directory listing the child
            // may not have — hand it the canonical long path instead.
            let cwd_text = spec
                .cwd
                .canonicalize()
                .unwrap_or_else(|_| spec.cwd.clone())
                .to_string_lossy()
                .replace("\\\\?\\", "");
            let cwd_w = wide(&cwd_text);
            let mut line = wide(&command_line(spec, &cwd_text));
            let mut process = PROCESS_INFORMATION::default();
            CreateProcessW(
                None,
                Some(PWSTR(line.as_mut_ptr())),
                None,
                None,
                true,
                PROCESS_CREATION_FLAGS(
                    EXTENDED_STARTUPINFO_PRESENT.0
                        | CREATE_NO_WINDOW.0
                        | CREATE_UNICODE_ENVIRONMENT.0,
                ),
                Some(env_block.as_ptr() as *const _),
                PCWSTR(cwd_w.as_ptr()),
                &info.StartupInfo,
                &mut process,
            )
            .map_err(|error| format!("{error}"))?;
            Ok(process)
        })();
        DeleteProcThreadAttributeList(list);
        spawned
    }

    fn make_job(settings: &IsolationSettings, cpu_seconds: u32) -> Result<OwnedHandle, String> {
        unsafe {
            let job = CreateJobObjectW(None, PCWSTR::null())
                .map_err(|error| format!("Could not create the process job: {error}"))?;
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.ActiveProcessLimit = settings.process_limit;
            limits.JobMemoryLimit = settings.memory_mb.saturating_mul(1024 * 1024) as usize;
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
                | JOB_OBJECT_LIMIT_JOB_MEMORY;
            if cpu_seconds > 0 {
                limits.BasicLimitInformation.PerProcessUserTimeLimit =
                    (cpu_seconds as i64).saturating_mul(10_000_000);
                limits.BasicLimitInformation.LimitFlags |=
                    windows::Win32::System::JobObjects::JOB_OBJECT_LIMIT_PROCESS_TIME;
            }
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            )
            .map_err(|error| {
                format!(
                    "Could not limit the process job (flags={:?}, memory={}, procs={}): {error}",
                    limits.BasicLimitInformation.LimitFlags,
                    limits.JobMemoryLimit,
                    limits.BasicLimitInformation.ActiveProcessLimit
                )
            })?;
            let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
                UIRestrictionsClass: windows::Win32::System::JobObjects::JOB_OBJECT_UILIMIT(
                    JOB_OBJECT_UILIMIT_ALL,
                ),
            };
            SetInformationJobObject(
                job,
                JobObjectBasicUIRestrictions,
                &ui as *const _ as *const _,
                std::mem::size_of_val(&ui) as u32,
            )
            .map_err(|error| format!("Could not restrict the process job: {error}"))?;
            Ok(OwnedHandle::from_raw_handle(job.0))
        }
    }

    /// A spawned AppContainer process. The job handle kills the whole tree on
    /// drop (KILL_ON_JOB_CLOSE), so cancel/panic releases descendants too.
    pub struct SandboxedChild {
        process: OwnedHandle,
        job: OwnedHandle,
        pid: u32,
        pub stdin: Option<tokio::process::ChildStdin>,
        pub stdout: Option<tokio::process::ChildStdout>,
        pub stderr: Option<tokio::process::ChildStderr>,
    }
    impl std::fmt::Debug for SandboxedChild {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("SandboxedChild")
                .field("pid", &self.pid)
                .finish()
        }
    }
    impl Drop for SandboxedChild {
        fn drop(&mut self) {
            unsafe {
                let _ = TerminateJobObject(HANDLE(self.job.as_raw_handle()), 1);
            }
        }
    }
    impl SandboxedChild {
        fn exit_status(&self) -> std::process::ExitStatus {
            unsafe {
                let mut code = 0u32;
                let _ = GetExitCodeProcess(HANDLE(self.process.as_raw_handle()), &mut code);
                std::os::windows::process::ExitStatusExt::from_raw(code)
            }
        }
    }
    impl ChildWrapper for SandboxedChild {
        fn inner(&self) -> &dyn ChildWrapper {
            self
        }
        fn inner_mut(&mut self) -> &mut dyn ChildWrapper {
            self
        }
        fn into_inner(self: Box<Self>) -> Box<dyn ChildWrapper> {
            self
        }
        fn stdin(&mut self) -> &mut Option<tokio::process::ChildStdin> {
            &mut self.stdin
        }
        fn stdout(&mut self) -> &mut Option<tokio::process::ChildStdout> {
            &mut self.stdout
        }
        fn stderr(&mut self) -> &mut Option<tokio::process::ChildStderr> {
            &mut self.stderr
        }
        fn id(&self) -> Option<u32> {
            Some(self.pid)
        }
        fn process_handle(&self) -> Option<std::os::windows::io::BorrowedHandle<'_>> {
            unsafe {
                Some(std::os::windows::io::BorrowedHandle::borrow_raw(
                    self.process.as_raw_handle(),
                ))
            }
        }
        fn start_kill(&mut self) -> io::Result<()> {
            unsafe {
                TerminateJobObject(HANDLE(self.job.as_raw_handle()), 1)
                    .map_err(|error| io::Error::other(format!("{error}")))
            }
        }
        fn try_wait(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
            unsafe {
                let process = HANDLE(self.process.as_raw_handle());
                if WaitForSingleObject(process, 0) == WAIT_OBJECT_0 {
                    Ok(Some(self.exit_status()))
                } else {
                    Ok(None)
                }
            }
        }
        fn wait(
            &mut self,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = io::Result<std::process::ExitStatus>> + Send + '_>,
        > {
            let raw = self.process.as_raw_handle() as usize;
            Box::pin(async move {
                let code = tokio::task::spawn_blocking(move || unsafe {
                    let process = HANDLE(raw as _);
                    WaitForSingleObject(process, u32::MAX);
                    let mut code = 0u32;
                    let _ = GetExitCodeProcess(process, &mut code);
                    code
                })
                .await
                .map_err(|error| {
                    io::Error::other(format!("Waiting on the sandboxed process failed: {error}"))
                })?;
                Ok(std::os::windows::process::ExitStatusExt::from_raw(code))
            })
        }
    }

    /// Profile -> ACE grants -> env -> pipes -> CreateProcessW -> job. Any
    /// failure refuses to run rather than silently running unsandboxed.
    /// `store` records ACE grants (for later revocation) and the LPAC/AppContainer kind.
    pub fn spawn_appcontainer(
        spec: &SpawnSpec,
        settings: &IsolationSettings,
        db_path: Option<&Path>,
    ) -> Result<Box<dyn ChildWrapper>, String> {
        let cwd = spec
            .cwd
            .canonicalize()
            .map_err(|error| format!("The working directory does not resolve: {error}"))?;
        let writable: Vec<PathBuf> = spec
            .writable
            .iter()
            .map(|path| {
                path.canonicalize()
                    .map_err(|error| format!("A sandbox root does not resolve: {error}"))
            })
            .collect::<Result<_, _>>()?;
        if writable.is_empty() {
            return Err("Enforced isolation needs at least one writable root.".into());
        }
        if !writable.iter().any(|root| cwd.starts_with(root)) {
            return Err(format!(
                "The working directory {} is outside every sandbox root.",
                cwd.display()
            ));
        }
        let name = profile_name(&writable[0]);
        let sid = profile_sid(&name)?;

        // ACEs: writable roots get full access; the program's own folder and
        // configured toolchain dirs get read+execute. A grant failure refuses
        // the run — a half-open container is worse than none.
        let read_execute = (FILE_GENERIC_READ | FILE_GENERIC_EXECUTE).0;
        let store = db_path.and_then(|path| crate::store::Store::open(path).ok());
        // Writable roots are required — without the ACE the workspace is
        // unreadable and the run is a guaranteed failure, so refuse early.
        // OI|CI ACEs propagate to existing descendants when written — a big
        // workspace pays a one-time crawl; new children inherit at creation.
        for root in &writable {
            let granted = update_grant(root, sid, FILE_ALL_ACCESS.0, false)?;
            if let Some(store) = &store {
                let _ = store.record_isolation_grant(&granted, &name);
            }
            // Absolute-path opens (and runtimes that resolve the cwd string)
            // need traverse on every ancestor of the root — Windows does not
            // give the container SeChangeNotifyPrivilege. Traverse + read-
            // attributes on each ancestor folder itself only, never inherited:
            // no listing, no file contents, no access to siblings. Best-
            // effort: the user may not own e.g. C:\Users.
            let ancestor_access = FILE_GENERIC_EXECUTE.0;
            for ancestor in root.ancestors().skip(1) {
                match update_grant_no_propagate(ancestor, sid, ancestor_access, false) {
                    Ok(granted) => {
                        if let Some(store) = &store {
                            let _ = store.record_isolation_grant(&granted, &name);
                        }
                    }
                    Err(error) => eprintln!(
                        "[isolation] ancestor grant skipped for {}: {error}",
                        ancestor.display()
                    ),
                }
            }
        }
        // Read+execute roots are best-effort: system and toolchain dirs are
        // typically readable by ALL_APPLICATION_PACKAGES already, and the
        // caller may not own them. A dir that genuinely needs the ACE fails
        // later at process start with the real error, not a hidden grant gap.
        let mut readonly_roots: Vec<PathBuf> = Vec::new();
        if let Some(folder) = Path::new(&spec.program).parent().map(Path::to_path_buf) {
            if !writable.iter().any(|root| folder.starts_with(root)) {
                readonly_roots.push(folder);
            }
        }
        readonly_roots.extend(
            settings
                .toolchain_dirs
                .iter()
                .map(PathBuf::from)
                .filter(|dir| dir.is_dir() && !writable.iter().any(|root| dir.starts_with(root))),
        );
        for root in readonly_roots {
            match update_grant(&root, sid, read_execute, false) {
                Ok(granted) => {
                    if let Some(store) = &store {
                        let _ = store.record_isolation_grant(&granted, &name);
                    }
                }
                Err(error) => eprintln!(
                    "[isolation] read+execute grant skipped for {}: {error}",
                    root.display()
                ),
            }
        }

        // Env block: Windows AppContainer process creation requires the full
        // standard environment (a stripped block fails with ENVVAR_NOT_FOUND),
        // so start from the inherited env, point profile vars at a private
        // directory inside the writable root, and apply spec overrides.
        let temp_dir = writable[0].join(".locallm-tmp");
        let _ = std::fs::create_dir_all(&temp_dir);
        // Verbatim paths in env vars confuse cmd/powershell — strip \\?\.
        let temp_text = temp_dir.to_string_lossy().replace("\\\\?\\", "");
        let temp_dir = PathBuf::from(temp_text);
        let overrides: Vec<(OsString, OsString)> = [
            "TEMP",
            "TMP",
            "USERPROFILE",
            "HOME",
            "APPDATA",
            "LOCALAPPDATA",
        ]
        .iter()
        .map(|name| (OsString::from(name), temp_dir.clone().into_os_string()))
        .chain(spec.env.iter().cloned())
        .collect();
        let overridden: std::collections::HashSet<String> = overrides
            .iter()
            .map(|(key, _)| key.to_string_lossy().to_ascii_uppercase())
            .collect();
        let mut env: Vec<(OsString, OsString)> = std::env::vars_os()
            .filter(|(key, _)| !overridden.contains(&key.to_string_lossy().to_ascii_uppercase()))
            .collect();
        env.extend(overrides);
        // CreateProcessW resolves required entries binary-search-style:
        // env blocks must be sorted by name (case-insensitive).
        env.sort_by(|a, b| {
            a.0.to_string_lossy()
                .to_ascii_uppercase()
                .cmp(&b.0.to_string_lossy().to_ascii_uppercase())
        });
        let mut block: Vec<u16> = Vec::new();
        for (key, value) in &env {
            block.extend(
                key.encode_wide()
                    .chain([u16::from(b'=')])
                    .chain(value.encode_wide())
                    .chain([0]),
            );
        }
        block.push(0);

        let mut capabilities: Vec<SID_AND_ATTRIBUTES> = Vec::new();
        if settings.network {
            capabilities.push(SID_AND_ATTRIBUTES {
                Sid: capability_sid("internetClient")?,
                Attributes: 0,
            });
            if settings.lan {
                capabilities.push(SID_AND_ATTRIBUTES {
                    Sid: capability_sid("privateNetworkClientServer")?,
                    Attributes: 0,
                });
            }
        }

        // stdio: pipes for streams the caller reads/writes, NUL for the rest.
        let stdin_pipe = spec.stdin.then(|| pipe(false)).transpose()?;
        let stdout_pipe = pipe(true)?;
        let stderr_pipe = spec.stderr.then(|| pipe(true)).transpose()?;
        let nul_in = (!spec.stdin).then(nul_handle).transpose()?;
        let nul_err = (!spec.stderr).then(nul_handle).transpose()?;
        let handles = [
            stdin_pipe
                .as_ref()
                .map(|pipe| pipe.child)
                .unwrap_or_else(|| nul_in.unwrap_or_default()),
            stdout_pipe.child,
            stderr_pipe
                .as_ref()
                .map(|pipe| pipe.child)
                .unwrap_or_else(|| nul_err.unwrap_or_default()),
        ];

        // Regular AppContainer first: LPAC strips ALL_APPLICATION_PACKAGES
        // reads, which kills interpreters (powershell/python cannot open their
        // HKLM registry keys without an admin ACL grant). `isolation.lpac`
        // opts into the stricter policy for tools that survive it; either way
        // a failed launch refuses to run — never silent unsandboxed.
        let order = if settings.lpac {
            [true, false]
        } else {
            [false, true]
        };
        let mut create_err = String::new();
        let mut spawned = None;
        for lpac in order {
            match unsafe { create_process(spec, &block, &handles, &capabilities, sid, lpac) } {
                Ok(process) => {
                    spawned = Some((process, if lpac { "lpac" } else { "appcontainer" }));
                    break;
                }
                Err(error) => {
                    create_err = format!(
                        "{create_err}{}: {error}. ",
                        if lpac { "LPAC" } else { "AppContainer" }
                    );
                }
            }
        }
        let (process, kind) = spawned.ok_or_else(|| {
            format!("Sandboxed launch failed ({create_err})Enforced isolation refuses to run unsandboxed.")
        })?;
        if let Some(store) = &store {
            let _ = store.save_setting("execution.isolation.kind", &Some(kind.to_string()));
        }

        let job = make_job(settings, spec.cpu_seconds)?;
        unsafe {
            AssignProcessToJobObject(HANDLE(job.as_raw_handle()), process.hProcess).map_err(
                |error| format!("Could not confine the spawned process to its job: {error}"),
            )?;
        }

        let mut child = SandboxedChild {
            process: unsafe { OwnedHandle::from_raw_handle(process.hProcess.0) },
            job,
            pid: process.dwProcessId,
            stdin: None,
            stdout: None,
            stderr: None,
        };
        if let Some(pipe) = stdin_pipe {
            let mut file = std::fs::File::from(pipe.parent);
            let reads_stdin = spec
                .args
                .windows(2)
                .any(|pair| pair[0].eq_ignore_ascii_case("-Command") && pair[1] == "-");
            if is_powershell(&spec.program) && reads_stdin {
                let cwd_text = cwd.to_string_lossy().replace("\\\\?\\", "");
                std::io::Write::write_all(
                    &mut file,
                    format!("{}\n", set_location(&cwd_text)).as_bytes(),
                )
                .map_err(|error| {
                    format!("Could not set the sandboxed working directory: {error}")
                })?;
            }
            child.stdin = Some(
                tokio::process::ChildStdin::from_std(std::process::ChildStdin::from(
                    OwnedHandle::from(file),
                ))
                .map_err(|error| format!("Could not open the stdin pipe: {error}"))?,
            );
        }
        {
            let file = std::fs::File::from(stdout_pipe.parent);
            child.stdout = Some(
                tokio::process::ChildStdout::from_std(std::process::ChildStdout::from(
                    OwnedHandle::from(file),
                ))
                .map_err(|error| format!("Could not open the stdout pipe: {error}"))?,
            );
        }
        if let Some(pipe) = stderr_pipe {
            let file = std::fs::File::from(pipe.parent);
            child.stderr = Some(
                tokio::process::ChildStderr::from_std(std::process::ChildStderr::from(
                    OwnedHandle::from(file),
                ))
                .map_err(|error| format!("Could not open the stderr pipe: {error}"))?,
            );
        }
        // CreateProcessW duplicated the child-side stdio handles into the
        // child — close ours (they were passed as raw HANDLEs still owned here).
        unsafe {
            let _ = CloseHandle(handles[0]);
            let _ = CloseHandle(handles[1]);
            let _ = CloseHandle(handles[2]);
            let _ = CloseHandle(process.hThread);
        }
        Ok(Box::new(child))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inside_root_rejects_escapes_and_accepts_children() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let child = root.path().join("a").join("b");
        std::fs::create_dir_all(&child).unwrap();
        let file = child.join("f.txt");
        std::fs::write(&file, "x").unwrap();
        assert!(inside_root(root.path(), &file));
        assert!(inside_root(root.path(), &child));
        let secret = outside.path().join("secret.txt");
        std::fs::write(&secret, "x").unwrap();
        assert!(!inside_root(root.path(), &secret));
        // .. traversal resolves against the root, not lexically.
        assert!(!inside_root(
            root.path(),
            &root.path().join("..").join("x.txt")
        ));
        // A not-yet-created file inside the root is allowed.
        assert!(inside_root(
            root.path(),
            &root.path().join("newdir").join("new.txt")
        ));
    }

    #[test]
    fn network_policy_blocks_in_process_tools() {
        let mut settings = IsolationSettings {
            mode: IsolationMode::Enforced,
            ..Default::default()
        };
        assert!(agent_network_blocked(&settings));
        for alias in ["browser", "web_search", "web_fetch"] {
            assert!(network_blocked_alias(alias), "{alias} should be gated");
        }
        settings.network = true;
        assert!(!agent_network_blocked(&settings));
        // Unsandboxed never gates in-process tools.
        assert!(!agent_network_blocked(&IsolationSettings::default()));
    }
}

#[cfg(all(test, windows))]
mod adversarial {
    use super::*;
    use tokio::io::AsyncReadExt;

    fn enforced() -> IsolationSettings {
        IsolationSettings {
            mode: IsolationMode::Enforced,
            ..Default::default()
        }
    }

    fn powershell() -> String {
        r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe".into()
    }

    fn spec(cwd: &Path, program: &str, args: &[&str], writable: &[PathBuf]) -> SpawnSpec {
        SpawnSpec {
            program: program.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            cwd: cwd.to_path_buf(),
            env: SpawnSpec::standard_env(&[]),
            stdin: false,
            stderr: true,
            writable: writable.to_vec(),
            cpu_seconds: 30,
        }
    }

    async fn collect(
        child: &mut Box<dyn ChildWrapper>,
    ) -> (std::process::ExitStatus, String, String) {
        let mut stdout = child.stdout().take();
        let mut stderr = child.stderr().take();
        let out = tokio::spawn(async move {
            let mut buf = Vec::new();
            if let Some(stream) = stdout.as_mut() {
                let _ = stream.read_to_end(&mut buf).await;
            }
            buf
        });
        let err = tokio::spawn(async move {
            let mut buf = Vec::new();
            if let Some(stream) = stderr.as_mut() {
                let _ = stream.read_to_end(&mut buf).await;
            }
            buf
        });
        let status =
            match tokio::time::timeout(std::time::Duration::from_secs(30), child.wait()).await {
                Ok(status) => status.expect("child wait failed"),
                Err(_) => {
                    let _ = child.start_kill();
                    child.wait().await.expect("kill+wait failed")
                }
            };
        (
            status,
            String::from_utf8_lossy(&out.await.unwrap()).into_owned(),
            String::from_utf8_lossy(&err.await.unwrap()).into_owned(),
        )
    }

    /// Deletes the AppContainer profile and revokes the ancestor traverse
    /// ACEs when the test ends so runs do not leak registry mappings, package
    /// folders, or permission entries on user directories.
    struct ProfileGuard {
        name: String,
        root: PathBuf,
    }

    impl ProfileGuard {
        fn for_root(root: &Path) -> Self {
            Self {
                name: win::profile_name(root),
                root: root.to_path_buf(),
            }
        }
    }

    impl Drop for ProfileGuard {
        fn drop(&mut self) {
            if let Ok(sid) = win::profile_sid(&self.name) {
                let access = windows::Win32::Storage::FileSystem::FILE_GENERIC_EXECUTE.0;
                for ancestor in self.root.ancestors().skip(1) {
                    let _ = win::update_grant_no_propagate(ancestor, sid, access, true);
                }
            }
            let _ = win::delete_profile(&self.name);
        }
    }

    /// Powershell inside the container; the first call doubles as the probe —
    /// if the container can't even be created the whole suite reports why.
    async fn sandboxed(
        cwd: &Path,
        args: &[&str],
        writable: &[PathBuf],
        settings: &IsolationSettings,
    ) -> Result<(std::process::ExitStatus, String, String), String> {
        let _profile = writable.first().map(|root| ProfileGuard::for_root(root));
        let spec = spec(cwd, &powershell(), args, writable);
        let mut child = win::spawn_appcontainer(&spec, settings, None)?;
        Ok(collect(&mut child).await)
    }

    async fn ac_available() -> bool {
        let temp = tempfile::tempdir().unwrap();
        let _profile = ProfileGuard::for_root(temp.path());
        let spec = spec(
            temp.path(),
            &powershell(),
            &["-NoProfile", "-NonInteractive", "-Command", "1"],
            &[temp.path().to_path_buf()],
        );
        match win::spawn_appcontainer(&spec, &enforced(), None) {
            Ok(mut child) => {
                let wait =
                    tokio::time::timeout(std::time::Duration::from_secs(15), child.wait()).await;
                if wait.is_err() {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                }
                true
            }
            Err(error) => {
                eprintln!("[adversarial] AppContainer unavailable: {error}");
                false
            }
        }
    }

    #[tokio::test]
    #[ignore = "real AppContainer spawn - run with cargo test -- --ignored"]
    async fn sandboxed_read_outside_root_fails() {
        if !ac_available().await {
            panic!("AppContainers unavailable on this machine");
        }
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.txt");
        std::fs::write(&secret, "top-secret-777").unwrap();
        let arg = format!("Get-Content {}", secret.display());
        let (status, stdout, stderr) = sandboxed(
            root.path(),
            &["-NoProfile", "-NonInteractive", "-Command", &arg],
            &[root.path().to_path_buf()],
            &enforced(),
        )
        .await
        .unwrap();
        assert!(!stdout.contains("top-secret-777"), "read leaked: {stdout}");
        assert!(
            !status.success() || stderr.contains("denied") || stderr.contains("Unauthorized"),
            "exit={status:?} err={stderr}"
        );
    }

    /// Positive control: without it every "outside fails" test would also pass
    /// for a container that cannot run anything at all.
    #[tokio::test]
    #[ignore = "real AppContainer spawn - run with cargo test -- --ignored"]
    async fn sandboxed_read_and_write_inside_root_succeed() {
        if !ac_available().await {
            panic!("AppContainers unavailable on this machine");
        }
        let root = tempfile::tempdir().unwrap();
        let _profile = ProfileGuard::for_root(root.path());
        std::fs::write(root.path().join("inside.txt"), "inside-ok-42").unwrap();
        let (status, stdout, stderr) = sandboxed(
            root.path(),
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-Content inside.txt; Set-Content -Path written.txt -Value wrote-ok",
            ],
            &[root.path().to_path_buf()],
            &enforced(),
        )
        .await
        .unwrap();
        assert!(status.success(), "exit={status:?} err={stderr}");
        assert!(
            stdout.contains("inside-ok-42"),
            "inside read failed: {stdout} {stderr}"
        );
        let written = std::fs::read_to_string(root.path().join("written.txt")).unwrap();
        assert!(written.contains("wrote-ok"));
        let spec_stdin = spec(
            root.path(),
            &powershell(),
            &["-NoProfile", "-NonInteractive", "-Command", "-"],
            &[root.path().to_path_buf()],
        );
        let spec_stdin = SpawnSpec {
            stdin: true,
            ..spec_stdin
        };
        let mut child = win::spawn_appcontainer(&spec_stdin, &enforced(), None).unwrap();
        {
            use tokio::io::AsyncWriteExt;
            let mut stdin = child.stdin().take().unwrap();
            stdin.write_all(b"Get-Content inside.txt\n").await.unwrap();
        }
        let (_, stdout, stderr) = collect(&mut child).await;
        assert!(
            stdout.contains("inside-ok-42"),
            "stdin shell: {stdout} {stderr}"
        );
        let spec = spec(
            root.path(),
            r"C:\Windows\System32\cmd.exe",
            &["/c", "cd & type inside.txt"],
            &[root.path().to_path_buf()],
        );
        let mut child = win::spawn_appcontainer(&spec, &enforced(), None).unwrap();
        let (status, stdout, stderr) = collect(&mut child).await;
        assert!(
            status.success() && stdout.contains("inside-ok-42"),
            "cmd: {stdout} {stderr}"
        );
    }

    #[tokio::test]
    #[ignore = "real AppContainer spawn - run with cargo test -- --ignored"]
    async fn sandboxed_write_outside_root_fails() {
        if !ac_available().await {
            panic!("AppContainers unavailable on this machine");
        }
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("planted.txt");
        let arg = format!(
            "Set-Content -Path {} -Value x -ErrorAction Stop",
            target.display()
        );
        let (status, _, stderr) = sandboxed(
            root.path(),
            &["-NoProfile", "-NonInteractive", "-Command", &arg],
            &[root.path().to_path_buf()],
            &enforced(),
        )
        .await
        .unwrap();
        assert!(
            !target.exists(),
            "write escaped the container (exit={status:?} err={stderr})"
        );
    }

    #[tokio::test]
    #[ignore = "real AppContainer spawn - run with cargo test -- --ignored"]
    async fn junction_inside_root_cannot_escape() {
        if !ac_available().await {
            panic!("AppContainers unavailable on this machine");
        }
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "junction-secret").unwrap();
        let link = root.path().join("escape");
        let status = std::process::Command::new("cmd.exe")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(outside.path())
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "junction fixture failed: {}",
            String::from_utf8_lossy(&status.stderr)
        );
        // The in-process file tool side is already proven by the workspace
        // junction test; the process side hits the missing ACE instead.
        let arg = format!("Get-Content {}\\secret.txt", link.display());
        let (status, stdout, stderr) = sandboxed(
            root.path(),
            &["-NoProfile", "-NonInteractive", "-Command", &arg],
            &[root.path().to_path_buf()],
            &enforced(),
        )
        .await
        .unwrap();
        assert!(
            !stdout.contains("junction-secret"),
            "junction escape: exit={status:?} err={stderr}"
        );
    }

    #[tokio::test]
    #[ignore = "real AppContainer spawn - run with cargo test -- --ignored"]
    async fn child_process_stays_confined() {
        if !ac_available().await {
            panic!("AppContainers unavailable on this machine");
        }
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.txt");
        std::fs::write(&secret, "grandchild-secret").unwrap();
        // The sandboxed powershell spawns another powershell — the child
        // inherits the AppContainer token and stays confined.
        let arg = format!(
            "powershell -NoProfile -NonInteractive -Command \"Get-Content {}\"",
            secret.display()
        );
        let (_, stdout, _) = sandboxed(
            root.path(),
            &["-NoProfile", "-NonInteractive", "-Command", &arg],
            &[root.path().to_path_buf()],
            &enforced(),
        )
        .await
        .unwrap();
        assert!(
            !stdout.contains("grandchild-secret"),
            "grandchild escaped: {stdout}"
        );
    }

    #[tokio::test]
    #[ignore = "real AppContainer spawn - run with cargo test -- --ignored"]
    async fn network_blocked_fails_local_and_remote_tcp() {
        if !ac_available().await {
            panic!("AppContainers unavailable on this machine");
        }
        let root = tempfile::tempdir().unwrap();
        // Loopback needs privateNetworkClientServer, which blocked mode lacks.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let arg = format!(
            "try {{ (New-Object System.Net.Sockets.TcpClient).Connect(\"127.0.0.1\", {port}); Write-Output \"LOOPBACK-OPEN\" }} catch {{ Write-Output \"BLOCKED\" }}"
        );
        let (_, stdout, stderr) = sandboxed(
            root.path(),
            &["-NoProfile", "-NonInteractive", "-Command", &arg],
            &[root.path().to_path_buf()],
            &enforced(),
        )
        .await
        .unwrap();
        assert!(
            stdout.contains("BLOCKED"),
            "loopback connect succeeded under no-network: {stdout} err={stderr}"
        );
        drop(listener);
        let arg =
            "try { (New-Object System.Net.Sockets.TcpClient).Connect(\"1.1.1.1\", 443); Write-Output \"REMOTE-OPEN\" } catch { Write-Output \"BLOCKED\" }";
        let (_, stdout, stderr) = sandboxed(
            root.path(),
            &["-NoProfile", "-NonInteractive", "-Command", arg],
            &[root.path().to_path_buf()],
            &enforced(),
        )
        .await
        .unwrap();
        assert!(
            stdout.contains("BLOCKED"),
            "remote connect succeeded under no-network: {stdout} err={stderr}"
        );
    }

    /// privateNetworkClientServer opens the LAN, but loopback stays blocked —
    /// reaching 127.0.0.1 (or this machine's own address, which the kernel
    /// delivers locally) from an AppContainer needs an admin LoopbackExempt
    /// rule the app cannot grant itself.
    #[tokio::test]
    #[ignore = "real AppContainer spawn - run with cargo test -- --ignored"]
    async fn network_lan_still_blocks_loopback() {
        if !ac_available().await {
            panic!("AppContainers unavailable on this machine");
        }
        let root = tempfile::tempdir().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut settings = enforced();
        settings.network = true;
        settings.lan = true;
        let arg = format!(
            "try {{ (New-Object System.Net.Sockets.TcpClient).Connect(\"127.0.0.1\", {port}); Write-Output \"CONNECTED\" }} catch {{ Write-Output \"BLOCKED\" }}"
        );
        let (_, stdout, stderr) = sandboxed(
            root.path(),
            &["-NoProfile", "-NonInteractive", "-Command", &arg],
            &[root.path().to_path_buf()],
            &settings,
        )
        .await
        .unwrap();
        assert!(
            stdout.contains("BLOCKED"),
            "loopback should stay blocked without LoopbackExempt: {stdout} err={stderr}"
        );
    }

    #[tokio::test]
    #[ignore = "real AppContainer spawn - run with cargo test -- --ignored"]
    async fn unlisted_inherited_handle_is_unusable() {
        if !ac_available().await {
            panic!("AppContainers unavailable on this machine");
        }
        let root = tempfile::tempdir().unwrap();
        let inside = root.path().join("victim.txt");
        std::fs::write(&inside, "victim").unwrap();
        // An inheritable handle to a file inside the workspace. The HANDLE_LIST
        // only carries stdio, so this handle is not duplicated into the child.
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&inside)
            .unwrap();
        let raw = std::os::windows::io::AsRawHandle::as_raw_handle(&file) as usize;
        let arg = format!(
            "try {{ $fs = New-Object System.IO.FileStream ([IntPtr]{raw}, [System.IO.FileAccess]::Write, $false); $fs.WriteByte(65); $fs.Flush(); Write-Output \"WROTE\" }} catch {{ Write-Output \"BLOCKED\" }}"
        );
        let (_, stdout, _) = sandboxed(
            root.path(),
            &["-NoProfile", "-NonInteractive", "-Command", &arg],
            &[root.path().to_path_buf()],
            &enforced(),
        )
        .await
        .unwrap();
        assert!(
            !stdout.contains("WROTE"),
            "inherited handle reached the child: {stdout}"
        );
    }

    #[tokio::test]
    #[ignore = "real AppContainer spawn - run with cargo test -- --ignored"]
    async fn cwd_outside_root_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let spec = spec(
            outside.path(),
            &powershell(),
            &["-Command", "1"],
            &[root.path().to_path_buf()],
        );
        match win::spawn_appcontainer(&spec, &enforced(), None) {
            Err(error) => assert!(error.contains("outside"), "unexpected error: {error}"),
            Ok(_) => panic!("cwd outside every sandbox root was accepted"),
        }
    }

    #[tokio::test]
    #[ignore = "real AppContainer spawn - run with cargo test -- --ignored"]
    async fn enforced_failure_refuses_instead_of_running_unsandboxed() {
        let root = tempfile::tempdir().unwrap();
        let _profile = ProfileGuard::for_root(root.path());
        // A missing program makes CreateProcessW fail; the error must refuse.
        let spec = spec(
            root.path(),
            r"C:\Windows\System32\does-not-exist-xyz.exe",
            &["-Command", "1"],
            &[root.path().to_path_buf()],
        );
        match win::spawn_appcontainer(&spec, &enforced(), None) {
            Err(error) => assert!(error.contains("refuses"), "silent fallback? error: {error}"),
            Ok(_) => panic!("a failing sandbox setup ran anyway"),
        }
    }

    #[tokio::test]
    async fn unsandboxed_spawns_and_reads_anywhere() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.txt");
        std::fs::write(&secret, "plain-read").unwrap();
        let arg = format!("Get-Content {}", secret.display());
        let spec = spec(
            root.path(),
            &powershell(),
            &["-NoProfile", "-NonInteractive", "-Command", &arg],
            &[root.path().to_path_buf()],
        );
        let mut child = spawn(&spec, &IsolationSettings::default(), None).unwrap();
        let (status, stdout, _) = collect(&mut child).await;
        assert!(status.success());
        assert!(
            stdout.contains("plain-read"),
            "unsandboxed read regressed: {stdout}"
        );
    }

    /// Cleanup helper: removes sandbox profiles left behind by older runs.
    /// Not a regression check — run it by hand when needed.
    #[test]
    #[ignore = "maintenance - run with cargo test -- --ignored"]
    fn purge_stale_profiles_removes_leftovers() {
        let removed = win::purge_stale_profiles();
        eprintln!("removed {removed} stale sandbox profiles");
    }

    #[tokio::test]
    #[ignore = "real AppContainer spawn via run_code - run with cargo test -- --ignored"]
    async fn harness_run_code_is_confined() {
        if !ac_available().await {
            panic!("AppContainers unavailable on this machine");
        }
        let root = tempfile::tempdir().unwrap();
        let _profile = ProfileGuard::for_root(root.path());
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.txt");
        std::fs::write(&secret, "run-code-secret").unwrap();
        let config = crate::execution::ExecutionConfig {
            powershell_path: powershell(),
            ..Default::default()
        };
        let code = format!("Get-Content {}", secret.display());
        let result = crate::harness::execute_workspace_tool(
            root.path().to_str().unwrap(),
            config,
            "run_code",
            serde_json::json!({"language": "powershell", "code": code, "timeout_seconds": 30}),
            &enforced(),
            None,
        )
        .await
        .unwrap();
        let dumped = serde_json::to_string(&result).unwrap();
        assert!(
            !dumped.contains("run-code-secret"),
            "run_code escaped the container: {dumped}"
        );
    }
}
