// Audit-only isolated fixture, copied from current production functions.
// known_source is stubbed to None for the generic owner/repo test.
// runtime_archive is a minimal manifest with the production naming shape.
use serde::{Serialize,Deserialize};
use std::path::{Path,PathBuf};
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HubFile {
    pub filename: String,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HubRepo {
    pub repo: String,
    pub revision: String,
    pub files: Vec<HubFile>,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HubSearchHit {
    pub id: String,
    pub downloads: Option<u64>,
    pub likes: Option<u64>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModel {
    pub id: String,
    pub filename: String,
    pub path: String,
    pub bytes: u64,
    pub repo: Option<String>,
    pub revision: Option<String>,
    pub files: Vec<String>,
    pub complete: bool,
    #[serde(default)]
    pub received: u64,
    #[serde(default)]
    pub projector_filename: Option<String>,
    #[serde(default)]
    pub projector_path: Option<String>,
}

fn valid_component(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 200
        && !s.starts_with('.')
        && !s.ends_with('.')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}
fn validate_repo(repo: &str) -> Result<(), String> {
    let parts: Vec<_> = repo.split('/').collect();
    if parts.len() != 2 || !parts.iter().all(|part| valid_component(part)) {
        return Err("Enter a Hugging Face repository as owner/model-name.".into());
    }
    Ok(())
}
fn valid_filename(name: &str) -> bool {
    name.len() < 1024
        && name.split('/').all(valid_component)
        && name.to_lowercase().ends_with(".gguf")
}
fn is_companion_path(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("mmproj")
        || lower.split('/').any(|part| {
            [
                "mtp",
                "imatrix",
                "adapter",
                "adapters",
                "lora",
                "projector",
                "projectors",
            ]
            .iter()
            .any(|prefix| {
                part.strip_prefix(*prefix)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with(['.', '_', '-']))
            })
        })
}
fn is_weight_file(name: &str) -> bool {
    valid_filename(name) && !is_companion_path(name)
}
fn selected_files<'a>(files: &'a [HubFile], name: &str) -> Result<Vec<&'a HubFile>, String> {
    if !is_weight_file(name) {
        return Err("Select standalone model weights. MTP heads, projectors, calibration files, and adapters are companion files.".into());
    }
    let selected = files
        .iter()
        .find(|f| f.filename == name)
        .ok_or("File not found in this revision.")?;
    let stem = name.strip_suffix(".gguf").unwrap_or(name);
    if let Some((left, count)) = stem.rsplit_once("-of-") {
        let (base, index) = left.rsplit_once('-').ok_or("Invalid GGUF shard name.")?;
        let count: usize = count.parse().map_err(|_| "Invalid GGUF shard count.")?;
        if index.len() != 5 || count == 0 || count > 999 {
            return Err("Unsupported GGUF shard count.".into());
        }
        return (1..=count)
            .map(|i| {
                let name = format!("{base}-{i:05}-of-{count:05}.gguf");
                files
                    .iter()
                    .find(|f| f.filename == name)
                    .ok_or_else(|| format!("Missing shard: {name}"))
            })
            .collect();
    }
    Ok(vec![selected])
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadState {
    repo: String,
    revision: String,
    filename: String,
    bytes: u64,
    sha256: String,
    received: u64,
    // Option keeps download.json files from earlier builds readable. Legacy
    // state can recover the choice when its saved total uniquely matches one
    // projector plus the selected weights.
    projector: Option<String>,
}
fn known_source(_: &str)->Option<String>{None}
fn scan(directory: &Path, depth: usize, result: &mut Vec<InstalledModel>) -> Result<(), String> {
    if !directory.exists() {
        return Ok(());
    }
    let root = std::fs::canonicalize(directory).map_err(|e| e.to_string())?;
    let mut files = Vec::new();
    for item in std::fs::read_dir(directory).map_err(|e| e.to_string())? {
        let item = item.map_err(|e| e.to_string())?;
        let path = item.path();
        let canonical = std::fs::canonicalize(&path).map_err(|e| e.to_string())?;
        if !canonical.starts_with(&root) {
            continue;
        }
        if item.file_type().map_err(|e| e.to_string())?.is_symlink() {
            continue;
        }
        if path.is_dir() && depth > 0 && !is_companion_path(&item.file_name().to_string_lossy()) {
            scan(&path, depth - 1, result)?;
        } else if path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("gguf"))
            && path.is_file()
        {
            files.push((
                canonical,
                std::fs::metadata(&path).map_err(|e| e.to_string())?.len(),
            ));
        }
    }
    let source = std::fs::read(directory.join("model.json"))
        .ok()
        .and_then(|v| serde_json::from_slice::<InstalledModel>(&v).ok());
    let hub_files: Vec<_> = files
        .iter()
        .map(|(p, bytes)| HubFile {
            filename: p.file_name().unwrap().to_string_lossy().into_owned(),
            bytes: *bytes,
            sha256: String::new(),
        })
        .collect();
    for file in &hub_files {
        if !is_weight_file(&file.filename) {
            continue;
        }
        if file.filename.contains("-of-") && !file.filename.contains("-00001-of-") {
            continue;
        }
        let group = selected_files(&hub_files, &file.filename);
        let managed = directory
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.len() == 64 && n.bytes().all(|b| b.is_ascii_hexdigit()));
        let complete = group.is_ok() && (!managed || source.is_some());
        let group = group.unwrap_or_else(|_| {
            let base = file
                .filename
                .split("-00001-of-")
                .next()
                .unwrap_or(&file.filename);
            hub_files
                .iter()
                .filter(|f| {
                    f.filename.starts_with(&format!("{base}-")) && f.filename.contains("-of-")
                })
                .collect()
        });
        let paths: Vec<_> = group
            .iter()
            .map(|f| root.join(&f.filename).to_string_lossy().into_owned())
            .collect();
        let path = paths[0].clone();
        if result.iter().any(|m| m.path == path) {
            continue;
        }
        let bytes = group.iter().map(|f| f.bytes).sum();
        let (projector_filename, projector_path) = match &source {
            Some(manifest) if manifest.path == path => (
                manifest.projector_filename.clone(),
                manifest.projector_path.clone(),
            ),
            _ => {
                let sibling = group.iter().find_map(|f| {
                    let dir = root.join(&f.filename).parent()?.to_path_buf();
                    let entries = std::fs::read_dir(&dir).ok()?;
                    entries.flatten().find_map(|entry| {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        if name.to_lowercase().contains("mmproj")
                            && name.to_lowercase().ends_with(".gguf")
                        {
                            Some((name.clone(), dir.join(&name).to_string_lossy().into_owned()))
                        } else {
                            None
                        }
                    })
                });
                sibling
                    .map(|(name, path)| (Some(name), Some(path)))
                    .unwrap_or((None, None))
            }
        };
        result.push(InstalledModel {
            id: path.clone(),
            path,
            filename: file.filename.clone(),
            bytes,
            repo: source
                .as_ref()
                .and_then(|s| s.repo.clone())
                .or_else(|| known_source(&file.filename)),
            revision: source.as_ref().and_then(|s| s.revision.clone()),
            files: paths,
            complete,
            received: if complete {
                bytes
            } else {
                group.iter().map(|f| f.bytes).sum()
            },
            projector_filename,
            projector_path,
        });
    }
    if let Some(state) = pending_download(directory).filter(|state| is_weight_file(&state.filename))
    {
        let dest = root.join(state.filename.rsplit('/').next().unwrap_or(&state.filename));
        let part = download::partial_path(&dest);
        let received = std::fs::metadata(&part)
            .map(|m| m.len())
            .unwrap_or(state.received);
        if !result.iter().any(|m| {
            m.filename == state.filename.rsplit('/').next().unwrap_or(&state.filename)
                || Path::new(&m.path) == dest.as_path()
        }) {
            let path = if dest.exists() { dest } else { part.clone() };
            let path_string = path.to_string_lossy().into_owned();
            result.push(InstalledModel {
                id: path_string.clone(),
                path: path_string.clone(),
                filename: state
                    .filename
                    .rsplit('/')
                    .next()
                    .unwrap_or(&state.filename)
                    .to_owned(),
                bytes: state.bytes,
                repo: Some(state.repo),
                revision: Some(state.revision),
                files: vec![path_string],
                complete: false,
                received,
                projector_filename: state.projector.clone(),
                projector_path: None,
            });
        }
    }
    Ok(())
}
fn pending_download(directory: &Path) -> Option<DownloadState> {
    serde_json::from_slice(&std::fs::read(directory.join("download.json")).ok()?).ok()
}
mod download { use std::path::{Path,PathBuf}; pub fn partial_path(p:&Path)->PathBuf {let mut n=p.file_name().unwrap().to_os_string();n.push(".part");p.with_file_name(n)}}
mod runtime_archive { pub struct File{pub name:String} pub struct Asset {pub name:String,pub files:Vec<File>} pub fn catalog()->Result<Vec<Asset>,String>{Ok(vec![Asset{name:"archive.zip".into(),files:vec![File{name:"llama-server.exe".into()}]}])}}
mod recovery { //! Startup-only cleanup of the installer's bounded, recognizable staging layout.
use cap_std::fs::Dir;
use std::path::Path;

fn owned(name: &str, prefix: &str, suffix: &str) -> bool {
    name.strip_prefix(prefix)
        .and_then(|name| name.strip_suffix(suffix))
        .is_some_and(|random| {
            random.len() == 32 && random.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
}
fn entries(directory: &Dir) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    for entry in directory.entries().map_err(|error| error.to_string())? {
        if names.len() >= 1000 {
            return Err("Too many staging entries; automatic cleanup stopped.".into());
        }
        names.push(
            entry
                .map_err(|error| error.to_string())?
                .file_name()
                .to_string_lossy()
                .into_owned(),
        );
    }
    Ok(names)
}
fn regular(directory: &Dir, name: &str) -> bool {
    directory
        .symlink_metadata(name)
        .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
}

/// A partial download left behind by an interrupted install.
///
/// The downloader writes `<name>.part` next to its destination: either
/// directly inside `<models>/` or inside a model's `<64-hex-digest>/`
/// directory. The startup collector must recognise those names, or an
/// interrupted multi-gigabyte download is never reclaimed and the app-data
/// directory grows without bound.
fn partial(name: &str) -> bool {
    name.ends_with(".part") && name.len() > ".part".len()
}
pub fn clean(root: &Path, runtime: bool) -> Result<usize, String> {
    if !root.exists() {
        return Ok(0);
    }
    let directory = Dir::open_ambient_dir(root, cap_std::ambient_authority())
        .map_err(|error| error.to_string())?;
    let mut removed = 0;
    for name in entries(&directory)? {
        if !runtime {
            if name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit()) {
                let metadata = directory
                    .symlink_metadata(&name)
                    .map_err(|e| e.to_string())?;
                if metadata.is_dir() && !metadata.file_type().is_symlink() {
                    let model = directory.open_dir(&name).map_err(|e| e.to_string())?;
                    for file in entries(&model)? {
                        if (owned(&file, ".locallm-download-", ".part") || partial(&file))
                            && regular(&model, &file)
                        {
                            model.remove_file(&file).map_err(|e| e.to_string())?;
                            removed += 1;
                        }
                    }
                }
            }
            if (owned(&name, ".locallm-download-", ".part") || partial(&name))
                && regular(&directory, &name)
            {
                directory
                    .remove_file(&name)
                    .map_err(|error| error.to_string())?;
                removed += 1;
            }
            continue;
        }
        if !owned(&name, ".locallm-runtime-", "") {
            continue;
        }
        let metadata = directory
            .symlink_metadata(&name)
            .map_err(|error| error.to_string())?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            continue;
        }
        let staging = directory
            .open_dir(&name)
            .map_err(|error| error.to_string())?;
        let assets = crate::runtime_archive::catalog()?;
        let mut files = Vec::new();
        let mut has_bundle = false;
        // Validate the entire small layout before removing anything; never recurse through links.
        for entry in entries(&staging)? {
            if entry == "bundle" {
                let metadata = staging
                    .symlink_metadata(&entry)
                    .map_err(|error| error.to_string())?;
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    return Err("Unexpected staging link; files retained.".into());
                }
                let bundle = staging
                    .open_dir("bundle")
                    .map_err(|error| error.to_string())?;
                for file in entries(&bundle)? {
                    if !regular(&bundle, &file)
                        || !assets
                            .iter()
                            .flat_map(|asset| &asset.files)
                            .any(|expected| expected.name == file)
                    {
                        return Err("Unexpected runtime staging content; files retained.".into());
                    }
                    files.push(format!("bundle/{file}"));
                }
                has_bundle = true;
            } else if regular(&staging, &entry)
                && (assets.iter().any(|asset| asset.name == entry)
                    || owned(&entry, ".locallm-download-", ".part"))
            {
                files.push(entry);
            } else {
                return Err("Unexpected runtime staging content; files retained.".into());
            }
        }
        for file in files {
            staging
                .remove_file(&file)
                .map_err(|error| error.to_string())?;
        }
        if has_bundle {
            staging
                .remove_dir("bundle")
                .map_err(|error| error.to_string())?;
        }
        drop(staging);
        directory
            .remove_dir(&name)
            .map_err(|error| error.to_string())?;
        removed += 1;
    }
    Ok(removed)
}

}

