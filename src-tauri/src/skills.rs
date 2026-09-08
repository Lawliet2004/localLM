use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
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
        let directory = self
            .directory(&skill)
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let root = self
            .root
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let target = directory
            .join(path)
            .canonicalize()
            .map_err(|error| error.to_string())?;
        if !directory.starts_with(&root) || !target.starts_with(&directory) {
            return Err("Skill file resolves outside its package.".into());
        }
        if std::fs::metadata(&target)
            .map_err(|error| error.to_string())?
            .len()
            != file.size as u64
        {
            return Err("Skill file size changed. Reinstall the skill.".into());
        }
        let bytes = std::fs::read(target).map_err(|error| error.to_string())?;
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
        }
        if context.len() > 65_536 {
            return Err("Selected skill instructions exceed 64 KiB. Activate fewer skills for this conversation.".into());
        }
        Ok(context)
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
