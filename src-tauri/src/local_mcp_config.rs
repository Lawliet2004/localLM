//! User-authored process configuration. Saving never starts a process.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    path::Path,
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalServer {
    pub id: String,
    pub name: String,
    pub executable: String,
    pub arguments: Vec<String>,
    pub working_directory: String,
    pub environment: BTreeMap<String, String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalServerSummary {
    pub id: String,
    pub name: String,
    pub executable: String,
    pub working_directory: String,
    pub argument_count: usize,
    pub environment_names: Vec<String>,
}
impl From<&LocalServer> for LocalServerSummary {
    fn from(server: &LocalServer) -> Self {
        Self {
            id: server.id.clone(),
            name: server.name.clone(),
            executable: server.executable.clone(),
            working_directory: server.working_directory.clone(),
            argument_count: server.arguments.len(),
            environment_names: server.environment.keys().cloned().collect(),
        }
    }
}
impl LocalServer {
    pub fn validate(&self) -> Result<(), String> {
        if self
            .id
            .strip_prefix("local-")
            .and_then(|id| uuid::Uuid::parse_str(id).ok())
            .is_none()
        {
            return Err("Invalid local connector identifier.".into());
        }
        if self.name.trim().is_empty()
            || self.name.len() > 160
            || self.name.chars().any(char::is_control)
        {
            return Err(
                "Use a connector name between 1 and 160 bytes without control characters.".into(),
            );
        }
        for path in [&self.executable, &self.working_directory] {
            if path.len() > 32768 || path.contains('\0') || !Path::new(path).is_absolute() {
                return Err(
                    "Local connector executable and working directory must be absolute paths."
                        .into(),
                );
            }
        }
        if self.arguments.len() > 128
            || self.arguments.iter().map(String::len).sum::<usize>() > 32768
            || self
                .arguments
                .iter()
                .any(|argument| argument.contains('\0'))
        {
            return Err("Local connector arguments exceed their limits or contain NUL.".into());
        }
        if self.environment.len() > 64 {
            return Err("At most 64 environment variables are supported.".into());
        }
        let mut names = HashSet::new();
        let mut bytes = 0;
        for (name, value) in &self.environment {
            if name.is_empty()
                || name.len() > 128
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                || name.as_bytes()[0].is_ascii_digit()
                || !names.insert(name.to_ascii_uppercase())
            {
                return Err(
                    "Environment names must be identifiers unique regardless of letter case."
                        .into(),
                );
            }
            if value.contains('\0') || value.len() > 8192 {
                return Err("Environment value exceeds its limit or contains NUL.".into());
            }
            bytes += name.len() + value.len();
        }
        if bytes > 32768 {
            return Err("Environment configuration exceeds 32 KiB.".into());
        }
        Ok(())
    }
    pub fn validate_launch(&self) -> Result<(), String> {
        self.validate()?;
        if !Path::new(&self.executable).is_file() || !Path::new(&self.working_directory).is_dir() {
            return Err("The local connector executable or working directory is missing.".into());
        }
        Ok(())
    }
}

pub fn load(vault: &crate::vault::Vault) -> Result<Vec<LocalServer>, String> {
    let servers: Vec<LocalServer> = match vault.load("local-mcp-configs")? {
        Some(bytes) => serde_json::from_slice(&bytes)
            .map_err(|_| "Saved local connector configuration is invalid.")?,
        None => Vec::new(),
    };
    validate_collection(&servers)?;
    Ok(servers)
}
fn validate_collection(servers: &[LocalServer]) -> Result<(), String> {
    if servers.len() > 32 {
        return Err("At most 32 local connectors can be configured.".into());
    }
    let mut ids = HashSet::new();
    for server in servers {
        server.validate()?;
        if !ids.insert(&server.id) {
            return Err("Duplicate local connector identifier.".into());
        }
    }
    Ok(())
}
pub fn save(vault: &crate::vault::Vault, servers: &[LocalServer]) -> Result<(), String> {
    validate_collection(servers)?;
    let bytes = serde_json::to_vec(servers)
        .map_err(|_| "Could not encode local connector configuration.")?;
    if bytes.len() > 131072 {
        return Err("Local connector configurations exceed 128 KiB.".into());
    }
    vault.save("local-mcp-configs", &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_process_arguments_and_encrypts_environment() {
        let temp = tempfile::tempdir().unwrap();
        let vault = crate::vault::Vault::new(temp.path().join("vault"));
        let mut server = LocalServer {
            id: format!("local-{}", uuid::Uuid::new_v4()),
            name: "Test server".into(),
            executable: std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            arguments: vec!["a b".into(), "--stdio".into()],
            working_directory: temp.path().to_string_lossy().into_owned(),
            environment: [("API_KEY".into(), "fixture-secret-no-network".into())].into(),
        };
        server.validate_launch().unwrap();
        save(&vault, &[server.clone()]).unwrap();
        assert_eq!(load(&vault).unwrap()[0].arguments, server.arguments);
        assert!(!String::from_utf8_lossy(
            &std::fs::read(temp.path().join("vault/local-mcp-configs.sealed")).unwrap()
        )
        .contains("fixture-secret-no-network"));
        assert!(save(&vault, &[server.clone(), server.clone()]).is_err());
        server
            .environment
            .insert("api_key".into(), "duplicate".into());
        assert!(server.validate().is_err());
        server.environment.clear();
        server.arguments.push("invalid\0argument".into());
        assert!(server.validate().is_err());
        server.arguments.clear();
        server.executable = "relative.exe".into();
        assert!(server.validate().is_err());
    }
}