fn main(){
 let root=PathBuf::from(std::env::args().nth(1).unwrap());std::fs::create_dir_all(&root).unwrap();
 let models=root.join("models"); let dir=models.join("a".repeat(64));std::fs::create_dir_all(&dir).unwrap();
 std::fs::write(dir.join("model.gguf.part"),b"saved-prefix").unwrap();
 std::fs::write(dir.join("download.json"),r#"{"repo":"owner/repo","revision":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","filename":"folder/model.gguf","bytes":100,"sha256":"hash","received":12,"projector":null}"#).unwrap();
 println!("restart cleanup removes partials: {}",recovery::clean(&models,false).unwrap());
 let mut list=Vec::new();scan(&models,1,&mut list).unwrap();println!("remaining item: {}",serde_json::to_string(&list).unwrap());
 println!("deleting phantom file: {:?}",std::fs::remove_file(&list[0].files[0]));
 std::fs::write(dir.join("model.gguf"),b"weights").unwrap();
 list.clear();scan(&models,1,&mut list).unwrap();println!("weights finished but projector pending: {}",serde_json::to_string(&list).unwrap());
 let runtimes=root.join("runtimes");let staging=runtimes.join(format!(".locallm-runtime-{}","a".repeat(32)));std::fs::create_dir_all(&staging).unwrap();std::fs::write(staging.join("archive.zip.part"),b"partial").unwrap();
 println!("crashed runtime cleanup: {:?}",recovery::clean(&runtimes,true));
 #[cfg(unix)] {std::os::unix::fs::symlink("does-not-exist",models.join("broken-link")).unwrap();list.clear();println!("inventory with broken symlink: {:?}",scan(&models,1,&mut list));}
}
