use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillFile {
    pub path: String,
    source_path: String,
    size: usize,
    sha256: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub id: String,
    description: String,
    repo: String,
    revision: String,
    source_path: String,
    files: Vec<SkillFile>,
}
/// A runtime requirement an installed skill needs before its scripted
/// workflows can run. Every dependency maps to a user-actionable remedy.
///
/// Skill authors declare only connectors and interpreters. `SkillDependency`
/// never executes installers or mutates configuration.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDependency {
    pub kind: String,
    pub name: String,
    pub detail: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDependencyStatus {
    pub dependency: SkillDependency,
    pub satisfied: bool,
    pub remedy: String,
}
#[derive(Deserialize)]
struct Catalog {
    skills: Vec<Skill>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillView {
    #[serde(flatten)]
    skill: Skill,
    installed: bool,
    active: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillUpdateStatus {
    pub id: String,
    pub installed_revision: Option<String>,
    pub catalog_revision: String,
    pub update_available: bool,
    pub intact: bool,
    pub problem: Option<String>,
}
#[derive(Clone)]
pub struct Skills {
    root: PathBuf,
}
fn catalog() -> Vec<Skill> {
    serde_json::from_str::<Catalog>(include_str!("../../catalog/skills.lock.json"))
        .expect("Reviewed skill lock is valid")
        .skills
}
fn skill(id: &str) -> Result<Skill, String> {
    catalog()
        .into_iter()
        .find(|item| item.id == id)
        .ok_or("Unknown skill.".into())
}
fn relative(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.contains(['\\', ':'])
        || path.split('/').any(|part| {
            part.is_empty()
                || part == "."
                || part == ".."
                || part.ends_with(['.', ' '])
                || part.chars().any(char::is_control)
        })
    {
        return Err("Invalid skill file path.".into());
    }
    if Path::new(path)
        .components()
        .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err("Skill path must be relative.".into());
    }
    Ok(())
}
fn verify(file: &SkillFile, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() != file.size || format!("{:x}", Sha256::digest(bytes)) != file.sha256 {
        return Err(format!(
            "Integrity check failed for {}. Reinstall the skill.",
            file.path
        ));
    }
    Ok(())
}
impl Skills {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
    fn directory(&self, skill: &Skill) -> PathBuf {
        self.root.join(format!("{}-{}", skill.id, skill.revision))
    }
    fn installed(&self, skill: &Skill) -> bool {
        self.directory(skill).join(".installed").is_file()
            && std::fs::read(self.directory(skill).join(".installed"))
                .is_ok_and(|bytes| bytes == skill.revision.as_bytes())
    }
    /// Statically declared runtime dependencies for every pinned skill.
    ///
    /// Sources: each skill's `SKILL.md` workflow at its pinned revision plus
    /// the packaged script inventory in `catalog/skills.lock.json`. Connector
    /// entries name the TrueForge preset in `src/lib/catalog.json`; interpreter
    /// entries name the Execution-page interpreter (`python`, `node`,
    /// `powershell`). `uv`, `gh`, `sentry`, and `tvly` are external CLIs the
    /// user installs separately; they are reported, never installed here.
    pub fn dependencies(id: &str) -> Result<Vec<SkillDependency>, String> {
        Ok(match skill(id)?.id.as_str() {
            "algorithmic-art" => vec![Self::interpreter(
                "node",
                "Render or preview the p5.js viewer artifact.",
            )],
            "skill-creator" => vec![
                Self::interpreter("python", "Run the eval/benchmark helper scripts."),
                Self::external_cli(
                    "uv",
                    "Run skill-creator helper scripts when its SKILL.md suggests uv.",
                ),
            ],
            "mcp-builder" => vec![
                Self::interpreter("python", "Run scripts/connections.py and scripts/evaluation.py."),
                Self::interpreter("node", "Build or inspect TypeScript MCP servers."),
                Self::external_cli(
                    "uv",
                    "Install scripts/requirements.txt (anthropic, mcp) for scripted checks.",
                ),
            ],
            "web-artifacts-builder" => vec![
                Self::interpreter("node", "Initialize and bundle React/Vite artifacts."),
                Self::external_cli("bash", "Run scripts/init-artifact.sh and bundle-artifact.sh."),
            ],
            "tavily-research" => vec![
                Self::connector("tavily", "Tavily MCP search/extract/crawl and reporting."),
                Self::external_cli(
                    "tvly",
                    "Run tvly research/search when the workflow uses the CLI instead of MCP tools.",
                ),
            ],
            "supabase" => vec![Self::connector(
                "supabase",
                "Supabase MCP execute_sql, advisors, docs, and project tools.",
            )],
            "wiki-architect" => vec![
                Self::connector("deepwiki", "DeepWiki repository wiki and structure tools."),
                Self::connector("github", "GitHub repository file access for private repos."),
            ],
            "wiki-qa" => vec![
                Self::connector("deepwiki", "DeepWiki repository wiki and structure tools."),
                Self::connector("github", "GitHub repository file access for private repos."),
            ],
            "linear" => vec![Self::connector(
                "linear",
                "Linear MCP issue/project/team management over OAuth.",
            )],
            "gh-fix-ci" => vec![
                Self::connector("github", "GitHub MCP pull-request and CI context."),
                Self::interpreter("python", "Run scripts/inspect_pr_checks.py for failing checks."),
                Self::external_cli(
                    "gh",
                    "Authenticate once with gh auth login, then inspect PR checks.",
                ),
            ],
            "notion-knowledge-capture" => vec![Self::connector(
                "notion",
                "Notion MCP search/fetch/create/update pages and databases.",
            )],
            "sentry" => vec![
                Self::connector("sentry", "Sentry MCP issue/event inspection over OAuth."),
                Self::external_cli(
                    "sentry",
                    "Run read-only sentry issue/event commands from its SKILL.md.",
                ),
            ],
            "jupyter-notebook" => vec![
                Self::interpreter("python", "Run scripts/new_notebook.py and notebook cells."),
                Self::external_cli(
                    "uv",
                    "Optional: uv pip install jupyterlab ipykernel for execution.",
                ),
            ],
            _ => return Err("Unknown skill.".into()),
        })
    }
    fn connector(name: &str, detail: &str) -> SkillDependency {
        SkillDependency { kind: "connector".into(), name: name.into(), detail: detail.into() }
    }
    fn interpreter(name: &str, detail: &str) -> SkillDependency {
        SkillDependency { kind: "interpreter".into(), name: name.into(), detail: detail.into() }
    }
    fn external_cli(name: &str, detail: &str) -> SkillDependency {
        SkillDependency { kind: "externalCli".into(), name: name.into(), detail: detail.into() }
    }
    /// Evaluate declared dependencies against live application state.
    ///
    /// Connector entries check the hub's connected sessions; interpreter
    /// entries check the saved Execution-page paths; external CLIs resolve on
    /// PATH. Nothing here launches installers, and skill instructions never
    /// bypass the conversation permission policy.
    pub fn dependency_status(
        &self,
        id: &str,
        connectors: &crate::connectors::McpHub,
        execution: &crate::execution::ExecutionConfig,
    ) -> Result<Vec<SkillDependencyStatus>, String> {
        if !self.installed(&skill(id)?) {
            return Err("Install this skill first.".into());
        }
        Self::dependencies(id)?
            .into_iter()
            .map(|dependency| {
                let (satisfied, remedy) = match dependency.kind.as_str() {
                    "connector" => {
                        let connected = connectors.is_connected(&dependency.name);
                        (
                            connected,
                            if connected {
                                format!("{} connector is connected.", dependency.name)
                            } else {
                                format!(
                                    "Connect {} in Connectors, then select its tools for this conversation.",
                                    dependency.name
                                )
                            },
                        )
                    }
                    "interpreter" => {
                        let path = match dependency.name.as_str() {
                            "python" => &execution.python_path,
                            "node" => &execution.node_path,
                            "powershell" => &execution.powershell_path,
                            _ => "",
                        };
                        let configured =
                            !path.is_empty() && Path::new(path).is_absolute() && Path::new(path).is_file();
                        (
                            configured,
                            if configured {
                                format!("{} interpreter is configured in Execution.", dependency.name)
                            } else {
                                format!(
                                    "Set the {} interpreter path in Execution; skill scripts stay idle until then.",
                                    dependency.name
                                )
                            },
                        )
                    }
                    _ => {
                        let available = std::env::var_os("PATH")
                            .into_iter()
                            .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
                            .any(|directory| {
                                let candidate = directory.join(format!("{}.exe", dependency.name));
                                let bare = directory.join(&dependency.name);
                                candidate.is_file() || bare.is_file()
                            });
                        (
                            available,
                            if available {
                                format!("{} is available on PATH.", dependency.name)
                            } else {
                                format!(
                                    "Install {} separately; LocalLM never installs external CLIs.",
                                    dependency.name
                                )
                            },
                        )
                    }
                };
                Ok(SkillDependencyStatus { dependency, satisfied, remedy })
            })
            .collect()
    }
    pub fn list(&self, active: &[String]) -> Vec<SkillView> {
        catalog()
            .into_iter()
            .map(|skill| SkillView {
                installed: self.installed(&skill),
                active: active.contains(&skill.id),
                skill,
            })
            .collect()
    }
    /// Versioned integrity status for one installed skill.
    ///
    /// Reports which revision is installed (when the marker matches a
    /// directory on disk), whether the catalog pins a different revision, and
    /// whether every pinned file still verifies. Recovery is explicit: a
    /// damaged package must be reinstalled through `install_skill`, which
    /// verifies before publishing and never executes package scripts.
    pub fn update_status(&self, id: &str) -> Result<SkillUpdateStatus, String> {
        let expected = skill(id)?;
        let mut installed_revision: Option<String> = None;
        let mut intact = false;
        let mut problem: Option<String> = None;
        if self.root.is_dir() {
            for entry in std::fs::read_dir(&self.root).map_err(|error| error.to_string())? {
                let entry = entry.map_err(|error| error.to_string())?;
                let name = entry.file_name().to_string_lossy().into_owned();
                let Some(revision) = name.strip_prefix(&format!("{}-", expected.id)) else {
                    continue;
                };
                if revision.len() != 40
                    || !revision.bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    continue;
                }
                let marker = entry.path().join(".installed");
                if std::fs::read(&marker).is_ok_and(|bytes| bytes == revision.as_bytes()) {
                    installed_revision = Some(revision.into());
                }
            }
        }
        if let Some(installed) = installed_revision.clone() {
            if installed == expected.revision {
                intact = self.installed(&expected);
                if intact {
                    for file in &expected.files {
                        match std::fs::read(self.directory(&expected).join(&file.path)) {
                            Ok(bytes) if verify(file, &bytes).is_ok() => {}
                            _ => {
                                intact = false;
                                problem = Some(format!(
                                    "Integrity check failed for {}. Reinstall the skill.",
                                    file.path
                                ));
                                break;
                            }
                        }
                    }
                } else {
                    problem = Some("The installed package is incomplete. Reinstall the skill.".into());
                }
            } else {
                problem = Some(format!(
                    "Installed revision {installed} differs from the pinned revision {}. Reinstall to update.",
                    expected.revision
                ));
            }
        }
        let update_available =
            installed_revision.as_deref() != Some(expected.revision.as_str());
        Ok(SkillUpdateStatus {
            id: expected.id.clone(),
            installed_revision,
            catalog_revision: expected.revision.clone(),
            update_available,
            intact,
            problem,
        })
    }
    pub fn read(&self, id: &str, path: &str) -> Result<String, String> {
        relative(path)?;
        let skill = skill(id)?;
        if !self.installed(&skill) {
            return Err("Install this skill first.".into());
        }
        let file = skill
            .files
            .iter()
            .find(|file| file.path == path)
            .ok_or("File is not in the verified skill package.")?;
        let root = cap_std::fs::Dir::open_ambient_dir(&self.root, cap_std::ambient_authority())
            .map_err(|error| error.to_string())?;
        let directory = root
            .open_dir(format!("{}-{}", skill.id, skill.revision))
            .map_err(|error| error.to_string())?;
        let source = directory.open(path).map_err(|error| error.to_string())?;
        if source.metadata().map_err(|error| error.to_string())?.len() != file.size as u64 {
            return Err("Skill file size changed. Reinstall the skill.".into());
        }
        let mut bytes = Vec::with_capacity(file.size);
        source
            .take(file.size as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        verify(file, &bytes)?;
        String::from_utf8(bytes)
            .map_err(|_| "This file is binary and cannot be displayed as text.".into())
    }
    pub fn instructions(&self, ids: &[String]) -> Result<String, String> {
        let mut context = String::new();
        for id in ids {
            context.push_str(&format!(
                "\n\nSelected skill: {id}\n{}",
                self.read(id, "SKILL.md")?
            ));
            context.push_str("\nPackage files (use skills_read_file with skill_id and a package-relative path to inspect; reading does not execute scripts):\n");
            for file in skill(id)?.files {
                context.push_str(&format!("{} ({} bytes)\n", file.path, file.size));
            }
        }
        if context.len() > 65_536 {
            return Err("Selected skill instructions exceed 64 KiB. Activate fewer skills for this conversation.".into());
        }
        Ok(context)
    }
    pub fn reader(
        &self,
        active: &[String],
    ) -> Result<Option<crate::connectors::AgentTool>, String> {
        if active.is_empty() {
            return Ok(None);
        }
        for id in active {
            self.read(id, "SKILL.md")?;
        }
        Ok(Some(crate::connectors::AgentTool::skills(std::sync::Arc::new(SkillReader {
            manager: self.clone(), active: active.to_vec(),
        }), crate::connectors::ToolView {
            name: "read_file".into(),
            description: "Read verified text from an active skill's package, including references and script source. Does not execute code. Paths are listed in skill instructions. Results contain numbered lines; request later lines when has_more is true.".into(),
            input_schema: serde_json::json!({"type":"object","properties":{
                "skill_id":{"type":"string","enum":active}, "path":{"type":"string"},
                "start_line":{"type":"integer","minimum":1,"default":1},
                "line_count":{"type":"integer","minimum":1,"maximum":500,"default":200}
            },"required":["skill_id","path"],"additionalProperties":false}),
        })))
    }
    pub async fn install(&self, id: &str) -> Result<(), String> {
        let skill = skill(id)?;
        // Repair path: a directory without a matching marker is incomplete,
        // so verify every file and reinstall when anything is missing or
        // fails its hash check. Verification never executes package scripts.
        let mut intact = self.installed(&skill);
        if intact {
            for file in &skill.files {
                match std::fs::read(self.directory(&skill).join(&file.path)) {
                    Ok(bytes) if verify(file, &bytes).is_ok() => {}
                    _ => {
                        intact = false;
                        break;
                    }
                }
            }
        }
        if intact {
            return Ok(());
        }
        std::fs::create_dir_all(&self.root).map_err(|error| error.to_string())?;
        let root = self
            .root
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let stage = root.join(format!("stage-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&stage).map_err(|error| error.to_string())?;
        let result = async {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(45))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|error| error.to_string())?;
            for file in &skill.files {
                relative(&file.path)?;
                relative(&file.source_path)?;
                let response = client
                    .get(format!(
                        "https://raw.githubusercontent.com/{}/{}/{}",
                        skill.repo, skill.revision, file.source_path
                    ))
                    .send()
                    .await
                    .map_err(|_| "Skill download failed. Check your connection.")?;
                if !response.status().is_success() {
                    return Err(format!("Skill download returned {}.", response.status()));
                }
                let mut stream = response.bytes_stream();
                let mut bytes = Vec::with_capacity(file.size);
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.map_err(|_| "Skill download was interrupted.")?;
                    if bytes.len() + chunk.len() > file.size {
                        return Err("Skill download exceeded its expected size.".into());
                    }
                    bytes.extend_from_slice(&chunk);
                }
                verify(file, &bytes)?;
                let target = stage.join(&file.path);
                std::fs::create_dir_all(target.parent().ok_or("Invalid skill directory.")?)
                    .map_err(|error| error.to_string())?;
                std::fs::write(target, bytes).map_err(|error| error.to_string())?;
            }
            std::fs::write(stage.join(".installed"), &skill.revision)
                .map_err(|error| error.to_string())?;
            // A leftover incomplete directory from an interrupted install must
            // not shadow the verified replacement.
            let destination = self.directory(&skill);
            if destination.exists() {
                std::fs::remove_dir_all(&destination).map_err(|error| error.to_string())?;
            }
            std::fs::rename(&stage, destination).map_err(|error| error.to_string())?;
            Ok(())
        }
        .await;
        if result.is_err() && stage.starts_with(&root) {
            let _ = std::fs::remove_dir_all(&stage);
        }
        result
    }
    pub fn remove(&self, id: &str) -> Result<(), String> {
        let skill = skill(id)?;
        let target = self.directory(&skill);
        if !target.exists() {
            return Ok(());
        }
        let root = self
            .root
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let resolved = target.canonicalize().map_err(|error| error.to_string())?;
        if !resolved.starts_with(&root) || resolved == root {
            return Err("Skill directory resolves outside its storage root.".into());
        }
        std::fs::remove_dir_all(target).map_err(|error| error.to_string())
    }
}
pub struct SkillReader {
    manager: Skills,
    active: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadRequest {
    skill_id: String,
    path: String,
    #[serde(default = "first_line")]
    start_line: usize,
    #[serde(default = "page_lines")]
    line_count: usize,
}
fn first_line() -> usize {
    1
}
fn page_lines() -> usize {
    200
}
impl SkillReader {
    pub fn call(&self, arguments: serde_json::Value) -> Result<serde_json::Value, String> {
        let request: ReadRequest = serde_json::from_value(arguments)
            .map_err(|error| format!("Invalid skill read: {error}"))?;
        if !self.active.contains(&request.skill_id) {
            return Err("This skill is not active for this turn.".into());
        }
        let text = self.manager.read(&request.skill_id, &request.path)?;
        excerpt(&text, request.start_line, request.line_count)
    }
}
fn excerpt(text: &str, start: usize, count: usize) -> Result<serde_json::Value, String> {
    if start == 0 || !(1..=500).contains(&count) {
        return Err("Use a positive start_line and a line_count between 1 and 500.".into());
    }
    let total = text.lines().count();
    if start > total.max(1) {
        return Err(format!("start_line exceeds this file's {total} lines."));
    }
    let mut lines = Vec::new();
    let mut bytes = 0;
    for (index, line) in text.lines().enumerate().skip(start - 1).take(count) {
        bytes += line.len();
        if bytes > 65_536 {
            if lines.is_empty() {
                return Err("This line exceeds the 64 KiB read limit.".into());
            }
            break;
        }
        lines.push(serde_json::json!({"number":index + 1,"text":line}));
    }
    let next = start + lines.len();
    Ok(
        serde_json::json!({"lines":lines,"total_lines":total,"has_more":next <= total,"next_line":if next <= total {Some(next)} else {None}}),
    )
}
#[tauri::command]
pub async fn list_skills(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<SkillView>, String> {
    let active = state.database()?.active_skills()?;
    Ok(state.skills.lock().await.list(&active))
}
#[tauri::command]
pub async fn install_skill(
    state: tauri::State<'_, crate::AppState>,
    id: String,
) -> Result<(), String> {
    state.skills.lock().await.install(&id).await
}
#[tauri::command]
pub async fn remove_skill(
    state: tauri::State<'_, crate::AppState>,
    id: String,
) -> Result<(), String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the current model operation to finish.")?;
    state.skills.lock().await.remove(&id)?;
    let store = state.database()?;
    let mut ids = store.active_skills()?;
    ids.retain(|value| value != &id);
    store.save_active_skills(&ids)
}
#[tauri::command]
pub async fn set_skill_active(
    state: tauri::State<'_, crate::AppState>,
    id: String,
    active: bool,
) -> Result<(), String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the current model operation to finish.")?;
    let skills = state.skills.lock().await;
    if active {
        skills.read(&id, "SKILL.md")?;
    } else {
        skill(&id)?;
    }
    let store = state.database()?;
    let mut ids = store.active_skills()?;
    ids.retain(|value| value != &id);
    if active {
        ids.push(id);
    }
    skills.instructions(&ids)?;
    store.save_active_skills(&ids)
}
#[tauri::command]
pub async fn read_skill_file(
    state: tauri::State<'_, crate::AppState>,
    id: String,
    path: String,
) -> Result<String, String> {
    state.skills.lock().await.read(&id, &path)
}

