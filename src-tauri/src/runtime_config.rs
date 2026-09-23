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
    /// llama.cpp `--parallel` / slot count. 1 is the 4 GB default; 2 keeps
    /// parent KV when a subagent runs and roughly doubles cache memory.
    #[serde(default = "default_inference_slots")]
    pub inference_slots: u32,
}

fn default_inference_slots() -> u32 {
    1
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
            // 32k keeps the KV cache small enough for 4 GB-class GPUs; larger
            // contexts push the cache into host memory and slow every round.
            context_length: 32768,
            gpu_layers: -1,
            cpu_threads: 6,
            batch_size: 512,
            micro_batch_size: 128,
            flash_attention: true,
            cache_type_k: CacheType::Q8_0,
            cache_type_v: CacheType::Q8_0,
            offload_kv_cache: true,
            mmap: true,
            inference_slots: 1,
        }
    }
}

impl RuntimeConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !(128..=2_097_152).contains(&self.context_length) {
            return Err("Context must be between 128 and 2097152 tokens.".into());
        }
        if !(-1..=999).contains(&self.gpu_layers) {
            return Err("GPU layers must be -1 (automatic), 0 (CPU), or 1–999.".into());
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
        if !(1..=2).contains(&self.inference_slots) {
            return Err("Inference slots must be 1 or 2.".into());
        }
        Ok(())
    }

    pub fn arguments(&self) -> Result<Vec<String>, String> {
        self.validate()?;
        let mut args = vec![
            "--ctx-size".into(),
            self.context_length.to_string(),
        ];
        // -1 means automatic: llama.cpp --fit chooses how many layers fill
        // VRAM and leaves the rest in RAM. Passing --n-gpu-layers disables that.
        if self.gpu_layers >= 0 {
            args.push("--n-gpu-layers".into());
            args.push(self.gpu_layers.to_string());
        }
        args.extend([
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
        ]);
        if !self.offload_kv_cache {
            args.push("--no-kv-offload".into());
        }
        if !self.mmap {
            args.push("--no-mmap".into());
        }
        args.push("--parallel".into());
        args.push(self.inference_slots.to_string());
        Ok(args)
    }

    pub fn automatic_gpu(&self) -> bool {
        self.gpu_layers < 0
    }

    /// How many layers fit in free VRAM (0 = CPU, -1 = the whole model).
    /// Leaves 1 GiB of GPU headroom for compute buffers and the desktop.
    pub fn auto_gpu_layers(
        model: &crate::gguf::ModelMetadata,
        config: &Self,
        available_gpu_bytes: Option<u64>,
    ) -> i32 {
        let Some(blocks) = model.block_count.filter(|count| *count > 0) else {
            return 0;
        };
        let max = i32::try_from(blocks.saturating_add(1)).unwrap_or(999).clamp(1, 999);
        let Some(available) = available_gpu_bytes else {
            return 0;
        };
        let usable = available.saturating_sub(1024 * 1024 * 1024);
        if usable == 0 {
            return 0;
        }
        let kv = kv_cache_bytes(model, config).unwrap_or(0);
        let gpu_full = model
            .file_bytes
            .saturating_add(if config.offload_kv_cache { kv } else { 0 });
        if gpu_full == 0 {
            return 0;
        }
        if usable >= gpu_full {
            return -1;
        }
        let fitted = ((usable as f64 / gpu_full as f64) * f64::from(max)).floor() as i32;
        fitted.clamp(0, max)
    }
}

fn kv_cache_bytes(model: &crate::gguf::ModelMetadata, config: &RuntimeConfig) -> Option<u64> {
    let architecture = model.architecture.as_deref()?;
    if !matches!(
        architecture,
        "llama" | "qwen2" | "qwen3" | "gemma" | "gemma2" | "mistral" | "phi3"
    ) {
        return None;
    }
    let blocks = u64::from(model.block_count.filter(|count| *count > 0)?);
    let heads_kv = u64::from(model.head_count_kv.filter(|count| *count > 0)?);
    let head = model
        .embedding_length
        .zip(model.head_count)
        .and_then(|(embed, heads)| (heads > 0).then_some(embed / heads));
    let key = u64::from(model.key_length.or(head).filter(|value| *value > 0)?);
    let value = u64::from(model.value_length.or(head).filter(|value| *value > 0)?);
    let slots = u64::from(config.inference_slots.max(1));
    Some(
        u64::from(config.context_length)
            * blocks
            * heads_kv
            * (cache_elem_bytes(config.cache_type_k, key) + cache_elem_bytes(config.cache_type_v, value))
            * slots,
    )
}

