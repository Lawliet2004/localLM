use crate::{runtime_config::RuntimeConfig, store::Preferences};
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use serde::Serialize;
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::Command;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub phase: String,
    pub message: String,
    pub model_path: Option<String>,
    pub loaded_config: Option<RuntimeConfig>,
    pub gpu_offload: Option<GpuOffload>,
    pub projector: Option<ProjectorStatus>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectorStatus {
    pub path: String,
    /// GPU offload is allowed; a CPU-only runtime may still fall back to CPU.
    pub offload: bool,
    pub file_bytes: u64,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GpuOffload {
    pub layers: u32,
    pub total_layers: u32,
}
fn parse_offload(log: &str) -> Option<GpuOffload> {
    log.lines()
        .filter_map(|line| {
            let (_, rest) = line.split_once("load_tensors: offloaded ")?;
            let counts = rest.strip_suffix(" layers to GPU")?;
            let (layers, total) = counts.split_once('/')?;
            let layers = layers.parse::<u32>().ok()?;
            let total_layers = total.parse::<u32>().ok()?;
            (total_layers > 0 && total_layers <= 100_000 && layers <= total_layers).then_some(
                GpuOffload {
                    layers,
                    total_layers,
                },
            )
        })
        .next_back()
}
fn read_offload(path: &Path) -> Option<GpuOffload> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(2 * 1024 * 1024)
        .read_to_end(&mut bytes)
        .ok()?;
    parse_offload(&String::from_utf8_lossy(&bytes))
}
impl Default for RuntimeStatus {
    fn default() -> Self {
        Self {
            phase: "stopped".into(),
            message: "No model loaded".into(),
            model_path: None,
            loaded_config: None,
            gpu_offload: None,
            projector: None,
        }
    }
}

