//! Developer acceptance helper. Uses LocalLM's production verified downloader/extractor.
//! No application settings, default runtime, or existing model files are changed.
use std::path::PathBuf;
use tauri_app_lib::{download, runtime_archive};

const TAG: &str = "prism-b10754-2459f68";
const MODEL_NAME: &str = "Bonsai-27B-Q1_0.gguf";
const MODEL: download::Asset<'static> = download::Asset {
    url: "https://huggingface.co/prism-ml/Bonsai-27B-gguf/resolve/f10afb355f104535e3e3e98cf7ab7795c72bd292/Bonsai-27B-Q1_0.gguf",
    bytes: 3_803_452_480,
    sha256: "17ef842e47450caeb8eaa3ebfbbab5d2f2278b62b79be107985fb69a2f819aa0",
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), String> {
    let mode = std::env::args().nth(1).ok_or("Choose runtime or model.")?;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().ok_or("Project root missing.")?.join(".local/experimental/bonsai-27b");
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let client = download::client()?;
    let (sender, cancel) = tokio::sync::watch::channel(false);
    if mode == "model" || mode == "model-cancel" {
        let directory = root.join("models");
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let destination = directory.join(MODEL_NAME);
        if mode == "model-cancel" && destination.exists() {
            return Err("Cancellation acceptance requires an absent final model file.".into());
        }
        let mut last = 0;
        let progress = |n| {
            if mode == "model-cancel" && n >= 32 * 1024 * 1024 {
                let _ = sender.send(true);
            }
            if n >= last + 128 * 1024 * 1024 || n == MODEL.bytes {
                println!("model: {n}/{} bytes", MODEL.bytes);
                last = n;
            }
        };
        if destination.exists() {
            download::verify_file(&destination, &MODEL, cancel, progress).await?;
        } else {
            download::fetch(&client, &MODEL, &destination, cancel, progress).await?;
        }
        println!("Verified model: {} SHA-256 {}", destination.display(), MODEL.sha256);
        return Ok(());
    }
    if mode != "runtime" { return Err("Choose runtime, model or model-cancel.".into()); }
    let archives = [
        ("llama-prism-b10754-2459f68-bin-win-cuda-12.4-x64.zip", 312_971_133,
         "200842b4c689fe092a8a1684ef1e185d8d025a0bfae6908737607a0f29560b96"),
        ("cudart-llama-bin-win-cuda-12.4-x64.zip", 391_443_627,
         "8c79a9b226de4b3cacfd1f83d24f962d0773be79f1e7b75c6af4ded7e32ae1d6"),
    ];
    let directory = root.join("downloads");
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let staging = tempfile::tempdir_in(&root).map_err(|e| e.to_string())?;
    let output = cap_std::fs::Dir::open_ambient_dir(staging.path(), cap_std::ambient_authority())
        .map_err(|e| e.to_string())?;
    let mut manifests = Vec::new();
    for (name, bytes, sha256) in archives {
        let url = format!("https://github.com/PrismML-Eng/llama.cpp/releases/download/{TAG}/{name}");
        let asset = download::Asset { url: &url, bytes, sha256 };
        let path = directory.join(name);
        let mut last = 0;
        let progress = |n| {
            if n >= last + 64 * 1024 * 1024 || n == bytes {
                println!("{name}: {n}/{bytes} bytes"); last = n;
            }
        };
        if path.exists() {
            download::verify_file(&path, &asset, cancel.clone(), progress).await?;
        } else {
            download::fetch(&client, &asset, &path, cancel.clone(), progress).await?;
        }
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let mut files = Vec::new();
        for i in 0..zip.len() {
            let entry = zip.by_index(i).map_err(|e| e.to_string())?;
            files.push(serde_json::json!({"name":entry.name(), "bytes":entry.size()}));
        }
        let manifest = serde_json::json!({"name":name,"url":url,"sha256":sha256,"bytes":bytes,"files":files});
        let parsed: runtime_archive::RuntimeAsset = serde_json::from_value(manifest.clone()).map_err(|e| e.to_string())?;
        let expanded: u64 = parsed.files.iter().map(|f| f.bytes).sum();
        if download::available_space(staging.path())? < download::required_space(expanded)? {
            return Err("Not enough disk space to extract the Bonsai runtime.".into());
        }
        runtime_archive::extract(&path, &output, &parsed, &cancel)?;
        println!("Verified and extracted {name}: SHA-256 {sha256}");
        manifests.push(manifest);
    }
    if !staging.path().join("llama-server.exe").is_file() || !staging.path().join("llama-bench.exe").is_file() {
        return Err("Release bundle lacks llama-server.exe or llama-bench.exe.".into());
    }
    drop(output);
    let destination = root.join(format!("{TAG}-cuda12.4-{}", uuid::Uuid::new_v4()));
    std::fs::rename(staging.path(), &destination).map_err(|e| e.to_string())?;
    std::fs::write(root.join("runtime-assets.json"), serde_json::to_vec_pretty(&manifests).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    println!("Verified runtime: {}", destination.display());
    Ok(())
}