fn cache_elem_bytes(cache: CacheType, elements: u64) -> u64 {
    match cache {
        CacheType::F16 => elements * 2,
        CacheType::Q8_0 => elements * 34 / 32,
        CacheType::Q4_0 => elements * 18 / 32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_configuration_is_valid_for_initial_load() {
        let config = RuntimeConfig::default();
        assert!(config.validate().is_ok());
        assert_eq!(config.context_length, 32768);
        assert_eq!(config.inference_slots, 1);
        assert!(config.arguments().unwrap().windows(2).any(|pair| pair == ["--ctx-size", "32768"]));
        assert!(config.arguments().unwrap().windows(2).any(|pair| pair == ["--parallel", "1"]));
        assert!(!config.arguments().unwrap().iter().any(|arg| arg == "--n-gpu-layers"));
    }

    #[test]
    fn automatic_gpu_omits_n_gpu_layers_so_llama_cpp_can_fit() {
        let auto = RuntimeConfig::default();
        assert!(auto.automatic_gpu());
        assert!(!auto.arguments().unwrap().iter().any(|arg| arg == "--n-gpu-layers"));
        let cpu = RuntimeConfig { gpu_layers: 0, ..Default::default() };
        assert!(cpu.arguments().unwrap().windows(2).any(|pair| pair == ["--n-gpu-layers", "0"]));
        let split = RuntimeConfig { gpu_layers: 18, ..Default::default() };
        assert!(split.arguments().unwrap().windows(2).any(|pair| pair == ["--n-gpu-layers", "18"]));
    }

    #[test]
    fn auto_gpu_layers_fills_vram_then_spills_to_ram() {
        let model = crate::gguf::ModelMetadata {
            architecture: Some("llama".into()),
            context_length: Some(8192),
            block_count: Some(32),
            embedding_length: Some(4096),
            head_count: Some(32),
            head_count_kv: Some(8),
            key_length: None,
            value_length: None,
            file_bytes: 4 * 1024 * 1024 * 1024,
        };
        let config = RuntimeConfig { context_length: 8192, ..Default::default() };
        assert_eq!(RuntimeConfig::auto_gpu_layers(&model, &config, None), 0);
        assert_eq!(
            RuntimeConfig::auto_gpu_layers(&model, &config, Some(24 * 1024 * 1024 * 1024)),
            -1
        );
        let tight = RuntimeConfig::auto_gpu_layers(&model, &config, Some(5 * 1024 * 1024 * 1024));
        assert!(tight > 0 && tight < 33, "tight fit was {tight}");
    }

    #[test]
    fn inference_slots_are_optional_in_saved_json_and_capped() {
        let parsed: RuntimeConfig = serde_json::from_value(serde_json::json!({
            "contextLength": 4096,
            "gpuLayers": 0,
            "cpuThreads": 4,
            "batchSize": 128,
            "microBatchSize": 32,
            "flashAttention": true,
            "cacheTypeK": "f16",
            "cacheTypeV": "f16",
            "offloadKvCache": false,
            "mmap": true
        }))
        .unwrap();
        assert_eq!(parsed.inference_slots, 1);
        let two = RuntimeConfig { inference_slots: 2, ..Default::default() };
        assert!(two.validate().is_ok());
        assert!(two.arguments().unwrap().windows(2).any(|pair| pair == ["--parallel", "2"]));
        let three = RuntimeConfig { inference_slots: 3, ..Default::default() };
        assert!(three.validate().is_err());
    }

    #[test]
    fn rejects_invalid_resource_limits() {
        for context_length in [0, 127, 2_097_153] {
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
    fn permits_large_model_contexts_within_the_harness_safety_cap() {
        assert!(RuntimeConfig { context_length: 262144, ..Default::default() }.validate().is_ok());
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
        assert!(args.windows(2).any(|pair| pair == ["--parallel", "1"]));
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
