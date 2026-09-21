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
    /// `--slot-save-path` directory when the loaded runtime supports it.
    pub slot_dir: Option<PathBuf>,
    /// Identity of the loaded model + launch configuration (kv_slots.rs).
    pub cache_key: String,
    /// Conversation whose KV state is believed to occupy the conversation
    /// slot. A wrong guess only costs prefill; llama.cpp verifies the prefix.
    pub slot_resident: Option<String>,
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
            slot_dir: None,
            cache_key: String::new(),
            slot_resident: None,
        }
    }
    pub fn inspect(&mut self) -> RuntimeStatus {
        if let Some(child) = &mut self.child {
            match child.try_wait() {
                Ok(Some(code)) => {
                    self.child = None;
                    self.status = RuntimeStatus {
                        phase: "error".into(),
                        message: format!(
                            "Model process exited ({code}). See {}",
                            self.log_path.display()
                        ),
                        model_path: None,
                        loaded_config: None,
                        gpu_offload: None,
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
        self.slot_dir = None;
        self.cache_key.clear();
        self.slot_resident = None;
        Ok(self.status.clone())
    }
    pub async fn load(
        &mut self,
        preferences: &Preferences,
        config: &RuntimeConfig,
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
        let projector = if preferences.projector_path.trim().is_empty() {
            None
        } else {
            let path = std::fs::canonicalize(preferences.projector_path.trim())
                .map_err(|_| "Select an existing mmproj projector file in Models, or clear it for text-only chat.")?;
            validate_projector(&path)?;
            Some(path)
        };
        let flags = inspect_runtime(&executable).await?;
        let launch = resolve_launch_config(config, &model, flags.fit).await?;
        let model_bytes = std::fs::metadata(&model).map(|metadata| metadata.len()).unwrap_or(0);
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
        command
            .args(launch.arguments()?)
            .arg("--model")
            .arg(&model);
        if let Some(projector) = &projector {
            command.arg("--mmproj").arg(projector);
        } else if flags.no_mmproj {
            // Newer llama-server may auto-discover an mmproj beside the GGUF.
            command.arg("--no-mmproj");
        }
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
            command.arg("--fit").arg(if config.automatic_gpu() { "on" } else { "off" });
        }
        // Enables the slot save/restore endpoints only; whether anything is
        // written is decided per turn by the KV cache setting.
        let slot_dir = if flags.slot_save {
            let dir = self.log_path.with_file_name("kv-slots");
            std::fs::create_dir_all(&dir).map_err(|error| format!("Cannot create the KV slot directory: {error}"))?;
            command.arg("--slot-save-path").arg(&dir);
            Some(dir)
        } else {
            None
        };
        command
            .env("LLAMA_API_KEY", &self.api_key)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Older Prism builds predate --no-agent. They default to no tools;
        // runtime_command removes LLAMA_* overrides for both old and new builds.
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
        };
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|error| error.to_string())?;
        for _ in 0..attempts {
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
                            self.slot_dir = slot_dir.clone();
                            self.cache_key = crate::kv_slots::cache_key(&model, projector.as_deref(), &executable, &launch);
                            self.status.loaded_config = Some(config.clone());
                            self.status.gpu_offload = read_offload(&self.log_path);
                            self.status.phase = "ready".into();
                            self.status.message = if projector.is_some() {
                                "Model ready · vision enabled".into()
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
    // Inherited llama options could expose tools or change the selected model.
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("LLAMA_") {
            command.env_remove(key);
        }
    }
    command
}

struct RuntimeFlags {
    no_agent: bool,
    fit: bool,
    no_mmproj: bool,
    slot_save: bool,
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
    Ok(RuntimeFlags {
        no_agent: help.contains("--no-agent"),
        fit: help.contains("--fit"),
        no_mmproj: help.contains("--no-mmproj"),
        slot_save: help.contains("--slot-save-path"),
    })
}

async fn resolve_launch_config(
    config: &RuntimeConfig,
    model: &Path,
    fit_supported: bool,
) -> Result<RuntimeConfig, String> {
    if !config.automatic_gpu() || fit_supported {
        return Ok(config.clone());
    }
    let available = crate::hardware::hardware_status().await.free_vram_bytes();
    let mut launch = config.clone();
    launch.gpu_layers = crate::gguf::model_metadata(model)
        .ok()
        .map(|metadata| RuntimeConfig::auto_gpu_layers(&metadata, &launch, available))
        .unwrap_or(0);
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
        if metadata.context_length.is_some_and(|max| config.context_length > max) {
            return Err(format!("This model supports at most {} context tokens. Adjust Context window in Runtime before loading.", metadata.context_length.unwrap()));
        }
        if metadata.block_count.is_some_and(|blocks| config.gpu_layers > 0 && config.gpu_layers as u32 > blocks.saturating_add(1)) {
            return Err("GPU layer selection exceeds the model's layer count. Choose Automatic or reduce the manual count in Runtime.".into());
        }
    }
    if path.file_name().and_then(|name| name.to_str()) == Some(crate::model_catalog::BONSAI_FILENAME)
        && config.context_length > crate::model_catalog::BONSAI_CONTEXT_LENGTH
    {
        return Err("Bonsai Q2_0 supports at most 65,536 context tokens. Set Context window to 65,536 or less in Runtime, save settings, then load again.".into());
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
        return Err("The projector must be an mmproj GGUF file (for example mmproj-model-f16.gguf).".into());
    }
    validate_model(path).map_err(|_| "The projector file is incomplete or is not a GGUF model.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn large_ggufs_get_a_longer_startup_window() {
        assert_eq!(startup_attempts(2 * 1024 * 1024 * 1024), 360);
        assert_eq!(startup_attempts(4 * 1024 * 1024 * 1024), 1200);
        assert_eq!(startup_attempts(6 * 1024 * 1024 * 1024), 1200);
    }

    #[test]
    fn bonsai_rejects_oversized_context_before_starting_runtime() {
        let bonsai = Path::new(crate::model_catalog::BONSAI_FILENAME);
        let mut config = RuntimeConfig { context_length: 131_072, ..RuntimeConfig::default() };
        assert!(validate_model_context(bonsai, &config).unwrap_err().contains("65,536 or less"));
        config.context_length = 65536;
        assert!(validate_model_context(bonsai, &config).is_ok());
        config.context_length = 4096;
        assert!(validate_model_context(bonsai, &config).is_ok());
        assert!(validate_model_context(Path::new(crate::model_catalog::MODEL_FILENAME), &RuntimeConfig::default()).is_ok());
    }

    #[tokio::test]
    #[ignore = "requires cached upstream and Prism Windows CUDA runtime installations"]
    async fn recognizes_old_prism_and_current_runtime_flags() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let current = inspect_runtime(&root.join(".local/runtime/llama-server.exe")).await.unwrap();
        assert!(current.no_agent);
        assert!(current.fit);
        let prism = inspect_runtime(&root.join(".local/runtime-prism-b9601-68faa14/llama-server.exe")).await.unwrap();
        assert!(!prism.no_agent);
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
        std::fs::write(&valid, b"GGUF").unwrap();
        assert!(validate_projector(&valid).is_ok());
        let wrong_name = directory.path().join("model.gguf");
        std::fs::write(&wrong_name, b"GGUF").unwrap();
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
