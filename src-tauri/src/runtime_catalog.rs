//! Pinned, independently installable engines. Runtime selection never accepts URLs.
use crate::runtime_archive::RuntimeAsset;
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize)]
pub struct Package {
    pub id: String,
    pub os: String,
    pub arch: String,
    pub backend: String,
    pub build: String,
    pub executable: String,
    pub assets: Vec<RuntimeAsset>,
}
impl Package {
    pub fn prefix(&self) -> String {
        if self.id == "llama-cuda" {
            "llama-b10855-cuda12.4-".into()
        } else {
            format!("{}-{}-", self.id, self.build)
        }
    }
    pub fn supported(&self) -> bool {
        self.os == std::env::consts::OS && self.arch == std::env::consts::ARCH
    }
}
pub fn packages() -> Result<Vec<Package>, String> {
    let mut packages: Vec<Package> =
        serde_json::from_str(include_str!("../../catalog/prism-runtimes.json"))
            .map_err(|e| e.to_string())?;
    packages.push(Package {
        id: "llama-cuda".into(),
        os: "windows".into(),
        arch: "x86_64".into(),
        backend: "CUDA 12.4".into(),
        build: "b10855".into(),
        executable: "llama-server.exe".into(),
        assets: crate::runtime_archive::catalog()?,
    });
    Ok(packages)
}
pub fn package(id: Option<&str>) -> Result<Package, String> {
    packages()?
        .into_iter()
        .find(|p| p.id == id.unwrap_or("llama-cuda"))
        .ok_or_else(|| "Choose a runtime from the managed catalog.".into())
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    id: String,
    backend: String,
    build: String,
}
#[tauri::command]
pub fn prism_runtime_choices() -> Result<Vec<Choice>, String> {
    Ok(packages()?
        .into_iter()
        .filter(|p| p.id.starts_with("prism-") && p.supported())
        .map(|p| Choice {
            id: p.id,
            backend: p.backend,
            build: p.build,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packages_pin_every_download_and_keep_distinct_installation_identities() {
        let packages = packages().unwrap();
        let mut ids = std::collections::HashSet::new();
        for package in &packages {
            assert!(ids.insert(package.prefix()));
            let mut asset_names = std::collections::HashSet::new();
            if package.id.starts_with("prism-") {
                assert!(package.assets[0].name.starts_with("llama-prism-"));
            }
            for asset in &package.assets {
                assert!(asset_names.insert(&asset.name));
                assert!(asset.url.starts_with("https://github.com/"));
                assert!(asset.url.contains(&package.build));
                assert_eq!(asset.sha256.len(), 64);
                assert!(asset.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
                assert!(asset.bytes > 0);
            }
        }
        assert!(package(Some("../../custom")).is_err());
        for choice in prism_runtime_choices().unwrap() {
            assert!(package(Some(&choice.id)).unwrap().supported());
        }
    }
}
