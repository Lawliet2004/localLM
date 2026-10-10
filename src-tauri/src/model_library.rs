//! Hugging Face discovery and a disk-backed library. Only verified weights are published.
use crate::{download, model_install::Status, AppState};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;

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
    #[serde(default)]
    pub external: bool,
    /// Basename, bytes and sha256 of every weight shard and the projector,
    /// written into model.json at publish time.
    #[serde(default)]
    pub expected: Vec<HubFile>,
    /// Computed at scan time; the value stored on disk is never trusted.
    #[serde(default)]
    pub verified: bool,
}

/// An mmproj/vision-projector companion for a multimodal GGUF model.
///
/// The vision checkpoint is what actually encodes images; loading the text
/// weights alone leaves a multimodal model text-only, so llama.cpp must get
/// `--mmproj`. A missing selection explicitly means text-only. When supplied,
/// the filename must exactly match a projector from the pinned repo revision.
pub fn projector_candidates(files: &[HubFile]) -> Vec<&HubFile> {
    let mut result: Vec<&HubFile> = files
        .iter()
        .filter(|file| {
            let lower = file.filename.to_lowercase();
            lower.ends_with(".gguf") && lower.contains("mmproj")
        })
        .collect();
    result.sort_by(|a, b| a.filename.cmp(&b.filename));
    result
}

pub fn select_projector<'a>(
    files: &'a [HubFile],
    _weight_filename: &str,
    explicit: Option<&str>,
) -> Result<Option<&'a HubFile>, String> {
    let Some(name) = explicit else {
        return Ok(None);
    };
    let candidates = projector_candidates(files);
    if candidates.is_empty() {
        return Err("This repository ships no mmproj projector file.".into());
    }
    candidates
        .iter()
        .find(|file| file.filename.eq_ignore_ascii_case(name))
        .map(|file| Some(*file))
        .ok_or_else(|| "Projector not found in this revision.".into())
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
fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

async fn hub_json(url: reqwest::Url, token: Option<&str>) -> Result<serde_json::Value, String> {
    let client = download::client()?;
    let mut request = client.get(url).timeout(Duration::from_secs(30));
    if let Some(token) = token.filter(|s| !s.is_empty()) {
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .await
        .map_err(|_| "Could not reach Hugging Face. Check your connection and retry.")?;
    match response.status().as_u16() {
        401 | 403 => return Err("Access denied. Accept the model's license on Hugging Face and supply a read token with access to this repository.".into()),
        404 => return Err("Repository or revision not found on Hugging Face.".into()),
        429 => return Err("Hugging Face rate limit reached. Wait a moment and retry.".into()),
        _ => {}
    }
    let response = response.error_for_status().map_err(|e| {
        format!(
            "Hugging Face returned {}.",
            e.status().map(|s| s.to_string()).unwrap_or_default()
        )
    })?;
    // API responses must not allocate an unbounded body.
    use futures_util::StreamExt;
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "Hugging Face response interrupted.")?;
        if body.len() + chunk.len() > 16 * 1024 * 1024 {
            return Err("Repository metadata exceeds 16 MiB.".into());
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| "Hugging Face returned invalid metadata.".into())
}

fn hugging_face_search_url(query: &str) -> reqwest::Url {
    let mut url = reqwest::Url::parse("https://huggingface.co/api/models").unwrap();
    {
        let mut pairs = url.query_pairs_mut();
        pairs.extend_pairs([
            ("filter", "gguf"),
            ("sort", "downloads"),
            ("direction", "-1"),
            ("limit", "30"),
        ]);
        let trimmed = query.trim();
        if !trimmed.is_empty() {
            pairs.append_pair("search", trimmed);
        }
    }
    url
}

fn parse_search_hits(data: &serde_json::Value) -> Result<Vec<HubSearchHit>, String> {
    Ok(data
        .as_array()
        .ok_or("Invalid search response.")?
        .iter()
        .filter_map(|value| {
            let id = value["id"].as_str()?;
            if validate_repo(id).is_err() {
                return None;
            }
            Some(HubSearchHit {
                id: id.to_owned(),
                downloads: value["downloads"].as_u64(),
                likes: value["likes"].as_u64(),
            })
        })
        .collect())
}

#[tauri::command]
pub async fn search_hugging_face(
    query: String,
    token: Option<String>,
) -> Result<Vec<HubSearchHit>, String> {
    if query.len() > 200 {
        return Err("Search must be at most 200 characters.".into());
    }
    let data = hub_json(hugging_face_search_url(&query), token.as_deref()).await?;
    parse_search_hits(&data)
}

async fn repository(
    repo: String,
    revision: Option<&str>,
    token: Option<&str>,
) -> Result<HubRepo, String> {
    validate_repo(&repo)?;
    let suffix = if let Some(rev) = revision {
        if rev.len() != 40 || !rev.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Invalid immutable repository revision.".into());
        }
        format!("/revision/{rev}")
    } else {
        String::new()
    };
    let url = reqwest::Url::parse(&format!(
        "https://huggingface.co/api/models/{repo}{suffix}?blobs=true"
    ))
    .unwrap();
    let data = hub_json(url, token).await?;
    let revision = data["sha"]
        .as_str()
        .filter(|s| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or("Repository has no immutable revision.")?
        .to_owned();
    let mut files = Vec::new();
    for item in data["siblings"]
        .as_array()
        .ok_or("Repository has no file listing.")?
    {
        let Some(name) = item["rfilename"].as_str().filter(|s| valid_filename(s)) else {
            continue;
        };
        let Some(hash) = item["lfs"]["sha256"]
            .as_str()
            .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        else {
            continue;
        };
        let Some(bytes) = item["lfs"]["size"].as_u64().filter(|n| *n > 0) else {
            continue;
        };
        files.push(HubFile {
            filename: name.into(),
            bytes,
            sha256: hash.into(),
        });
    }
    files.sort_by(|a, b| a.filename.cmp(&b.filename));
    Ok(HubRepo {
        repo,
        revision,
        files,
    })
}
#[tauri::command]
pub async fn hugging_face_files(repo: String, token: Option<String>) -> Result<HubRepo, String> {
    repository(repo, None, token.as_deref()).await
}

/// Resolve all shards at the same revision; a partial set is never a usable model.
fn selected_files<'a>(files: &'a [HubFile], name: &str) -> Result<Vec<&'a HubFile>, String> {
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

