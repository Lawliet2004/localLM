use crate::{runtime_config::RuntimeConfig, store::Preferences};
use serde::Serialize;
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::{Child, Command};

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
    child: Option<Child>,
    pub status: RuntimeStatus,
    pub endpoint: String,
    pub api_key: String,
    pub context_length: u32,
    log_path: PathBuf,
}
impl Runtime {
    pub fn new(log_path: PathBuf) -> Self {
        Self {
            child: None,
            status: RuntimeStatus::default(),
            endpoint: String::new(),
            api_key: String::new(),
            context_length: 0,
            log_path,
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
            child
                .kill()
                .await
                .map_err(|error| format!("Could not stop model: {error}"))?;
        }
        self.status = RuntimeStatus::default();
        self.api_key.clear();
        self.endpoint.clear();
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
        self.stop().await?;
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .map_err(|error| error.to_string())?
            .local_addr()
            .map_err(|error| error.to_string())?
            .port();
        self.api_key = uuid::Uuid::new_v4().to_string();
        self.endpoint = format!("http://127.0.0.1:{port}");
        let log = std::fs::File::create(&self.log_path)
            .map_err(|error| format!("Cannot create runtime log: {error}"))?;
        let mut command = Command::new(executable);
        // Inherited llama options could expose tools or change the selected model.
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("LLAMA_") {
                command.env_remove(key);
            }
        }
        command
            .args(config.arguments()?)
            .arg("--model")
            .arg(&model)
            .args([
                "--host",
                "127.0.0.1",
                "--port",
                &port.to_string(),
                "--parallel",
                "1",
                "--jinja",
                "--no-webui",
                "--no-agent",
                "--fit",
                "off",
                "--log-verbosity",
                "4",
            ])
            .env("LLAMA_API_KEY", &self.api_key)
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|error| error.to_string())?)
            .stderr(log)
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        self.child = Some(
            command
                .spawn()
                .map_err(|error| format!("Cannot start llama-server: {error}"))?,
        );
        self.status = RuntimeStatus {
            phase: "loading".into(),
            message: "Loading model into memory".into(),
            model_path: Some(model.to_string_lossy().into_owned()),
            loaded_config: None,
            gpu_offload: None,
        };
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|error| error.to_string())?;
        for _ in 0..120 {
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
                            self.status.loaded_config = Some(config.clone());
                            self.status.gpu_offload = read_offload(&self.log_path);
                            self.status.phase = "ready".into();
                            self.status.message = "Model ready".into();
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

#[cfg(test)]
mod tests {
    use super::*;
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
}