pub struct Runtime {
    child: Option<Box<dyn ChildWrapper>>,
    pub status: RuntimeStatus,
    pub endpoint: String,
    pub api_key: String,
    pub context_length: u32,
    /// A loaded vision projector makes the local llama.cpp endpoint vision-capable.
    pub supports_images: bool,
    log_path: PathBuf,
    log_tasks: Vec<tokio::task::JoinHandle<std::io::Result<()>>>,
}
impl Runtime {
    pub fn new(log_path: PathBuf) -> Self {
        Self {
            child: None,
            status: RuntimeStatus::default(),
            endpoint: String::new(),
            api_key: String::new(),
            context_length: 0,
            supports_images: false,
            log_path,
            log_tasks: Vec::new(),
        }
    }
    pub fn inspect(&mut self) -> RuntimeStatus {
        if let Some(child) = &mut self.child {
            match child.try_wait() {
                Ok(Some(code)) => {
                    self.child = None;
                    self.supports_images = false;
                    self.status = RuntimeStatus {
                        phase: "error".into(),
                        message: format!(
                            "Model process exited ({code}). See {}",
                            self.log_path.display()
                        ),
                        model_path: None,
                        loaded_config: None,
                        gpu_offload: None,
                        projector: None,
                    };
                }
                Err(error) => {
                    self.status.message = format!("Could not inspect model process: {error}")
                }
                Ok(None) => {}
            }
        }
        self.status.clone()
    }
    pub async fn stop(&mut self) -> Result<RuntimeStatus, String> {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
        }
        for mut task in self.log_tasks.drain(..) {
            if tokio::time::timeout(Duration::from_secs(2), &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        }
        self.status = RuntimeStatus::default();
        self.api_key.clear();
        self.endpoint.clear();
        self.supports_images = false;
        Ok(self.status.clone())
    }
    #[cfg(test)]
    pub async fn load(
        &mut self,
        preferences: &Preferences,
        config: &RuntimeConfig,
    ) -> Result<RuntimeStatus, String> {
        self.load_observed(preferences, config, None).await
    }
    pub async fn load_observed(
        &mut self,
        preferences: &Preferences,
        config: &RuntimeConfig,
        observer: Option<std::sync::Arc<std::sync::Mutex<Option<RuntimeStatus>>>>,
    ) -> Result<RuntimeStatus, String> {
        config.validate()?;
        preferences.validate()?;
        let executable = std::fs::canonicalize(&preferences.runtime_path)
            .map_err(|_| "Select an existing llama-server executable in Models.")?;
        let model = std::fs::canonicalize(&preferences.model_path)
            .map_err(|_| "Select an existing GGUF model in Models.")?;
        validate_model(&model)?;
        validate_model_context(&model, config)?;
        // A vision projector is optional: text-only models load exactly as
        // before, while multimodal GGUFs load vision support when a valid
        // projector file is configured.
        let projector = configured_projector(preferences, config)?;
        let flags = inspect_runtime(&executable).await?;
        // Validate requested projector controls before stopping a working model.
        let projector_args = projector_arguments(config, projector.as_deref(), &flags)?;
        let projector_bytes = projector
            .as_ref()
            .and_then(|path| std::fs::metadata(path).ok())
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        let launch =
            resolve_launch_config(config, &model, &executable, flags.fit, projector_bytes).await?;
        let model_bytes = crate::gguf::model_metadata(&model)?.file_bytes;
        let attempts = startup_attempts(model_bytes);
        self.stop().await?;
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .map_err(|error| error.to_string())?
            .local_addr()
            .map_err(|error| error.to_string())?
            .port();
        self.api_key = uuid::Uuid::new_v4().to_string();
        self.endpoint = format!("http://127.0.0.1:{port}");
        let log = crate::runtime_log::RuntimeLog::create(&self.log_path)
            .await
            .map_err(|error| format!("Cannot create runtime log: {error}"))?;
        let mut command = runtime_command(&executable);
        command.args(launch.arguments()?).arg("--model").arg(&model);
        command.args(projector_args);
        command.args([
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--jinja",
            "--no-webui",
            "--log-verbosity",
            "4",
        ]);
        if flags.fit {
            command
                .arg("--fit")
                .arg(if config.automatic_gpu() { "on" } else { "off" });
        }
        command
            .env("LLAMA_API_KEY", &self.api_key)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Older llama-server builds predate --no-agent and default to no
        // tools; runtime_command removes LLAMA_* overrides either way.
        if flags.no_agent {
            command.arg("--no-agent");
        }
        let mut command = CommandWrap::from(command);
        command.wrap(KillOnDrop);
        #[cfg(windows)]
        {
            command.wrap(process_wrap::tokio::CreationFlags(
                windows::Win32::System::Threading::CREATE_NO_WINDOW,
            ));
            command.wrap(process_wrap::tokio::JobObject);
        }
        #[cfg(unix)]
        command.wrap(process_wrap::tokio::ProcessGroup::leader());
        let mut child = command
            .spawn()
            .map_err(|error| format!("Cannot start llama-server: {error}"))?;
        let stdout = child
            .stdout()
            .take()
            .ok_or("Runtime stdout pipe unavailable")?;
        let stderr = child
            .stderr()
            .take()
            .ok_or("Runtime stderr pipe unavailable")?;
        self.log_tasks
            .push(tokio::spawn(crate::runtime_log::drain(stdout, log.clone())));
        self.log_tasks
            .push(tokio::spawn(crate::runtime_log::drain(stderr, log)));
        self.child = Some(child);
        self.status = RuntimeStatus {
            phase: "loading".into(),
            message: "Loading model into memory".into(),
            // Keep the user-facing spelling from settings. `canonicalize` on
            // Windows adds a `\\?\\` device prefix, which made the UI report
            // a false model mismatch after every successful load.
            model_path: Some(preferences.model_path.clone()),
            loaded_config: None,
            gpu_offload: None,
            projector: None,
        };
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|error| error.to_string())?;
        let started = std::time::Instant::now();
        for _ in 0..attempts {
            self.status.message = format!(
                "Loading model · {} seconds elapsed · checking server readiness",
                started.elapsed().as_secs()
            );
            if let Some(observer) = &observer {
                if let Ok(mut status) = observer.lock() {
                    *status = Some(self.status.clone());
                }
            }

            if self.inspect().phase == "error" {
                return Err(self.status.message.clone());
            }
            // Authenticated endpoint prevents accepting another process that won the port race.
            if let Ok(response) = client
                .get(format!("{}/v1/models", self.endpoint))
                .bearer_auth(&self.api_key)
                .send()
                .await
            {
                if response.status().is_success() {
                    if let Ok(health) = client.get(format!("{}/health", self.endpoint)).send().await
                    {
                        if health.status().is_success() {
                            self.context_length = config.context_length;
                            self.supports_images = projector.is_some();
                            self.status.loaded_config = Some(config.clone());
                            self.status.gpu_offload = read_offload(&self.log_path);
                            self.status.projector = projector.as_ref().map(|_| ProjectorStatus {
                                path: preferences.projector_path.clone(),
                                offload: config.offload_projector,
                                file_bytes: projector_bytes,
                            });
                            self.status.phase = "ready".into();
                            self.status.message = if projector.is_some() {
                                if config.offload_projector {
                                    "Model ready · vision enabled (GPU preferred)".into()
                                } else {
                                    "Model ready · vision enabled (CPU)".into()
                                }
                            } else {
                                "Model ready".into()
                            };
                            return Ok(self.status.clone());
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        self.stop().await?;
        self.status.phase = "error".into();
        self.status.message = format!("Model startup timed out. See {}", self.log_path.display());
        Err(self.status.message.clone())
    }
}

fn runtime_command(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    #[cfg(target_os = "linux")]
    {
        let parent = rustix::process::getpid();
        // Dev restarts and crashes can bypass Drop and Tauri's Exit handler.
        // Release GPU memory even when the app is forcibly terminated.
        // SAFETY: this hook only makes async-signal-safe system calls between
        // fork and exec; it does not allocate or acquire locks.
        unsafe {
            command.pre_exec(move || {
                rustix::process::set_parent_process_death_signal(Some(
                    rustix::process::Signal::KILL,
                ))?;
                // The parent might have exited before the signal was armed.
                if rustix::process::getppid() != Some(parent) {
                    return Err(std::io::ErrorKind::BrokenPipe.into());
                }
                Ok(())
            });
        }
    }
    // Inherited llama options could expose tools or change the selected model.
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("LLAMA_") || key == "MTMD_BACKEND_DEVICE" {
            command.env_remove(key);
        }
    }
    command
}

struct RuntimeFlags {
    no_agent: bool,
    fit: bool,
    no_mmproj: bool,
    mmproj: bool,
    mmproj_offload: bool,
    no_mmproj_offload: bool,
}

impl RuntimeFlags {
    fn from_help(help: &str) -> Self {
        // Match whole flags: --no-mmproj-offload must not imply --no-mmproj.
        let has = |flag| {
            help.split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
                .any(|word| word == flag)
        };
        Self {
            no_agent: has("--no-agent"),
            fit: has("--fit"),
            no_mmproj: has("--no-mmproj"),
            mmproj: has("--mmproj"),
            mmproj_offload: has("--mmproj-offload"),
            no_mmproj_offload: has("--no-mmproj-offload"),
        }
    }
}

fn configured_projector(
    preferences: &Preferences,
    config: &RuntimeConfig,
) -> Result<Option<PathBuf>, String> {
    if !config.vision_enabled || preferences.projector_path.trim().is_empty() {
        return Ok(None);
    }
    let path = std::fs::canonicalize(preferences.projector_path.trim())
        .map_err(|_| "Select an existing mmproj projector file in Model files, or disable Load vision projector in Runtime for text-only chat.")?;
    validate_projector(&path)?;
    Ok(Some(path))
}

fn projector_arguments(
    config: &RuntimeConfig,
    projector: Option<&Path>,
    flags: &RuntimeFlags,
) -> Result<Vec<std::ffi::OsString>, String> {
    let mut args = Vec::new();
    if let Some(path) = projector {
        if !flags.mmproj {
            return Err("This llama-server does not support vision projectors. Select a newer compatible runtime in Model files.".into());
        }
        args.push("--mmproj".into());
        args.push(path.as_os_str().to_owned());
        if !config.offload_projector {
            if !flags.no_mmproj_offload {
                return Err("This llama-server cannot force the vision projector onto CPU. Update the runtime or choose GPU preferred in Runtime.".into());
            }
            args.push("--no-mmproj-offload".into());
        } else if flags.mmproj_offload {
            args.push("--mmproj-offload".into());
        }
    } else if flags.no_mmproj {
        // Explicitly prevent auto-discovery, even if an mmproj lives beside the model.
        args.push("--no-mmproj".into());
    } else if !config.vision_enabled && flags.mmproj {
        return Err("This llama-server cannot explicitly disable projector auto-loading. Update the runtime before using text-only projector mode.".into());
    }
    Ok(args)
}

async fn inspect_runtime(executable: &Path) -> Result<RuntimeFlags, String> {
    let mut command = runtime_command(executable);
    command.arg("--help").kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(windows::Win32::System::Threading::CREATE_NO_WINDOW.0);
    let output = tokio::time::timeout(Duration::from_secs(10), command.output())
        .await
        .map_err(|_| "Runtime compatibility check timed out.".to_string())?
        .map_err(|error| format!("Cannot inspect llama-server: {error}"))?;
    if !output.status.success() {
        return Err(format!("llama-server could not start ({}). Check that its matching CUDA DLLs are installed alongside the executable.", output.status));
    }
    let help = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(RuntimeFlags::from_help(&help))
}

/// Integrity is independent of loadability. The server help proves flag
/// support only; architecture/quantization support remains unknown until load.
pub(crate) async fn preflight_download(runtime_path: &str, projector: bool) -> Result<(), String> {
    if runtime_path.is_empty() {
        return Ok(());
    } // Offline provisioning is allowed.
    let executable = std::fs::canonicalize(runtime_path).map_err(|_| "The selected runtime is missing. Select a native llama-server or clear its saved path before downloading for later use.")?;
    let flags = inspect_runtime(&executable).await?;
    if projector && !flags.mmproj {
        return Err("The selected runtime does not support vision projectors. Update it or download text weights without a projector.".into());
    }
    Ok(())
}

async fn resolve_launch_config(
    config: &RuntimeConfig,
    model: &Path,
    executable: &Path,
    fit_supported: bool,
    projector_bytes: u64,
) -> Result<RuntimeConfig, String> {
    if !config.automatic_gpu() || fit_supported {
        return Ok(config.clone());
    }
    let hardware = crate::hardware::hardware_status().await;
    // Legacy numeric fitting is safe only when the selected server reports the
    // same sole CUDA device. Other backends and ambiguous device lists stay auto.
    let matches = match (
        hardware.single_gpu_name(),
        query_runtime_devices(executable).await,
    ) {
        (Some(name), Ok(devices)) => matches_single_cuda_device(&devices, name),
        _ => false,
    };
    let available = matches
        .then(|| hardware.free_vram_bytes())
        .flatten()
        .map(|bytes| {
            bytes.saturating_sub(if config.vision_enabled && config.offload_projector {
                projector_bytes
            } else {
                0
            })
        });
    // Missing NVIDIA telemetry cannot establish that a native backend is CPU-only.
    if available.is_none() {
        return Ok(config.clone());
    }
    let mut launch = config.clone();
    launch.gpu_layers = crate::gguf::model_metadata(model)
        .ok()
        .map(|metadata| RuntimeConfig::auto_gpu_layers(&metadata, &launch, available))
        .unwrap_or(-1);
    if launch.gpu_layers == 0 {
        launch.offload_kv_cache = false;
    }
    Ok(launch)
}

/// Health-poll budget after llama-server starts. 6 GiB-class models and a
/// cold CUDA context routinely exceed the original 60s window.
fn startup_attempts(model_bytes: u64) -> u32 {
    if model_bytes >= 4 * 1024 * 1024 * 1024 {
        1200
    } else {
        360
    }
}

fn validate_model_context(path: &Path, config: &RuntimeConfig) -> Result<(), String> {
    if let Ok(metadata) = crate::gguf::model_metadata(path) {
        if metadata
            .context_length
            .is_some_and(|max| config.context_length > max)
        {
            return Err(format!("This model supports at most {} context tokens. Adjust Context window in Runtime before loading.", metadata.context_length.unwrap()));
        }
        if metadata.block_count.is_some_and(|blocks| {
            config.gpu_layers > 0 && config.gpu_layers as u32 > blocks.saturating_add(1)
        }) {
            return Err("GPU layer selection exceeds the model's layer count. Choose Automatic or reduce the manual count in Runtime.".into());
        }
    }
    Ok(())
}

fn validate_model(path: &Path) -> Result<(), String> {
    let mut file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)
        .map_err(|_| "The model file is incomplete.")?;
    if magic != *b"GGUF" {
        return Err("The selected file is not a GGUF model.".into());
    }
    crate::gguf::model_metadata(path).map_err(|error| {
        format!("The GGUF metadata is incomplete, corrupt, or incompatible: {error}")
    })?;
    Ok(())
}

/// A vision projector must be a real GGUF file inside managed or user-owned
/// storage. Symlink, junction, and mount-point escapes fail before launch,
/// and a mistyped non-GGUF file fails with an actionable message instead of
/// a silent text-only startup or a crashed llama-server.
fn validate_projector(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() {
        return Err("The projector must be a regular file, not a shortcut or symlink.".into());
    }
    if !metadata.is_file() {
        return Err("The projector must be an mmproj GGUF file.".into());
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_lowercase();
    if !name.ends_with(".gguf") || !name.contains("mmproj") {
        return Err(
            "The projector must be an mmproj GGUF file (for example mmproj-model-f16.gguf).".into(),
        );
    }
    validate_model(path)
        .map_err(|_| "The projector file is incomplete or is not a GGUF model.".to_string())
}

#[cfg(test)]
mod tests {
    // Minimal metadata-only GGUF fixture. The mock server below tests launch
    // and projector flags, not tensor inference (live inference is separate).
    fn metadata_fixture() -> Vec<u8> {
        let mut bytes = b"GGUF".to_vec();
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn download_preflight_rejects_missing_runtime_and_known_missing_projector_support() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let executable = temp.path().join("llama-server");
        assert!(preflight_download(executable.to_str().unwrap(), false)
            .await
            .unwrap_err()
            .contains("missing"));
        preflight_download("", true).await.unwrap();
        std::fs::write(&executable, "#!/bin/sh\necho --fit\n").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        preflight_download(executable.to_str().unwrap(), false)
            .await
            .unwrap();
        assert!(preflight_download(executable.to_str().unwrap(), true)
            .await
            .unwrap_err()
            .contains("does not support vision"));
        std::fs::write(&executable, "#!/bin/sh\necho --mmproj\n").unwrap();
        preflight_download(executable.to_str().unwrap(), true)
            .await
            .unwrap();
    }

    #[test]
    fn legacy_fit_requires_the_selected_runtimes_single_matching_cuda_device() {
        assert!(matches_single_cuda_device(
            "Available devices:\n CUDA0: NVIDIA Example (8192 MiB)",
            "NVIDIA Example"
        ));
        for devices in [
            "Vulkan0: NVIDIA Example",
            "CUDA0: Other GPU",
            "CUDA0: NVIDIA Example\nCUDA1: Second GPU",
            "",
            "Available devices:",
        ] {
            assert!(!matches_single_cuda_device(devices, "NVIDIA Example"));
        }
    }

    #[test]
    fn model_validation_rejects_truncated_metadata_and_unknown_gguf_versions() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("model.gguf");
        std::fs::write(&path, b"GGUF").unwrap();
        assert!(super::validate_model(&path)
            .unwrap_err()
            .contains("metadata"));
        let mut bytes = metadata_fixture();
        bytes[4..8].copy_from_slice(&99u32.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        assert!(super::validate_model(&path).is_err());
    }

    use super::*;

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn parent_death_fixture() {
        let Some(pid_file) = std::env::var_os("LOCALLM_TEST_PARENT_DEATH_PID") else {
            return;
        };
        let mut child = runtime_command(Path::new("/bin/sleep"))
            .arg("60")
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        std::fs::write(pid_file, child.id().unwrap().to_string()).unwrap();
        let _ = child.wait().await;
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn runtime_exits_when_parent_is_forcibly_killed() {
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("runtime.pid");
        let mut parent = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "runtime::tests::parent_death_fixture",
                "--nocapture",
            ])
            .env("LOCALLM_TEST_PARENT_DEATH_PID", &pid_file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let pid = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(pid) = std::fs::read_to_string(&pid_file) {
                    if let Ok(pid) = pid.parse::<u32>() {
                        break pid;
                    }
                }
                assert!(
                    parent.try_wait().unwrap().is_none(),
                    "Parent fixture exited before starting the runtime"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        parent.kill().await.unwrap();
        parent.wait().await.unwrap();
        let exited = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                // A reparented zombie has exited and released its GPU memory.
                let running = std::fs::read_to_string(format!("/proc/{pid}/stat"))
                    .ok()
                    .and_then(|stat| {
                        stat.rsplit_once(") ")
                            .map(|(_, fields)| !fields.starts_with('Z'))
                    })
                    .unwrap_or(false);
                if !running {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        if exited.is_err() {
            // Keep a failing regression from leaving its own orphan behind.
            if let Some(pid) = rustix::process::Pid::from_raw(pid as i32) {
                let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
            }
        }
        assert!(
            exited.is_ok(),
            "Runtime survived its parent and would retain GPU memory"
        );
    }

    #[test]
    fn projector_controls_emit_independent_launch_flags_and_reject_unsupported_modes() {
        let flags = RuntimeFlags::from_help(
            "--mmproj FILE --no-mmproj --mmproj-offload --no-mmproj-offload --fit --no-agent",
        );
        let config = RuntimeConfig::default();
        let path = Path::new("mmproj-model-F16.gguf");
        let gpu = projector_arguments(&config, Some(path), &flags).unwrap();
        assert_eq!(
            gpu,
            ["--mmproj", "mmproj-model-F16.gguf", "--mmproj-offload"].map(std::ffi::OsString::from)
        );
        let cpu = RuntimeConfig {
            offload_projector: false,
            ..config.clone()
        };
        assert!(projector_arguments(&cpu, Some(path), &flags)
            .unwrap()
            .contains(&"--no-mmproj-offload".into()));
        let off = RuntimeConfig {
            vision_enabled: false,
            ..config.clone()
        };
        assert_eq!(
            projector_arguments(&off, None, &flags).unwrap(),
            vec![std::ffi::OsString::from("--no-mmproj")]
        );
        let old = RuntimeFlags::from_help("--mmproj FILE --mmproj-offload");
        assert!(projector_arguments(&cpu, Some(path), &old)
            .unwrap_err()
            .contains("CPU"));
        assert!(projector_arguments(&off, None, &old)
            .unwrap_err()
            .contains("auto-loading"));
        assert!(!RuntimeFlags::from_help("--no-mmproj-offload").no_mmproj);
        assert!(!RuntimeFlags::from_help("--fit-target").fit);
        assert!(!RuntimeFlags::from_help("--mmproj-offload").mmproj);
    }

    #[test]
    fn disabled_vision_does_not_require_or_clear_the_saved_projector_file() {
        let preferences = Preferences {
            projector_path: "missing/mmproj-model-F16.gguf".into(),
            ..Default::default()
        };
        let disabled = RuntimeConfig {
            vision_enabled: false,
            ..Default::default()
        };
        assert!(configured_projector(&preferences, &disabled)
            .unwrap()
            .is_none());
        assert!(!preferences.projector_path.is_empty());
        assert!(configured_projector(&preferences, &RuntimeConfig::default()).is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reloads_projector_modes_and_updates_image_capability() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("llama-server");
        // Exercise the real subprocess/health/reload path without model weights
        // or a GPU. This fixture implements only startup and authenticated health.
        std::fs::write(&executable, r#"#!/usr/bin/env python3
import http.server,json,os,pathlib,sys
if '--help' in sys.argv:
    print('--mmproj FILE --no-mmproj --mmproj-offload --no-mmproj-offload --fit --no-agent')
    sys.exit(0)
pathlib.Path(__file__).with_suffix('.args.json').write_text(json.dumps(sys.argv[1:]))
port=int(sys.argv[sys.argv.index('--port')+1])
class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path != '/health' and self.headers.get('Authorization') != 'Bearer '+os.environ['LLAMA_API_KEY']:
            self.send_error(401)
            return
        self.send_response(200)
        self.send_header('Content-Type','application/json')
        self.end_headers()
        self.wfile.write(b'{"data":[]}')
    def log_message(self,*args): pass
http.server.HTTPServer(('127.0.0.1',port),Handler).serve_forever()
"#).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        let model = directory.path().join("model.gguf");
        let projector = directory.path().join("mmproj-model-F16.gguf");
        std::fs::write(&model, metadata_fixture()).unwrap();
        std::fs::write(&projector, metadata_fixture()).unwrap();
        let mut preferences = Preferences {
            runtime_path: executable.to_string_lossy().into_owned(),
            model_path: model.to_string_lossy().into_owned(),
            projector_path: projector.to_string_lossy().into_owned(),
            ..Default::default()
        };
        let mut runtime = Runtime::new(directory.path().join("runtime.log"));
        for (vision_enabled, offload_projector) in
            [(false, true), (true, false), (true, true), (false, false)]
        {
            let config = RuntimeConfig {
                vision_enabled,
                offload_projector,
                ..Default::default()
            };
            let status = runtime.load(&preferences, &config).await.unwrap();
            assert_eq!(status.phase, "ready");
            assert_eq!(runtime.supports_images, vision_enabled);
            assert_eq!(status.projector.is_some(), vision_enabled);
            assert_eq!(status.loaded_config, Some(config));
            let args: Vec<String> = serde_json::from_slice(
                &std::fs::read(executable.with_extension("args.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(args.iter().any(|arg| arg == "--mmproj"), vision_enabled);
            assert_eq!(args.iter().any(|arg| arg == "--no-mmproj"), !vision_enabled);
            assert_eq!(
                args.iter().any(|arg| arg == "--no-mmproj-offload"),
                vision_enabled && !offload_projector
            );
            if let Some(loaded) = status.projector {
                assert_eq!(loaded.offload, offload_projector);
                assert_eq!(loaded.file_bytes, metadata_fixture().len() as u64);
            }
        }
        // Disabling a configured but subsequently missing projector must still
        // allow the base language model to start.
        preferences.projector_path = directory
            .path()
            .join("mmproj-missing.gguf")
            .to_string_lossy()
            .into_owned();
        assert!(runtime
            .load(
                &preferences,
                &RuntimeConfig {
                    vision_enabled: false,
                    ..Default::default()
                }
            )
            .await
            .is_ok());
        let before = runtime.inspect();
        let invalid = Preferences {
            model_path: directory
                .path()
                .join("missing.gguf")
                .to_string_lossy()
                .into_owned(),
            ..preferences.clone()
        };
        assert!(runtime
            .load(&invalid, &RuntimeConfig::default())
            .await
            .is_err());
        assert_eq!(runtime.inspect().phase, "ready");
        assert_eq!(runtime.inspect().model_path, before.model_path);
        assert_eq!(runtime.inspect().loaded_config, before.loaded_config);
        runtime.stop().await.unwrap();
        assert!(!runtime.supports_images);
        assert!(runtime.status.projector.is_none());
    }

    #[test]
    fn large_ggufs_get_a_longer_startup_window() {
        assert_eq!(startup_attempts(2 * 1024 * 1024 * 1024), 360);
        assert_eq!(startup_attempts(4 * 1024 * 1024 * 1024), 1200);
        assert_eq!(startup_attempts(6 * 1024 * 1024 * 1024), 1200);
    }

    #[tokio::test]
    #[ignore = "requires a cached upstream Windows CUDA runtime installation"]
    async fn recognizes_current_runtime_flags() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let current = inspect_runtime(&root.join(".local/runtime/llama-server.exe"))
            .await
            .unwrap();
        assert!(current.no_agent);
        assert!(current.fit);
    }

    #[test]
    fn offload_reporting_requires_valid_runtime_evidence() {
        assert_eq!(
            parse_offload("load_tensors: offloaded 25/25 layers to GPU\n"),
            Some(GpuOffload {
                layers: 25,
                total_layers: 25
            })
        );
        assert_eq!(
            parse_offload("0.01 I load_tensors: offloaded 0/25 layers to GPU\r\n"),
            Some(GpuOffload {
                layers: 0,
                total_layers: 25
            })
        );
        for text in [
            "offloaded 25/25 layers to GPU",
            "load_tensors: offloaded 26/25 layers to GPU",
            "load_tensors: offloaded -1/25 layers to GPU",
            "load_tensors: offloaded 0/0 layers to GPU",
            "load_tensors: offloaded 25/25 layers to GPU extra",
            "load_tensors: offloaded 9999999999999999/25 layers to GPU",
        ] {
            assert_eq!(parse_offload(text), None);
        }
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("log");
        let mut content = vec![b'x'; 2 * 1024 * 1024];
        content.extend_from_slice(b"\nload_tensors: offloaded 1/25 layers to GPU\n");
        std::fs::write(&file, content).unwrap();
        assert_eq!(read_offload(&file), None);
    }
    #[test]
    fn rejects_truncated_or_non_gguf_models() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fake.gguf");
        std::fs::write(&path, b"GG").unwrap();
        assert!(validate_model(&path).is_err());
        std::fs::write(&path, b"HTML error document").unwrap();
        assert!(validate_model(&path).is_err());
    }
    #[test]
    fn projector_must_be_a_regular_mmproj_gguf_file() {
        let directory = tempfile::tempdir().unwrap();
        let valid = directory.path().join("mmproj-model-f16.gguf");
        std::fs::write(&valid, metadata_fixture()).unwrap();
        assert!(validate_projector(&valid).is_ok());
        let wrong_name = directory.path().join("model.gguf");
        std::fs::write(&wrong_name, metadata_fixture()).unwrap();
        let error = validate_projector(&wrong_name).unwrap_err();
        assert!(error.contains("mmproj"), "{error}");
        let truncated = directory.path().join("mmproj-truncated.gguf");
        std::fs::write(&truncated, b"GG").unwrap();
        assert!(validate_projector(&truncated).is_err());
        assert!(validate_projector(&directory.path().join("mmproj-missing.gguf")).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let link = directory.path().join("mmproj-link.gguf");
            symlink(&valid, &link).unwrap();
            let error = validate_projector(&link).unwrap_err();
            assert!(error.contains("symlink"), "{error}");
        }
    }
}

#[tauri::command]
pub async fn runtime_devices(state: tauri::State<'_, crate::AppState>) -> Result<String, String> {
    let path = state.database()?.preferences()?.runtime_path;
    if path.is_empty() {
        return Err("Select and save a native runtime before inspecting its devices.".into());
    }
    let executable = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
    query_runtime_devices(&executable).await
}
fn matches_single_cuda_device(devices: &str, name: &str) -> bool {
    let lines: Vec<_> = devices
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && *line != "Available devices:")
        .collect();
    lines.len() == 1 && lines[0].starts_with("CUDA0:") && lines[0].contains(name)
}
async fn query_runtime_devices(executable: &Path) -> Result<String, String> {
    let mut command = runtime_command(executable);
    command
        .arg("--list-devices")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    use tokio::io::AsyncReadExt;
    let mut child = command
        .spawn()
        .map_err(|e| format!("Cannot inspect selected runtime devices: {e}"))?;
    let mut bytes = Vec::new();
    let mut output = child
        .stdout
        .take()
        .ok_or("Runtime device output unavailable")?
        .take(65537);
    let status = tokio::time::timeout(Duration::from_secs(10), async {
        output
            .read_to_end(&mut bytes)
            .await
            .map_err(|e| e.to_string())?;
        child.wait().await.map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "Selected runtime device query timed out")??;
    if !status.success() || bytes.is_empty() || bytes.len() > 65536 {
        return Err("The selected runtime cannot report its devices. Acceleration capability remains unknown; update llama-server or inspect its native logs.".into());
    }
    String::from_utf8(bytes).map_err(|_| "Runtime device output was not readable.".into())
}
