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
        if self.installed(&skill) {
            for file in &skill.files {
                let bytes = std::fs::read(self.directory(&skill).join(&file.path))
                    .map_err(|error| error.to_string())?;
                verify(file, &bytes)?;
            }
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
            std::fs::rename(&stage, self.directory(&skill)).map_err(|error| error.to_string())?;
            Ok(())
        }
        .await;
        if result.is_err() && stage.starts_with(&root) {
            let _ = std::fs::remove_dir_all(&stage);
        }
        result
    }
    pub fn remove(&self, id: &str) -> Result<(), String> {
        let target = self.directory(&skill(id)?);
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

#[cfg(test)]
mod tests {
    use super::*;
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