#[tauri::command]
pub async fn skill_update_status(
    state: tauri::State<'_, crate::AppState>,
    id: String,
) -> Result<SkillUpdateStatus, String> {
    state.skills.lock().await.update_status(&id)
}

#[tauri::command]
pub async fn skill_dependencies(
    state: tauri::State<'_, crate::AppState>,
    id: String,
) -> Result<Vec<SkillDependencyStatus>, String> {
    let execution = {
        let store = state.database()?;
        store.execution_config().unwrap_or_default()
    };
    let hub = state.connectors.lock().await;
    let skills = state.skills.lock().await.clone();
    skills.dependency_status(&id, &hub, &execution)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn versioned_update_status_distinguishes_missing_stale_and_damaged_packages() {
        let temp = tempfile::tempdir().unwrap();
        let manager = Skills::new(temp.path().into());
        let item = skill("wiki-qa").unwrap();
        let missing = manager.update_status("wiki-qa").unwrap();
        assert_eq!(missing.installed_revision, None);
        assert!(!missing.intact);
        assert!(missing.update_available);
        let directory = manager.directory(&item);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(".installed"), &item.revision).unwrap();
        std::fs::write(directory.join("SKILL.md"), "partial").unwrap();
        let damaged = manager.update_status("wiki-qa").unwrap();
        assert_eq!(damaged.installed_revision.as_deref(), Some(item.revision.as_str()));
        assert!(!damaged.intact);
        assert!(damaged.problem.as_deref().unwrap().contains("Reinstall"));
        assert!(manager.update_status("unknown-skill").is_err());
    }
    #[test]
    fn damaged_or_mismarked_packages_repair_instead_of_loading_partially() {
        let temp = tempfile::tempdir().unwrap();
        let manager = Skills::new(temp.path().into());
        let item = skill("wiki-qa").unwrap();
        let directory = manager.directory(&item);
        // A directory without a matching marker is incomplete: reads fail and
        // the package is not reported as installed.
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("SKILL.md"), "partial").unwrap();
        assert!(manager.read("wiki-qa", "SKILL.md").is_err());
        assert!(!manager.installed(&item));
        // A stale marker from another revision is not an installation either.
        std::fs::write(directory.join(".installed"), "stale-marker").unwrap();
        assert!(!manager.installed(&item));
        assert!(manager.read("wiki-qa", "SKILL.md").is_err());
    }
    #[test]
    fn every_skill_declares_reviewed_connector_and_interpreter_dependencies() {
        for item in catalog() {
            let dependencies = Skills::dependencies(&item.id).unwrap();
            assert!(!dependencies.is_empty(), "no dependencies for {}", item.id);
            for dependency in &dependencies {
                assert!(
                    matches!(
                        dependency.kind.as_str(),
                        "connector" | "interpreter" | "externalCli"
                    ),
                    "unknown kind for {}",
                    item.id
                );
                assert!(!dependency.name.trim().is_empty());
                assert!(!dependency.detail.trim().is_empty());
            }
            let packaged_scripts = item
                .files
                .iter()
                .filter(|file| {
                    file.path.ends_with(".py") || file.path.ends_with(".sh")
                })
                .count();
            let has = |kind: &str, name: &str| {
                dependencies
                    .iter()
                    .any(|dependency| dependency.kind == kind && dependency.name == name)
            };
            match item.id.as_str() {
                "mcp-builder" | "skill-creator" | "gh-fix-ci" | "jupyter-notebook" => {
                    assert!(has("interpreter", "python"), "python missing for {}", item.id);
                }
                "algorithmic-art" | "web-artifacts-builder" => {
                    assert!(has("interpreter", "node"), "node missing for {}", item.id);
                }
                _ => {}
            }
            if packaged_scripts > 0 {
                assert!(
                    dependencies.iter().any(|dependency| dependency.kind == "interpreter"),
                    "scripts without interpreter for {}",
                    item.id
                );
            }
            for name in ["tavily", "supabase", "linear", "github", "notion", "sentry", "deepwiki"] {
                if item.id.contains(name)
                    || (item.id == "wiki-architect" && name == "deepwiki")
                    || (item.id == "wiki-qa" && name == "deepwiki")
                {
                    assert!(has("connector", name), "{name} missing for {}", item.id);
                }
            }
        }
        assert!(Skills::dependencies("unknown-skill").is_err());
    }
    #[test]
    fn dependency_status_reports_actionable_remedies_without_launching() {
        let temp = tempfile::tempdir().unwrap();
        let manager = Skills::new(temp.path().into());
        let item = skill("gh-fix-ci").unwrap();
        let directory = manager.directory(&item);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(".installed"), &item.revision).unwrap();
        let hub = crate::connectors::McpHub::new(std::sync::Arc::new(crate::vault::Vault::new(
            temp.path().join("vault"),
        )));
        let empty = crate::execution::ExecutionConfig {
            python_path: String::new(),
            node_path: String::new(),
            powershell_path: String::new(),
        };
        let statuses = manager.dependency_status("gh-fix-ci", &hub, &empty).unwrap();
        assert_eq!(statuses.len(), 3);
        let by_name = |name: &str| statuses.iter().find(|status| status.dependency.name == name).unwrap();
        // PATH-dependent CLI rows vary by machine; only the actionable remedy is asserted.
        assert!(!by_name("github").satisfied);
        assert!(!by_name("python").satisfied);
        assert!(!by_name("gh").remedy.trim().is_empty());
        for status in &statuses {
            assert!(!status.remedy.trim().is_empty());
        }
        let connector = statuses
            .iter()
            .find(|status| status.dependency.name == "github")
            .unwrap();
        assert!(connector.remedy.contains("Connect github in Connectors"));
        let interpreter = statuses
            .iter()
            .find(|status| status.dependency.name == "python")
            .unwrap();
        assert!(interpreter.remedy.contains("Execution"));
        let cli = statuses
            .iter()
            .find(|status| status.dependency.name == "gh")
            .unwrap();
        assert!(
            cli.remedy.contains("never installs") || cli.remedy.contains("available on PATH"),
            "unexpected gh remedy: {}",
            cli.remedy
        );
        // A configured interpreter flips only its own row; connectors stay missing.
        let python = std::env::current_exe().unwrap().to_string_lossy().into_owned();
        let configured = crate::execution::ExecutionConfig {
            python_path: python,
            node_path: String::new(),
            powershell_path: String::new(),
        };
        let statuses = manager.dependency_status("gh-fix-ci", &hub, &configured).unwrap();
        assert!(statuses
            .iter()
            .find(|status| status.dependency.name == "python")
            .unwrap()
            .satisfied);
        assert!(!statuses
            .iter()
            .find(|status| status.dependency.name == "github")
            .unwrap()
            .satisfied);
        assert!(manager.dependency_status("gh-fix-ci", &hub, &empty).unwrap().len() == 3);
        assert!(manager.dependency_status("unknown-skill", &hub, &empty).is_err());
        let fresh = Skills::new(temp.path().join("empty-root"));
        assert!(fresh.dependency_status("gh-fix-ci", &hub, &empty).unwrap_err().contains("Install this skill"));
    }
    #[cfg(windows)]
    #[test]
    fn package_junction_cannot_escape_skill_storage() {
        use std::os::windows::process::CommandExt;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let manager = Skills::new(root.path().into());
        let item = skill("wiki-qa").unwrap();
        std::fs::write(outside.path().join(".installed"), &item.revision).unwrap();
        let size = item
            .files
            .iter()
            .find(|file| file.path == "SKILL.md")
            .unwrap()
            .size;
        std::fs::write(outside.path().join("SKILL.md"), vec![b'x'; size]).unwrap();
        let output = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", "New-Item -ItemType Junction -Path $env:LOCALLM_TEST_LINK -Target $env:LOCALLM_TEST_TARGET -ErrorAction Stop | Out-Null"])
            .env("LOCALLM_TEST_LINK", manager.directory(&item))
            .env("LOCALLM_TEST_TARGET", outside.path())
            .creation_flags(0x08000000).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let error = manager.read("wiki-qa", "SKILL.md").unwrap_err();
        // An integrity error would mean we reached the outside file. The directory capability must reject it first.
        assert!(!error.contains("Integrity check failed"), "{error}");
        assert!(!error.contains("size changed"), "{error}");
        assert_eq!(
            std::fs::metadata(outside.path().join("SKILL.md"))
                .unwrap()
                .len(),
            size as u64
        );
    }
    #[test]
    fn reference_pages_preserve_unicode_and_report_continuation_without_silent_truncation() {
        let page = excerpt("one\nবাংলা\nthree\nfour", 2, 2).unwrap();
        assert_eq!(page["lines"][0]["text"], "বাংলা");
        assert_eq!(page["lines"][0]["number"], 2);
        assert_eq!(page["next_line"], 4);
        assert_eq!(page["has_more"], true);
        let last = excerpt("one\ntwo", 2, 200).unwrap();
        assert_eq!(last["has_more"], false);
        assert!(last["next_line"].is_null());
        assert_eq!(excerpt("", 1, 200).unwrap()["lines"], serde_json::json!([]));
        for (start, count) in [(0, 1), (1, 0), (1, 501), (usize::MAX, 1)] {
            assert!(excerpt("one", start, count).is_err());
        }
        let large = format!("{}\n{}", "a".repeat(40_000), "b".repeat(40_000));
        let first = excerpt(&large, 1, 200).unwrap();
        assert_eq!(first["lines"].as_array().unwrap().len(), 1);
        assert_eq!(first["next_line"], 2);
        assert!(excerpt(&"x".repeat(65_537), 1, 1).is_err());
    }
    #[test]
    fn skill_reader_rejects_inactive_packages_extra_arguments_and_unverified_files() {
        let temp = tempfile::tempdir().unwrap();
        let manager = Skills::new(temp.path().into());
        assert!(manager.reader(&[]).unwrap().is_none());
        let reader = SkillReader {
            manager,
            active: vec!["wiki-qa".into()],
        };
        let error = reader
            .call(serde_json::json!({"skill_id":"sentry","path":"SKILL.md"}))
            .unwrap_err();
        assert!(error.contains("not active"));
        assert!(reader
            .call(serde_json::json!({"skill_id":"wiki-qa","path":"SKILL.md","execute":true}))
            .is_err());
        assert!(reader
            .call(serde_json::json!({"skill_id":"wiki-qa","path":"../secret"}))
            .is_err());
        assert!(reader
            .call(serde_json::json!({"skill_id":"wiki-qa","path":"SKILL.md"}))
            .is_err());
        let tool = crate::connectors::AgentTool::skills(
            std::sync::Arc::new(reader),
            crate::connectors::ToolView {
                name: "read_file".into(),
                description: String::new(),
                input_schema: serde_json::json!({}),
            },
        );
        assert!(!tool.trusted_read());
        assert_eq!(
            tool.alias,
            crate::connectors::tool_alias("Skills", "read_file")
        );
    }
    #[test]
    fn all_presets_are_pinned_and_paths_are_safe() {
        let skills = catalog();
        assert_eq!(skills.len(), 13);
        for skill in skills {
            assert_eq!(skill.revision.len(), 40);
            assert!(skill.files.iter().any(|file| file.path == "SKILL.md"));
            for file in skill.files {
                relative(&file.path).unwrap();
                relative(&file.source_path).unwrap();
                assert!(file.size <= 4 * 1024 * 1024);
                assert_eq!(file.sha256.len(), 64);
            }
        }
        for path in [
            "../secret",
            "/absolute",
            "C:/secret",
            "a\\b",
            "a/../b",
            "a/",
            "a:stream",
        ] {
            assert!(relative(path).is_err());
        }
    }
    #[test]
    fn rejects_modified_package_content_and_unknown_files() {
        let temp = tempfile::tempdir().unwrap();
        let manager = Skills::new(temp.path().into());
        let item = skill("wiki-qa").unwrap();
        let directory = manager.directory(&item);
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join(".installed"), &item.revision).unwrap();
        std::fs::write(directory.join("SKILL.md"), "tampered").unwrap();
        assert!(manager.read("wiki-qa", "SKILL.md").is_err());
        assert!(manager.read("wiki-qa", "../secret").is_err());
        assert!(manager.read("wiki-qa", "other.txt").is_err());
        manager.remove("wiki-qa").unwrap();
        assert!(!directory.exists());
    }
}