#[tauri::command]
pub async fn download_hugging_face_model(
    state: tauri::State<'_, AppState>,
    repo: String,
    revision: String,
    filename: String,
    token: Option<String>,
    projector: Option<String>,
) -> Result<Status, String> {
    let installer = &state.model_installer;
    let key = format!("model:hf:{repo}@{revision}:{filename}");
    let (_guard, receiver, _job_id) = installer.begin(&key, &filename)?;
    let result = async {
        installer.update(|status| status.phase = "resolving".into());
        let catalog = tokio::select! {
            result = repository(repo, Some(&revision), token.as_deref()) => result?,
            _ = async { let mut cancel = receiver.clone(); let _ = cancel.changed().await; } => return Err("Download cancelled.".into()),
        };
        let files = selected_files(&catalog.files, &filename)?;
        let weight_bytes = files.iter().try_fold(0u64, |n, f| n.checked_add(f.bytes).ok_or("Model size overflow."))?;
        let directory = state.data_dir.join("models").join(digest(&format!("{}/{}/{}", catalog.repo, catalog.revision, files[0].filename)));
        let previous = pending_download(&directory);
        let projector_name = projector_for_resume(&catalog.files, projector.as_deref(), previous.as_ref(), weight_bytes);
        let projector_file = select_projector(&catalog.files, &filename, projector_name.as_deref())?;
        let projector_in_weights = projector_file.is_some_and(|p| files.iter().any(|f| f.filename == p.filename));
        let total = if let Some(p) = projector_file.filter(|_| !projector_in_weights) {
            weight_bytes.checked_add(p.bytes).ok_or("Model size overflow.")?
        } else {
            weight_bytes
        };
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let root = std::fs::canonicalize(state.data_dir.join("models")).map_err(|e| e.to_string())?;
        let directory = std::fs::canonicalize(&directory).map_err(|e| e.to_string())?;
        if !directory.starts_with(&root) { return Err("Model directory resolves outside managed storage.".into()); }
        let mut already = 0u64;
        // The projector downloads into the same managed directory and folds
        // into the same progress total; it is never billed as a weight shard.
        let projector_pending = projector_file.filter(|_| !projector_in_weights);
        let mut pending: Vec<&HubFile> = files.to_vec();
        if let Some(p) = projector_pending {
            pending.push(p);
        }
        let needed = pending.iter().try_fold(0u64, |n, f| {
            let basename = f.filename.rsplit('/').next().unwrap();
            let path = directory.join(basename);
            if path.exists() {
                already = already.saturating_add(f.bytes);
                return Ok(n);
            }
            let have = std::fs::metadata(download::partial_path(&path)).map(|m| m.len()).unwrap_or(0).min(f.bytes);
            already = already.saturating_add(have);
            n.checked_add(f.bytes.saturating_sub(have)).ok_or("Model size overflow.")
        })?;
        let download_files: Vec<HubFile> = pending.iter().map(|f| (*f).clone()).collect();
        write_download_state(&directory, &DownloadState {
            repo: catalog.repo.clone(), revision: catalog.revision.clone(),
            filename: files[0].filename.clone(), bytes: total, sha256: files[0].sha256.clone(), received: already,
            projector: projector_file.map(|p| p.filename.clone()),
            files: download_files,
        })?;
        installer.update(|status| {
            status.path = Some(directory.join(files[0].filename.rsplit('/').next().unwrap()).to_string_lossy().into_owned());
            status.total = total;
            status.received = already;
            status.saved = already;
            status.asset_count = pending.len() as u32;
            status.phase = if already > 0 { "resuming".into() } else { "downloading".into() };
        });
        let space = download::storage(&directory, needed, 0)?;
        if space.available < space.required { return Err("Not enough disk space for this model and verification overhead.".into()); }
        let mut builder = reqwest::Client::builder().https_only(true).connect_timeout(Duration::from_secs(15)).timeout(Duration::from_secs(7200));
        if let Some(token) = token.as_deref().filter(|s| !s.is_empty()) {
            let mut headers = reqwest::header::HeaderMap::new();
            let mut value = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| "Invalid read token.")?;
            value.set_sensitive(true); headers.insert(reqwest::header::AUTHORIZATION, value); builder = builder.default_headers(headers);
        }
        let client = builder.build().map_err(|_| "Could not initialize downloader.")?;
        let mut finished = 0;
        let mut paths = Vec::new();
        for (index, file) in pending.iter().enumerate() {
            let basename = file.filename.rsplit('/').next().unwrap();
            let path = directory.join(basename);
            if path.exists() && !std::fs::canonicalize(&path).map_err(|e| e.to_string())?.starts_with(&directory) { return Err("Model file resolves outside managed storage.".into()); }
            let url = format!("https://huggingface.co/{}/resolve/{}/{}", catalog.repo, catalog.revision, file.filename);
            let asset = download::Asset { url: &url, bytes: file.bytes, sha256: &file.sha256 };
            let have = std::fs::metadata(download::partial_path(&path)).map(|m| m.len()).unwrap_or(0);
            let exists = path.exists();
            installer.update(|status| {
                status.asset = Some(basename.to_owned());
                status.asset_index = index as u32 + 1;
                status.asset_total = file.bytes;
                status.asset_received = if exists { 0 } else { have.min(file.bytes) };
                status.phase = if exists { "verifying".into() } else if have > 0 { "resuming".into() } else { "downloading".into() };
            });
            if exists {
                download::verify_file(&path, &asset, receiver.clone(), |n| {
                    installer.update(|status| {
                        status.asset_received = n;
                        status.received = finished + n;
                    });
                }).await?;
            } else if let Err(error) = download::fetch(&client, &asset, &path, receiver.clone(), |event| installer.progress(finished, event)).await {
                let saved = std::fs::metadata(download::partial_path(&path)).map(|m| m.len()).unwrap_or(0);
                write_download_state(&directory, &DownloadState {
                    repo: catalog.repo.clone(), revision: catalog.revision.clone(),
                    filename: files[0].filename.clone(), bytes: total, sha256: files[0].sha256.clone(),
                    received: finished + saved,
                    projector: projector_file.map(|p| p.filename.clone()),
                    files: pending.iter().map(|f| (*f).clone()).collect(),
                })?;
                return Err(error);
            }
            finished += file.bytes;
            paths.push(path.to_string_lossy().into_owned());
        }
        let projector_record = projector_file.map(|p| {
            let name = p.filename.rsplit('/').next().unwrap_or(&p.filename).to_owned();
            (name, directory.join(p.filename.rsplit('/').next().unwrap()).to_string_lossy().into_owned())
        });
        let mut weight_paths = paths.clone();
        if let Some((_, ref projector_path)) = projector_record {
            weight_paths.retain(|p| p != projector_path);
        }
        let weight_first = weight_paths.first().cloned().unwrap_or_else(|| paths[0].clone());
        let weight_bytes = total.saturating_sub(projector_file.map(|p| if projector_in_weights { 0 } else { p.bytes }).unwrap_or(0));
        let expected: Vec<HubFile> = pending
            .iter()
            .map(|f| HubFile {
                filename: f.filename.rsplit('/').next().unwrap_or(&f.filename).to_owned(),
                bytes: f.bytes,
                sha256: f.sha256.clone(),
            })
            .collect();
        let model = InstalledModel { id: weight_first.clone(), path: weight_first, filename: files[0].filename.clone(), bytes: weight_bytes, repo: Some(catalog.repo.clone()), revision: Some(catalog.revision.clone()), files: weight_paths, complete: true, received: weight_bytes,
            projector_filename: projector_record.as_ref().map(|(name, _)| name.clone()),
            projector_path: projector_record.as_ref().map(|(_, path)| path.clone()), external: false,
            expected, verified: true };
        crate::gguf::read_architecture(Path::new(&model.path))?.ok_or("Downloaded file is not a supported GGUF header.")?;
        if let Some(ref projector_path) = model.projector_path {
            // The projector is a vision encoder, not a language model; a GGUF
            // magic check catches truncated transfers without a full parse.
            let mut magic = [0u8; 4];
            use std::io::Read;
            std::fs::File::open(projector_path)
                .and_then(|mut f| f.read_exact(&mut magic))
                .map_err(|_| "The projector file is incomplete.")?;
            if magic != *b"GGUF" {
                return Err("The projector file is not a GGUF model.".into());
            }
        }
        if *receiver.borrow() { return Err("Download cancelled.".into()); }
        publish_manifest(&directory, &model)?;
        Ok::<_, String>(model.path)
    }.await;
    Ok(installer.finish(result))
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
    /// Every weight shard plus the selected projector (repo filename, bytes,
    /// sha256). Empty for state written by earlier builds.
    #[serde(default)]
    files: Vec<HubFile>,
}
fn projector_for_resume(
    files: &[HubFile],
    requested: Option<&str>,
    previous: Option<&DownloadState>,
    weight_bytes: u64,
) -> Option<String> {
    if let Some(name) = requested {
        return Some(name.to_owned());
    }
    let previous = previous?;
    if let Some(name) = &previous.projector {
        return Some(name.clone());
    }
    let projector_bytes = previous.bytes.checked_sub(weight_bytes)?;
    if projector_bytes == 0 {
        return None;
    }
    let mut matches = projector_candidates(files)
        .into_iter()
        .filter(|file| file.bytes == projector_bytes);
    let projector = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(projector.filename.clone())
}
/// Publish model.json atomically; interrupted downloads stay visible but
/// unusable. The pending marker is only cleared once model.json is durable.
fn publish_manifest(directory: &Path, model: &InstalledModel) -> Result<(), String> {
    use std::io::Write;
    let mut manifest = tempfile::NamedTempFile::new_in(directory).map_err(|e| e.to_string())?;
    manifest
        .write_all(&serde_json::to_vec_pretty(model).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    manifest.as_file().sync_all().map_err(|e| e.to_string())?;
    manifest
        .persist(directory.join("model.json"))
        .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(directory.join("download.json"));
    Ok(())
}

fn write_download_state(directory: &Path, state: &DownloadState) -> Result<(), String> {
    use std::io::Write;
    let body = serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?;
    let mut temp = tempfile::NamedTempFile::new_in(directory).map_err(|e| e.to_string())?;
    temp.write_all(&body).map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(directory.join("download.json"))
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn known_source(filename: &str) -> Option<String> {
    let (_, asset) = crate::model_catalog::model(Some(filename)).ok()?;
    asset
        .url
        .strip_prefix("https://huggingface.co/")?
        .split("/resolve/")
        .next()
        .map(str::to_owned)
}

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
        if path.is_dir() && depth > 0 {
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
    let managed = directory
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.len() == 64 && n.bytes().all(|b| b.is_ascii_hexdigit()));
    let hub_files: Vec<_> = files
        .iter()
        .map(|(p, bytes)| HubFile {
            filename: p.file_name().unwrap().to_string_lossy().into_owned(),
            bytes: *bytes,
            sha256: String::new(),
        })
        .collect();
    let mut local: Vec<InstalledModel> = Vec::new();
    for file in &hub_files {
        if file.filename.to_lowercase().contains("mmproj") {
            continue;
        }
        if file.filename.contains("-of-") && !file.filename.contains("-00001-of-") {
            continue;
        }
        let group = selected_files(&hub_files, &file.filename);
        let group_ok = group.is_ok();
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
        if local.iter().any(|m| m.path == path) {
            continue;
        }
        let bytes = group.iter().map(|f| f.bytes).sum();
        let manifest_row = source.as_ref().is_some_and(|s| s.path == path);
        // A managed manifest is only as good as the bytes it declares: pinned
        // expectations must match on size, and legacy manifests must still
        // point at files that exist.
        let (complete, verified) = if managed && manifest_row {
            let manifest = source.as_ref().unwrap();
            if !manifest.expected.is_empty() {
                let intact = manifest.expected.iter().all(|expected| {
                    std::fs::symlink_metadata(root.join(&expected.filename))
                        .is_ok_and(|m| {
                            m.is_file() && !m.file_type().is_symlink() && m.len() == expected.bytes
                        })
                });
                (intact && group_ok, intact)
            } else {
                let intact = manifest.files.iter().all(|p| Path::new(p).is_file())
                    && manifest
                        .projector_path
                        .as_ref()
                        .is_none_or(|p| Path::new(p).is_file());
                (intact && group_ok, false)
            }
        } else {
            (group_ok && (!managed || source.is_some()), false)
        };
        let (projector_filename, projector_path) = match &source {
            Some(manifest) if manifest_row => (
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
        local.push(InstalledModel {
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
            external: false,
            expected: if manifest_row {
                source.as_ref().map(|s| s.expected.clone()).unwrap_or_default()
            } else {
                Vec::new()
            },
            verified,
        });
    }
    // A pending download state is authoritative for this directory's matching
    // row; suppression never reaches rows from other directories, so two repos
    // with identical filenames both stay listed.
    if let Some(state) = pending_download(directory) {
        let basename = state.filename.rsplit('/').next().unwrap_or(&state.filename);
        let dest = root.join(basename);
        let part = download::partial_path(&dest);
        let received = if state.files.is_empty() {
            std::fs::metadata(&part)
                .map(|m| m.len())
                .unwrap_or(state.received)
        } else {
            state
                .files
                .iter()
                .map(|f| {
                    let path = root.join(f.filename.rsplit('/').next().unwrap_or(&f.filename));
                    match std::fs::metadata(&path).ok().filter(|m| m.is_file()) {
                        Some(m) => m.len(),
                        None => std::fs::metadata(download::partial_path(&path))
                            .map(|m| m.len())
                            .unwrap_or(0)
                            .min(f.bytes),
                    }
                })
                .sum()
        };
        let finals: Vec<String> = state
            .files
            .iter()
            .filter_map(|f| {
                let path = root.join(f.filename.rsplit('/').next().unwrap_or(&f.filename));
                path.is_file()
                    .then(|| path.to_string_lossy().into_owned())
            })
            .collect();
        if let Some(row) = local.iter_mut().find(|m| m.filename == basename) {
            row.repo = Some(state.repo);
            row.revision = Some(state.revision);
            row.projector_filename = state.projector.clone();
            row.projector_path = None;
            row.bytes = state.bytes;
            row.complete = false;
            row.received = received;
            if !state.files.is_empty() {
                row.files = finals;
            }
        } else {
            let path = if dest.exists() { dest } else { part.clone() };
            let path_string = path.to_string_lossy().into_owned();
            local.push(InstalledModel {
                id: path_string.clone(),
                path: path_string.clone(),
                filename: basename.to_owned(),
                bytes: state.bytes,
                repo: Some(state.repo),
                revision: Some(state.revision),
                files: if finals.is_empty() {
                    vec![path_string]
                } else {
                    finals
                },
                complete: false,
                received,
                projector_filename: state.projector.clone(),
                projector_path: None,
                external: false,
                expected: Vec::new(),
                verified: false,
            });
        }
    }
    result.extend(local);
    Ok(())
}
fn pending_download(directory: &Path) -> Option<DownloadState> {
    serde_json::from_slice(&std::fs::read(directory.join("download.json")).ok()?).ok()
}
fn inventory(state: &AppState) -> Result<Vec<InstalledModel>, String> {
    let mut result = Vec::new();
    scan(&state.data_dir.join("models"), 1, &mut result)?;
    if let Some(root) = Path::new(env!("CARGO_MANIFEST_DIR")).parent() {
        let mut legacy = Vec::new();
        scan(&root.join(".local/models"), 0, &mut legacy)?;
        for model in legacy {
            // The development installer made a second MiniCPM hard link. Treat
            // matching catalog filenames/sizes as one model, listing both locations.
            if let Some(existing) = result.iter_mut().find(|m| {
                m.filename == model.filename
                    && m.bytes == model.bytes
                    && crate::model_catalog::model(Some(&m.filename))
                        .is_ok_and(|(_, a)| a.bytes == m.bytes)
            }) {
                for path in model.files {
                    if !existing.files.contains(&path) {
                        existing.files.push(path);
                    }
                }
            } else {
                result.push(model);
            }
        }
    }
    if let Some(model) = k2_horizon_model() {
        if !result.iter().any(|item| paths_match(&item.path, &model.path)) {
            result.push(model);
        }
    }
    let selected = state.database()?.preferences()?.model_path;
    if let Ok(path) = std::fs::canonicalize(&selected) {
        let path_string = path.to_string_lossy().into_owned();
        if !result.iter().any(|m| m.files.contains(&path_string))
            && path.is_file()
            && path
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("gguf"))
        {
            result.push(InstalledModel {
                id: path_string.clone(),
                path: path_string.clone(),
                filename: path.file_name().unwrap().to_string_lossy().into_owned(),
                bytes: path.metadata().map_err(|e| e.to_string())?.len(),
                repo: known_source(path.file_name().unwrap().to_str().unwrap_or_default()),
                revision: None,
                files: vec![path_string],
                complete: true,
                received: path.metadata().map_err(|e| e.to_string())?.len(),
                projector_filename: None,
                projector_path: None,
                external: true,
                expected: Vec::new(),
                verified: false,
            });
        }
    }
    result.sort_by(|a, b| a.filename.cmp(&b.filename));
    Ok(result)
}

fn k2_horizon_model() -> Option<InstalledModel> {
    k2_horizon_model_at(&PathBuf::from(std::env::var_os("APPDATA")?))
}

fn k2_horizon_model_at(app_data: &Path) -> Option<InstalledModel> {
    let path = app_data.join("studio.k2horizon.app/models/k2-horizon-3.7b/K2-Horizon-4B-Q4_K_M.gguf");
    let path = std::fs::canonicalize(path).ok()?;
    let bytes = path.metadata().ok()?.len();
    let path = path.to_string_lossy().into_owned();
    Some(InstalledModel {
        id: path.clone(), path: path.clone(), filename: "K2-Horizon-4B-Q4_K_M.gguf".into(),
        bytes, repo: None, revision: None, files: vec![path], complete: true,
        received: bytes, projector_filename: None, projector_path: None, external: true,
        expected: Vec::new(), verified: false,
    })
}

fn k2_horizon_runtime(model: &InstalledModel) -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent()?.parent()?;
    k2_horizon_runtime_at(model, root)
}

fn k2_horizon_runtime_at(model: &InstalledModel, root: &Path) -> Option<PathBuf> {
    if !model.external || model.filename != "K2-Horizon-4B-Q4_K_M.gguf" {
        return None;
    }
    let runtime = root.join("k2 horizon agent/local-k2/runtime/windows/cuda/llama-server.exe");
    runtime.is_file().then_some(runtime)
}
#[tauri::command]
pub fn list_installed_models(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<InstalledModel>, String> {
    inventory(&state)
}

#[tauri::command]
pub async fn delete_installed_model(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Stop generation or loading before deleting a model.")?;
    let _model_download = state
        .model_installer
        .run
        .try_lock()
        .map_err(|_| "Pause the model download before deleting a model.")?;
    let model = inventory(&state)?
        .into_iter()
        .find(|m| m.id == id)
        .ok_or("Model is no longer in the library.")?;
    if model.external {
        return Err("This model belongs to another project. Remove it from that project instead.".into());
    }
    let mut runtime = state.runtime.lock().await;
    if runtime
        .inspect()
        .model_path
        .as_ref()
        .and_then(|p| std::fs::canonicalize(p).ok())
        .is_some_and(|p| model.files.iter().any(|file| Path::new(file) == p))
    {
        runtime.stop().await?;
    }
    let mut preferences = state.database()?.preferences()?;
    let selected = std::fs::canonicalize(&preferences.model_path)
        .ok()
        .is_some_and(|p| model.files.iter().any(|file| Path::new(file) == p));
    let projector_deleted = std::fs::canonicalize(&preferences.projector_path)
        .ok()
        .is_some_and(|p| model.files.iter().any(|file| Path::new(file) == p));
    for path in &model.files {
        std::fs::remove_file(path).map_err(|e| format!("Could not delete model: {e}"))?;
    }
    cleanup_model_storage(&model, &state.data_dir)?;
    let store = state.database()?;
    if selected {
        preferences.model_path.clear();
        preferences.projector_path.clear();
    } else if projector_deleted {
        preferences.projector_path.clear();
    }
    if selected || projector_deleted {
        store.save_preferences(&preferences)?;
    }
    Ok(())
}

fn profile_path_for(data_dir: &Path, path: &str) -> PathBuf {
    data_dir
        .join("model-profiles")
        .join(format!("{}.json", digest(&path.to_lowercase())))
}

fn cleanup_model_storage(model: &InstalledModel, data_dir: &Path) -> Result<(), String> {
    let models_root = std::fs::canonicalize(data_dir.join("models")).ok();
    let mut parents = Vec::new();
    for path in &model.files {
        if let Some(parent) = Path::new(path).parent() {
            let parent = parent.to_path_buf();
            if !parents.contains(&parent) {
                parents.push(parent);
            }
        }
    }
    for parent in parents {
        let Ok(canonical) = std::fs::canonicalize(&parent) else {
            continue;
        };
        let managed = models_root.as_ref().is_some_and(|root| {
            canonical.starts_with(root)
                && canonical != *root
                && canonical
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit())
                    })
        });
        if !managed {
            continue;
        }
        let _ = std::fs::remove_file(canonical.join("model.json"));
        let _ = std::fs::remove_file(canonical.join("download.json"));
        if let Ok(entries) = std::fs::read_dir(&canonical) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().ends_with(".part") {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        if std::fs::read_dir(&canonical)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(false)
        {
            let _ = std::fs::remove_dir(&canonical);
        }
    }
    let mut identities = vec![model.path.clone()];
    identities.extend(model.files.iter().cloned());
    identities.sort();
    identities.dedup();
    for path in identities {
        let _ = std::fs::remove_file(profile_path_for(data_dir, &path));
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct Profile {
    runtime_path: String,
    config: crate::runtime_config::RuntimeConfig,
}
fn profile_path(state: &AppState, path: &str) -> PathBuf {
    profile_path_for(&state.data_dir, path)
}

fn paths_match(left: &str, right: &str) -> bool {
    if left.eq_ignore_ascii_case(right) {
        return true;
    }
    std::fs::canonicalize(left)
        .ok()
        .zip(std::fs::canonicalize(right).ok())
        .is_some_and(|(a, b)| a == b)
}

fn already_using_model(saved_path: &str, model: &InstalledModel) -> bool {
    paths_match(saved_path, &model.path)
        || model.files.iter().any(|path| paths_match(saved_path, path))
}

fn write_profile(
    state: &AppState,
    model_path: &str,
    runtime_path: &str,
    config: &crate::runtime_config::RuntimeConfig,
) -> Result<(), String> {
    if model_path.is_empty() {
        return Ok(());
    }
    let profile = Profile {
        runtime_path: runtime_path.to_string(),
        config: config.clone(),
    };
    let bytes = serde_json::to_vec(&profile).map_err(|e| e.to_string())?;
    let mut keys = vec![model_path.to_string()];
    if let Ok(canonical) = std::fs::canonicalize(model_path) {
        keys.push(canonical.to_string_lossy().into_owned());
    }
    keys.sort();
    keys.dedup();
    for key in keys {
        let path = profile_path(state, &key);
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(path, &bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Persist the current model's runtime profile so a later Use click restores
/// the saved context instead of the first-load 8,192-token default.
pub fn save_active_profile(
    state: &AppState,
    config: &crate::runtime_config::RuntimeConfig,
) -> Result<(), String> {
    let preferences = state.database()?.preferences()?;
    write_profile(
        state,
        &preferences.model_path,
        &preferences.runtime_path,
        config,
    )
}

fn initial_config(
    model: Option<&crate::gguf::ModelMetadata>,
) -> crate::runtime_config::RuntimeConfig {
    // F16 works for head dimensions that cannot use block-quantized caches.
    // Flash Attention is opt-in until the selected runtime proves support.
    crate::runtime_config::RuntimeConfig {
        context_length: model
            .and_then(|m| m.context_length)
            .unwrap_or(8192)
            .clamp(128, 8192),
        cache_type_k: crate::runtime_config::CacheType::F16,
        cache_type_v: crate::runtime_config::CacheType::F16,
        flash_attention: false,
        cpu_threads: (std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(12)
            / 2)
        .clamp(1, 256) as u32,
        ..Default::default()
    }
}
#[tauri::command]
pub async fn use_installed_model(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<crate::runtime::RuntimeStatus, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Stop generation or loading before switching models.")?;
    // Loading an already complete file does not mutate installation storage.
    // Keep generation/loading serialized, but allow downloads to continue.
    let models = inventory(&state)?;
    let model = models
        .iter()
        .find(|m| m.id == id && m.complete)
        .ok_or("Model is missing or has incomplete shards.")?;
    let (next_preferences, next_config) = {
        let store = state.database()?;
        let mut preferences = store.preferences()?;
        let current_config = store.runtime_config()?;
        if !preferences.model_path.is_empty() {
            write_profile(
                &state,
                &preferences.model_path,
                &preferences.runtime_path,
                &current_config,
            )?;
        }
        let already_selected = already_using_model(&preferences.model_path, model);
        let config = if already_selected {
            // Re-using the active model must not reset a just-saved full
            // context back to the first-load 8,192-token default.
            current_config
        } else if let Ok(bytes) = std::fs::read(profile_path(&state, &model.path)) {
            let profile: Profile =
                serde_json::from_slice(&bytes).map_err(|_| "Saved model profile is invalid.")?;
            preferences.runtime_path = profile.runtime_path;
            profile.config
        } else {
            preferences.runtime_path.clear();
            let mut config = initial_config(
                crate::gguf::model_metadata(Path::new(&model.path))
                    .ok()
                    .as_ref(),
            );
            if model.external && model.filename == "K2-Horizon-4B-Q4_K_M.gguf" {
                config.context_length = 16_384;
            }
            config
        };
        if model.external && model.filename == "K2-Horizon-4B-Q4_K_M.gguf" {
            preferences.runtime_path = k2_horizon_runtime(model)
                .ok_or("K2 Horizon's custom CUDA runtime is missing from the neighboring k2 horizon agent project.")?
                .to_string_lossy().into_owned();
        }
        preferences.model_path = model.path.clone();
        // Keep the projector in lockstep with the weights: a stale mmproj from
        // a previous model makes llama-server crash on an embedding-size
        // mismatch, and a missing projector silently disables vision.
        preferences.projector_path = model.projector_path.clone().unwrap_or_default();
        (preferences, config)
    };
    crate::commands::switch_and_load(&state, next_preferences, next_config).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovers_k2_horizon_without_taking_ownership_of_its_file() {
        let app_data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let path = app_data.path().join("studio.k2horizon.app/models/k2-horizon-3.7b/K2-Horizon-4B-Q4_K_M.gguf");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"GGUF").unwrap();
        let model = k2_horizon_model_at(app_data.path()).unwrap();
        assert!(model.external);
        assert_eq!(model.bytes, 4);
        assert!(k2_horizon_runtime_at(&model, project.path()).is_none());
        let runtime = project.path().join("k2 horizon agent/local-k2/runtime/windows/cuda/llama-server.exe");
        std::fs::create_dir_all(runtime.parent().unwrap()).unwrap();
        std::fs::write(&runtime, b"runtime").unwrap();
        assert_eq!(k2_horizon_runtime_at(&model, project.path()), Some(runtime));
    }
    #[test]
    fn saving_preferences_updates_the_active_models_runtime_profile() {
        let directory = tempfile::tempdir().unwrap();
        let state = crate::web_search::tests::test_state(directory.path());
        let model = directory.path().join("model.gguf");
        std::fs::write(&model, b"GGUF").unwrap();
        let first = directory.path().join("runtime-a.exe");
        let second = directory.path().join("runtime-b.exe");
        std::fs::write(&first, b"runtime").unwrap();
        std::fs::write(&second, b"runtime").unwrap();
        let preferences = crate::store::Preferences {
            model_path: model.to_string_lossy().into_owned(),
            runtime_path: first.to_string_lossy().into_owned(),
            ..Default::default()
        };
        crate::commands::persist_preferences(&state, preferences.clone()).unwrap();
        crate::commands::persist_preferences(&state, crate::store::Preferences {
            runtime_path: second.to_string_lossy().into_owned(),
            ..preferences
        }).unwrap();
        let bytes = std::fs::read(profile_path(&state, &model.to_string_lossy())).unwrap();
        let profile: Profile = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(profile.runtime_path, second.to_string_lossy());
        let store = state.database().unwrap();
        assert_eq!(store.preferences().unwrap().runtime_path, second.to_string_lossy().into_owned());
    }

    #[test]
    fn new_models_do_not_inherit_unsupported_quantized_cache_settings() {
        let config = initial_config(Some(&crate::gguf::ModelMetadata {
            context_length: Some(2048),
            key_length: Some(8),
            ..Default::default()
        }));
        assert_eq!(config.context_length, 2048);
        assert_eq!(config.cache_type_k, crate::runtime_config::CacheType::F16);
        assert!(!config.flash_attention);
    }

    #[test]
    fn reusing_the_active_model_is_detected_without_resetting_context() {
        let model = InstalledModel {
            id: "model.gguf".into(),
            filename: "model.gguf".into(),
            path: r"C:\models\model.gguf".into(),
            bytes: 1,
            repo: None,
            revision: None,
            files: vec![r"C:\models\model.gguf".into()],
            complete: true,
            received: 1,
            projector_filename: None,
            projector_path: None,
            external: false,
            expected: Vec::new(),
            verified: false,
        };
        assert!(already_using_model(r"C:\models\model.gguf", &model));
        assert!(already_using_model(r"c:\models\model.gguf", &model));
        assert!(!already_using_model(r"C:\models\other.gguf", &model));
    }
    #[test]
    fn rejects_traversal_and_url_injection() {
        for repo in [
            "../model",
            "owner/../../etc",
            "https://evil.com",
            "owner/model?x=1",
            "owner\\model",
        ] {
            assert!(validate_repo(repo).is_err());
        }
        assert!(validate_repo("bartowski/Qwen3-GGUF").is_ok());
        for name in [
            "../model.gguf",
            "C:/model.gguf",
            "x\\a.gguf",
            "x/a.gguf:stream",
        ] {
            assert!(!valid_filename(name));
        }
        assert!(valid_filename("Q4_K_M/model.gguf"));
    }
    #[test]
    fn split_models_require_every_shard_and_preserve_order() {
        let files = vec![HubFile {
            filename: "test-00001-of-00002.gguf".into(),
            bytes: 10,
            sha256: String::new(),
        }];
        assert!(selected_files(&files, &files[0].filename).is_err());
        let mut files = files;
        files.push(HubFile {
            filename: "test-00002-of-00002.gguf".into(),
            bytes: 20,
            sha256: String::new(),
        });
        assert_eq!(
            selected_files(&files, &files[1].filename).unwrap()[0].filename,
            files[0].filename
        );
    }
    #[test]
    fn scans_legacy_files_and_incomplete_shards_without_network() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(crate::model_catalog::MODEL_FILENAME),
            b"GGUF",
        )
        .unwrap();
        std::fs::write(dir.path().join("custom-00001-of-00002.gguf"), b"GGUF").unwrap();
        std::fs::write(dir.path().join("unfinished.part"), b"GGUF").unwrap();
        let mut result = Vec::new();
        scan(dir.path(), 1, &mut result).unwrap();
        assert_eq!(result.len(), 2);
        let minicpm = result
            .iter()
            .find(|m| m.filename == crate::model_catalog::MODEL_FILENAME)
            .unwrap();
        assert_eq!(
            minicpm.repo.as_deref(),
            Some("prithivMLmods/MiniCPM5-2B-GGUF")
        );
        assert!(minicpm.complete);
        assert!(!minicpm.id.is_empty());
        assert!(result.iter().any(|m| !m.complete));
    }

    #[test]
    fn scan_keeps_interrupted_download_progress() {
        let dir = tempfile::tempdir().unwrap();
        let model_dir = dir.path().join("b".repeat(64));
        std::fs::create_dir(&model_dir).unwrap();
        std::fs::write(model_dir.join("model.gguf.part"), b"fixt").unwrap();
        write_download_state(
            &model_dir,
            &DownloadState {
                repo: "unsloth/gemma-4-E4B".into(),
                revision: "a".repeat(40),
                filename: "model.gguf".into(),
                bytes: 7,
                sha256: "b".repeat(64),
                received: 4,
                projector: Some("mmproj-model-f16.gguf".into()),
                files: Vec::new(),
            },
        )
        .unwrap();
        let mut result = Vec::new();
        scan(dir.path(), 1, &mut result).unwrap();
        assert_eq!(result.len(), 1);
        assert!(!result[0].complete);
        assert_eq!(result[0].received, 4);
        assert_eq!(result[0].bytes, 7);
        assert_eq!(result[0].repo.as_deref(), Some("unsloth/gemma-4-E4B"));
        assert_eq!(
            result[0].projector_filename.as_deref(),
            Some("mmproj-model-f16.gguf")
        );
    }

    #[test]
    fn catalog_models_are_hugging_face_sources() {
        assert_eq!(
            known_source(crate::model_catalog::MODEL_FILENAME).as_deref(),
            Some("prithivMLmods/MiniCPM5-2B-GGUF")
        );
        assert_eq!(known_source("custom.gguf"), None);
    }

    #[test]
    fn popular_list_omits_blank_search_param() {
        let popular = hugging_face_search_url("  ");
        let query = popular.query().unwrap();
        assert!(!query.contains("search="), "{query}");
        assert!(query.contains("filter=gguf"));
        let named = hugging_face_search_url(" qwen ");
        assert!(named.query().unwrap().contains("search=qwen"));
    }

    #[test]
    fn search_hits_include_download_counts_and_skip_invalid_ids() {
        let data = serde_json::json!([
            {"id": "bartowski/Qwen2.5-GGUF", "downloads": 100, "likes": 5},
            {"id": "../evil", "downloads": 1},
            {"id": "owner/model"}
        ]);
        let hits = parse_search_hits(&data).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].downloads, Some(100));
        assert_eq!(hits[0].likes, Some(5));
        assert_eq!(hits[1].id, "owner/model");
        assert_eq!(hits[1].downloads, None);
    }

    #[test]
    fn delete_cleanup_removes_manifest_empty_dir_and_profile() {
        let data = tempfile::tempdir().unwrap();
        let hash = "a".repeat(64);
        let dir = data.path().join("models").join(&hash);
        std::fs::create_dir_all(&dir).unwrap();
        let gguf = dir.join("model.gguf");
        std::fs::write(&gguf, b"GGUF").unwrap();
        std::fs::write(dir.join("model.json"), b"{}").unwrap();
        let profiles = data.path().join("model-profiles");
        std::fs::create_dir_all(&profiles).unwrap();
        let profile = profile_path_for(data.path(), &gguf.to_string_lossy());
        std::fs::write(&profile, b"{}").unwrap();
        std::fs::remove_file(&gguf).unwrap();
        let model = InstalledModel {
            id: gguf.to_string_lossy().into_owned(),
            path: gguf.to_string_lossy().into_owned(),
            filename: "model.gguf".into(),
            bytes: 4,
            repo: None,
            revision: None,
            files: vec![gguf.to_string_lossy().into_owned()],
            complete: true,
            received: 4,
            projector_filename: None,
            projector_path: None,
            external: false,
            expected: Vec::new(),
            verified: false,
        };
        cleanup_model_storage(&model, data.path()).unwrap();
        assert!(!dir.exists());
        assert!(!profile.exists());
    }

    #[test]
    fn projector_selection_is_optional_and_validates_explicit_files() {
        let files = vec![
            HubFile {
                filename: "BAAI_AREX-Turbo-Q4_K_M.gguf".into(),
                bytes: 10,
                sha256: "c".repeat(64),
            },
            HubFile {
                filename: "BAAI_AREX-Turbo-Q8_0.gguf".into(),
                bytes: 20,
                sha256: "d".repeat(64),
            },
            HubFile {
                filename: "mmproj-BAAI_AREX-Turbo-Q4_K_M.gguf".into(),
                bytes: 5,
                sha256: "a".repeat(64),
            },
            HubFile {
                filename: "mmproj-BAAI_AREX-Turbo-Q8_0.gguf".into(),
                bytes: 6,
                sha256: "b".repeat(64),
            },
        ];
        assert!(
            select_projector(&files, "BAAI_AREX-Turbo-Q4_K_M.gguf", None)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            select_projector(
                &files,
                "BAAI_AREX-Turbo-Q4_K_M.gguf",
                Some("MMPROJ-baai_arex-turbo-q8_0.gguf")
            )
            .unwrap()
            .unwrap()
            .filename,
            "mmproj-BAAI_AREX-Turbo-Q8_0.gguf"
        );
        assert!(select_projector(
            &files,
            "BAAI_AREX-Turbo-Q4_K_M.gguf",
            Some("mmproj-missing.gguf")
        )
        .is_err());
        let ambiguous = vec![
            HubFile {
                filename: "model-Q4_K_M.gguf".into(),
                bytes: 10,
                sha256: "c".repeat(64),
            },
            HubFile {
                filename: "mmproj-a.gguf".into(),
                bytes: 5,
                sha256: "a".repeat(64),
            },
            HubFile {
                filename: "mmproj-b.gguf".into(),
                bytes: 6,
                sha256: "b".repeat(64),
            },
        ];
        assert!(select_projector(&ambiguous, "model-Q4_K_M.gguf", None)
            .unwrap()
            .is_none());
        let solo = vec![
            HubFile {
                filename: "model.gguf".into(),
                bytes: 10,
                sha256: "c".repeat(64),
            },
            HubFile {
                filename: "mmproj-model-f16.gguf".into(),
                bytes: 5,
                sha256: "a".repeat(64),
            },
        ];
        assert!(select_projector(&solo, "model.gguf", None)
            .unwrap()
            .is_none());
        let plain = vec![HubFile {
            filename: "model.gguf".into(),
            bytes: 10,
            sha256: "c".repeat(64),
        }];
        assert!(select_projector(&plain, "model.gguf", None)
            .unwrap()
            .is_none());
        assert!(select_projector(&plain, "model.gguf", Some("mmproj.gguf")).is_err());
    }

    #[test]
    fn legacy_download_state_recovers_projector_from_saved_total() {
        let files = vec![
            HubFile {
                filename: "model.gguf".into(),
                bytes: 100,
                sha256: "a".repeat(64),
            },
            HubFile {
                filename: "mmproj-BF16.gguf".into(),
                bytes: 30,
                sha256: "b".repeat(64),
            },
            HubFile {
                filename: "mmproj-Q8_0.gguf".into(),
                bytes: 20,
                sha256: "c".repeat(64),
            },
        ];
        let legacy = DownloadState {
            repo: "owner/repo".into(),
            revision: "rev".into(),
            filename: "model.gguf".into(),
            bytes: 120,
            sha256: "a".repeat(64),
            received: 40,
            projector: None,
            files: Vec::new(),
        };
        assert_eq!(
            projector_for_resume(&files, None, Some(&legacy), 100).as_deref(),
            Some("mmproj-Q8_0.gguf")
        );
        assert_eq!(
            projector_for_resume(&files, Some("mmproj-BF16.gguf"), Some(&legacy), 100).as_deref(),
            Some("mmproj-BF16.gguf")
        );
        let text_only = DownloadState {
            bytes: 100,
            ..legacy
        };
        assert_eq!(
            projector_for_resume(&files, None, Some(&text_only), 100),
            None
        );
    }

    #[test]
    fn download_state_keeps_the_projector_choice_for_resume() {
        let state = DownloadState {
            repo: "owner/repo-GGUF".into(),
            revision: "main".into(),
            filename: "model-Q4_K_M.gguf".into(),
            bytes: 10,
            sha256: "a".repeat(64),
            received: 4,
            projector: Some("mmproj-model-f16.gguf".into()),
            files: Vec::new(),
        };
        let parsed: DownloadState =
            serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert_eq!(parsed.projector.as_deref(), Some("mmproj-model-f16.gguf"));
        let legacy = format!(
            r#"{{"repo":"owner/repo-GGUF","revision":"main","filename":"model-Q4_K_M.gguf","bytes":10,"sha256":"{}","received":4}}"#,
            "a".repeat(64)
        );
        let legacy: DownloadState = serde_json::from_str(&legacy).unwrap();
        assert_eq!(legacy.projector, None);
    }

    fn hub_file(filename: &str, bytes: u64) -> HubFile {
        HubFile {
            filename: filename.into(),
            bytes,
            sha256: "a".repeat(64),
        }
    }

    #[test]
    fn an_interrupted_second_shard_reports_saved_progress() {
        let dir = tempfile::tempdir().unwrap();
        let model_dir = dir.path().join("a".repeat(64));
        std::fs::create_dir(&model_dir).unwrap();
        std::fs::write(model_dir.join("model-00001-of-00002.gguf"), b"done!").unwrap();
        std::fs::write(model_dir.join("model-00002-of-00002.gguf.part"), b"hal").unwrap();
        write_download_state(
            &model_dir,
            &DownloadState {
                repo: "owner/repo".into(),
                revision: "r".repeat(40),
                filename: "model-00001-of-00002.gguf".into(),
                bytes: 15,
                sha256: "c".repeat(64),
                received: 8,
                projector: Some("mmproj-model-f16.gguf".into()),
                files: vec![
                    hub_file("model-00001-of-00002.gguf", 5),
                    hub_file("model-00002-of-00002.gguf", 10),
                ],
            },
        )
        .unwrap();
        let mut result = Vec::new();
        scan(dir.path(), 1, &mut result).unwrap();
        assert_eq!(result.len(), 1);
        let row = &result[0];
        assert!(!row.complete);
        assert_eq!(row.received, 8);
        assert_eq!(row.bytes, 15);
        assert_eq!(row.repo.as_deref(), Some("owner/repo"));
        assert_eq!(
            row.projector_filename.as_deref(),
            Some("mmproj-model-f16.gguf")
        );
    }

    #[test]
    fn a_pending_projector_keeps_the_download_incomplete() {
        let dir = tempfile::tempdir().unwrap();
        let model_dir = dir.path().join("b".repeat(64));
        std::fs::create_dir(&model_dir).unwrap();
        std::fs::write(model_dir.join("model.gguf"), b"weights").unwrap();
        std::fs::write(model_dir.join("mmproj-model-f16.gguf.part"), b"mm").unwrap();
        write_download_state(
            &model_dir,
            &DownloadState {
                repo: "owner/repo".into(),
                revision: "r".repeat(40),
                filename: "model.gguf".into(),
                bytes: 12,
                sha256: "c".repeat(64),
                received: 9,
                projector: Some("mmproj-model-f16.gguf".into()),
                files: vec![hub_file("model.gguf", 7), hub_file("mmproj-model-f16.gguf", 5)],
            },
        )
        .unwrap();
        let mut result = Vec::new();
        scan(dir.path(), 1, &mut result).unwrap();
        assert_eq!(result.len(), 1);
        assert!(!result[0].complete);
        assert_eq!(result[0].received, 9);
        assert_eq!(
            result[0].projector_filename.as_deref(),
            Some("mmproj-model-f16.gguf")
        );
    }

    #[test]
    fn identical_filenames_in_different_repos_stay_separate_rows() {
        let dir = tempfile::tempdir().unwrap();
        for (name, repo) in [("c".repeat(64), "owner/one"), ("d".repeat(64), "owner/two")] {
            let model_dir = dir.path().join(name);
            std::fs::create_dir(&model_dir).unwrap();
            std::fs::write(model_dir.join("model.gguf"), b"same!").unwrap();
            write_download_state(
                &model_dir,
                &DownloadState {
                    repo: repo.into(),
                    revision: "r".repeat(40),
                    filename: "model.gguf".into(),
                    bytes: 9,
                    sha256: "c".repeat(64),
                    received: 5,
                    projector: None,
                    files: vec![hub_file("model.gguf", 9)],
                },
            )
            .unwrap();
        }
        let mut result = Vec::new();
        scan(dir.path(), 1, &mut result).unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.iter().any(|m| m.repo.as_deref() == Some("owner/one")));
        assert!(result.iter().any(|m| m.repo.as_deref() == Some("owner/two")));
        assert!(result.iter().all(|m| !m.complete));
    }

    fn manifest(directory: &Path, expected: Vec<HubFile>, projector: Option<String>) -> String {
        let root = std::fs::canonicalize(directory).unwrap();
        let weights = root.join("model.gguf").to_string_lossy().into_owned();
        serde_json::to_string(&InstalledModel {
            id: weights.clone(),
            path: weights.clone(),
            filename: "model.gguf".into(),
            bytes: 7,
            repo: Some("owner/repo".into()),
            revision: Some("r".repeat(40)),
            files: vec![weights],
            complete: true,
            received: 7,
            projector_filename: projector.clone(),
            projector_path: projector,
            external: false,
            expected,
            verified: true,
        })
        .unwrap()
    }

    #[test]
    fn expected_sizes_decide_whether_a_managed_model_is_complete() {
        let dir = tempfile::tempdir().unwrap();
        let model_dir = dir.path().join("e".repeat(64));
        std::fs::create_dir(&model_dir).unwrap();
        std::fs::write(model_dir.join("model.gguf"), b"weights").unwrap();
        // Truncated file: the manifest's pinned size does not match.
        let expected = vec![hub_file("model.gguf", 99)];
        std::fs::write(model_dir.join("model.json"), manifest(&model_dir, expected, None))
            .unwrap();
        let mut result = Vec::new();
        scan(&model_dir, 0, &mut result).unwrap();
        assert_eq!(result.len(), 1);
        assert!(!result[0].complete);
        assert!(!result[0].verified);
        // The right size makes the same manifest complete and verified.
        let expected = vec![hub_file("model.gguf", 7)];
        std::fs::write(model_dir.join("model.json"), manifest(&model_dir, expected, None))
            .unwrap();
        let mut result = Vec::new();
        scan(&model_dir, 0, &mut result).unwrap();
        assert!(result[0].complete);
        assert!(result[0].verified);
    }

    #[test]
    fn a_missing_declared_projector_breaks_legacy_completeness() {
        let dir = tempfile::tempdir().unwrap();
        let model_dir = dir.path().join("f".repeat(64));
        std::fs::create_dir(&model_dir).unwrap();
        std::fs::write(model_dir.join("model.gguf"), b"weights").unwrap();
        let missing = model_dir.join("mmproj-model-f16.gguf").to_string_lossy().into_owned();
        std::fs::write(
            model_dir.join("model.json"),
            manifest(&model_dir, Vec::new(), Some(missing)),
        )
        .unwrap();
        let mut result = Vec::new();
        scan(&model_dir, 0, &mut result).unwrap();
        assert_eq!(result.len(), 1);
        assert!(!result[0].complete);
        assert!(!result[0].verified);
        // A legacy manifest with every declared file present stays complete.
        std::fs::write(
            model_dir.join("model.json"),
            manifest(&model_dir, Vec::new(), None),
        )
        .unwrap();
        let mut result = Vec::new();
        scan(&model_dir, 0, &mut result).unwrap();
        assert!(result[0].complete);
        assert!(!result[0].verified);
    }

    #[test]
    fn a_failed_manifest_publish_keeps_the_download_state() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("model.json")).unwrap();
        std::fs::write(dir.path().join("download.json"), b"{}").unwrap();
        let model = InstalledModel {
            id: "x".into(),
            path: dir.path().join("model.gguf").to_string_lossy().into_owned(),
            filename: "model.gguf".into(),
            bytes: 7,
            repo: None,
            revision: None,
            files: Vec::new(),
            complete: true,
            received: 7,
            projector_filename: None,
            projector_path: None,
            external: false,
            expected: Vec::new(),
            verified: true,
        };
        assert!(publish_manifest(dir.path(), &model).is_err());
        assert!(dir.path().join("download.json").exists());
        std::fs::remove_dir(dir.path().join("model.json")).unwrap();
        publish_manifest(dir.path(), &model).unwrap();
        assert!(dir.path().join("model.json").is_file());
        assert!(!dir.path().join("download.json").exists());
    }
}
