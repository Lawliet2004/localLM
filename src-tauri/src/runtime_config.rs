use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeConfig {
    pub context_length: u32,
    pub gpu_layers: i32,
    pub cpu_threads: u32,
    pub batch_size: u32,
    pub micro_batch_size: u32,
    pub flash_attention: bool,
    pub cache_type_k: CacheType,
    pub cache_type_v: CacheType,
    pub offload_kv_cache: bool,
    pub mmap: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CacheType {
    F16,
    Q8_0,
    Q4_0,
}

impl CacheType {
    fn argument(self) -> &'static str {
        match self {
            Self::F16 => "f16",
            Self::Q8_0 => "q8_0",
            Self::Q4_0 => "q4_0",
        }
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            context_length: 8192,
            gpu_layers: -1,
            cpu_threads: 6,
            batch_size: 512,
            micro_batch_size: 128,
            flash_attention: true,
            cache_type_k: CacheType::Q8_0,
            cache_type_v: CacheType::Q8_0,
            offload_kv_cache: true,
            mmap: true,
        }
    }
}

impl RuntimeConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !(128..=131072).contains(&self.context_length) {
            return Err("Context must be between 128 and 131072 tokens.".into());
        }
        if !(-1..=999).contains(&self.gpu_layers) {
            return Err("GPU layers must be -1 (all), 0 (CPU), or 1–999.".into());
        }
        if !(1..=256).contains(&self.cpu_threads) {
            return Err("CPU threads must be between 1 and 256.".into());
        }
        if !(1..=8192).contains(&self.batch_size) {
            return Err("Batch size must be between 1 and 8192.".into());
        }
        if self.micro_batch_size == 0 || self.micro_batch_size > self.batch_size {
            return Err("Micro batch size must be positive and no larger than batch size.".into());
        }
        if !self.flash_attention && self.cache_type_v != CacheType::F16 {
            return Err("Quantized value cache requires Flash Attention.".into());
        }
        Ok(())
    }

    pub fn arguments(&self) -> Result<Vec<String>, String> {
        self.validate()?;
        let mut args = vec![
            "--ctx-size".into(),
            self.context_length.to_string(),
            "--n-gpu-layers".into(),
            self.gpu_layers.to_string(),
            "--threads".into(),
            self.cpu_threads.to_string(),
            "--batch-size".into(),
            self.batch_size.to_string(),
            "--ubatch-size".into(),
            self.micro_batch_size.to_string(),
            "--flash-attn".into(),
            if self.flash_attention { "on" } else { "off" }.into(),
            "--cache-type-k".into(),
            self.cache_type_k.argument().into(),
            "--cache-type-v".into(),
            self.cache_type_v.argument().into(),
        ];
        if !self.offload_kv_cache {
            args.push("--no-kv-offload".into());
        }
        if !self.mmap {
            args.push("--no-mmap".into());
        }
        Ok(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_configuration_is_valid_for_initial_load() {
        let config = RuntimeConfig::default();
        assert!(config.validate().is_ok());
        assert_eq!(config.context_length, 8192);
    }

    #[test]
    fn rejects_invalid_resource_limits() {
        for context_length in [0, 127, 131073] {
            let config = RuntimeConfig {
                context_length,
                ..Default::default()
            };
            assert!(config.validate().is_err());
        }
        let config = RuntimeConfig {
            cpu_threads: 0,
            ..Default::default()
        };
        assert!(config.validate().is_err());
        let config = RuntimeConfig {
            micro_batch_size: 1024,
            batch_size: 256,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn quantized_value_cache_requires_flash_attention() {
        let mut config = RuntimeConfig {
            flash_attention: false,
            ..Default::default()
        };
        assert!(config.validate().is_err());
        config.cache_type_v = CacheType::F16;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn cpu_mode_and_ram_cache_are_explicit_in_arguments() {
        let config = RuntimeConfig {
            gpu_layers: 0,
            offload_kv_cache: false,
            mmap: false,
            ..Default::default()
        };
        let args = config.arguments().unwrap();
        assert!(args.windows(2).any(|pair| pair == ["--n-gpu-layers", "0"]));
        assert!(args.iter().any(|arg| arg == "--no-kv-offload"));
        assert!(args.iter().any(|arg| arg == "--no-mmap"));
        assert!(!args.iter().any(|arg| arg == "--host"));
    }

    #[test]
    fn rejects_unknown_cache_types_and_unrecognized_settings() {
        let mut value = serde_json::to_value(RuntimeConfig::default()).unwrap();
        value["cacheTypeK"] = serde_json::json!("arbitrary");
        assert!(serde_json::from_value::<RuntimeConfig>(value).is_err());
        let mut value = serde_json::to_value(RuntimeConfig::default()).unwrap();
        value["extraArguments"] = serde_json::json!("--host 0.0.0.0");
        assert!(serde_json::from_value::<RuntimeConfig>(value).is_err());
    }
}
