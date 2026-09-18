//! Minimal GGUF header reader: enough to recover `general.architecture`
//! without loading weights, so a model whose architecture the selected
//! runtime cannot support is caught before a doomed load.

use std::io::Read;
use std::path::Path;

/// Header budget: GGUF metadata (keys, tokenizer arrays) can be large but is
/// bounded; a bigger header means a corrupt or hostile file.
const MAX_HEADER_BYTES: u64 = 64 * 1024 * 1024;

const TYPE_U8: u32 = 0;
const TYPE_I8: u32 = 1;
const TYPE_U16: u32 = 2;
const TYPE_I16: u32 = 3;
const TYPE_U32: u32 = 4;
const TYPE_I32: u32 = 5;
const TYPE_F32: u32 = 6;
const TYPE_BOOL: u32 = 7;
const TYPE_STRING: u32 = 8;
const TYPE_ARRAY: u32 = 9;
const TYPE_U64: u32 = 10;
const TYPE_I64: u32 = 11;
const TYPE_F64: u32 = 12;

fn read_u32(file: &mut impl Read) -> std::io::Result<u32> {
    let mut buffer = [0u8; 4];
    file.read_exact(&mut buffer)?;
    Ok(u32::from_le_bytes(buffer))
}

fn read_u64(file: &mut impl Read) -> std::io::Result<u64> {
    let mut buffer = [0u8; 8];
    file.read_exact(&mut buffer)?;
    Ok(u64::from_le_bytes(buffer))
}

fn read_string(file: &mut impl Read) -> std::io::Result<String> {
    let length = read_u64(file)?;
    if length > MAX_HEADER_BYTES {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "GGUF string exceeds header budget"));
    }
    let mut buffer = vec![0u8; length as usize];
    file.read_exact(&mut buffer)?;
    String::from_utf8(buffer).map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "GGUF string is not UTF-8"))
}

/// Skip one metadata value of `value_type`, tracking the header budget.
fn skip_value(file: &mut impl Read, value_type: u32, budget: &mut u64) -> std::io::Result<()> {
    match value_type {
        TYPE_U8 | TYPE_I8 | TYPE_BOOL => consume(file, 1, budget),
        TYPE_U16 | TYPE_I16 => consume(file, 2, budget),
        TYPE_U32 | TYPE_I32 | TYPE_F32 => consume(file, 4, budget),
        TYPE_U64 | TYPE_I64 | TYPE_F64 => consume(file, 8, budget),
        TYPE_STRING => {
            let length = read_u64(file)?;
            consume(file, length, budget)
        }
        TYPE_ARRAY => {
            let element_type = read_u32(file)?;
            if element_type == TYPE_ARRAY { return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Nested GGUF arrays are not supported")); }
            let count = read_u64(file)?;
            if count > MAX_HEADER_BYTES {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "GGUF array exceeds header budget"));
            }
            for _ in 0..count {
                skip_value(file, element_type, budget)?;
            }
            Ok(())
        }
        other => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Unknown GGUF metadata type {other}"),
        )),
    }
}

#[derive(Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelMetadata {
    pub architecture: Option<String>,
    pub context_length: Option<u32>,
    pub block_count: Option<u32>,
    pub embedding_length: Option<u32>,
    pub head_count: Option<u32>,
    pub head_count_kv: Option<u32>,
    pub key_length: Option<u32>,
    pub value_length: Option<u32>,
    pub file_bytes: u64,
}

pub fn model_metadata(path: &Path) -> Result<ModelMetadata, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = file.metadata().map_err(|e| e.to_string())?.len();
    if let Some(name) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".gguf")) {
        if let Some((left, total)) = name.rsplit_once("-of-") {
            if let (Some((base, _)), Ok(count)) = (left.rsplit_once('-'), total.parse::<usize>()) {
                if (1..=999).contains(&count) {
                    bytes = 0;
                    for i in 1..=count {
                        let part = path.with_file_name(format!("{base}-{i:05}-of-{count:05}.gguf"));
                        bytes = bytes.checked_add(std::fs::metadata(part).map_err(|_| "Model has missing shards.")?.len()).ok_or("Model size overflow.")?;
                    }
                }
            }
        }
    }
    parse_metadata(&mut file.take(MAX_HEADER_BYTES), bytes).map_err(|e| format!("Cannot read model metadata: {e}"))
}
fn parse_metadata(file: &mut impl Read, file_bytes: u64) -> std::io::Result<ModelMetadata> {
    let invalid = || std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid GGUF header");
    let mut magic = [0; 4]; file.read_exact(&mut magic)?;
    if &magic != b"GGUF" || !(2..=3).contains(&read_u32(file)?) { return Err(invalid()); }
    let _tensors = read_u64(file)?;
    let count = read_u64(file)?;
    if count > 1_000_000 { return Err(invalid()); }
    let mut numbers = std::collections::HashMap::new();
    let mut result = ModelMetadata { file_bytes, ..Default::default() };
    let mut budget = MAX_HEADER_BYTES;
    for _ in 0..count {
        let key = read_string(file)?;
        let kind = read_u32(file)?;
        if key == "general.architecture" && kind == TYPE_STRING { result.architecture = Some(read_string(file)?); }
        else if kind == TYPE_U32 { numbers.insert(key, read_u32(file)?); }
        else if kind == TYPE_U64 { let value = read_u64(file)?; if let Ok(value) = u32::try_from(value) { numbers.insert(key, value); } }
        else { skip_value(file, kind, &mut budget)?; }
    }
    if let Some(architecture) = &result.architecture {
        let number = |suffix: &str| numbers.get(&format!("{architecture}.{suffix}")).copied().filter(|n| *n > 0);
        result.context_length = number("context_length"); result.block_count = number("block_count");
        result.embedding_length = number("embedding_length"); result.head_count = number("attention.head_count");
        result.head_count_kv = number("attention.head_count_kv"); result.key_length = number("attention.key_length"); result.value_length = number("attention.value_length");
    }
    Ok(result)
}

#[tauri::command]
pub async fn read_model_metadata(path: String) -> Result<ModelMetadata, String> {
    tauri::async_runtime::spawn_blocking(move || model_metadata(Path::new(&path))).await.map_err(|e| e.to_string())?
}

fn consume(file: &mut impl Read, bytes: u64, budget: &mut u64) -> std::io::Result<()> {
    if bytes > *budget {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "GGUF header exceeds its size budget"));
    }
    *budget -= bytes;
    if std::io::copy(&mut file.take(bytes), &mut std::io::sink())? != bytes {
        return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "Truncated GGUF metadata"));
    }
    Ok(())
}

/// Read `general.architecture` from a GGUF file. `Ok(None)` means the file
/// could not be parsed as GGUF or carries no architecture key — the runtime
/// remains responsible for reporting that; this check only refuses what it
/// can positively identify.
pub fn read_architecture(path: &Path) -> Result<Option<String>, String> {
    let mut file = std::fs::File::open(path).map_err(|error| error.to_string())?.take(MAX_HEADER_BYTES);
    let mut budget = MAX_HEADER_BYTES;
    let mut magic = [0u8; 4];
    if file.read_exact(&mut magic).is_err() || &magic != b"GGUF" {
        return Ok(None);
    }
    budget -= 4;
    let version = read_u32(&mut file).map_err(|_| "Could not read the GGUF version.".to_string())?;
    budget -= 4;
    if !(2..=3).contains(&version) {
        return Ok(None);
    }
    let _tensor_count = read_u64(&mut file).map_err(|_| "Could not read the GGUF tensor count.".to_string())?;
    let kv_count = read_u64(&mut file).map_err(|_| "Could not read the GGUF metadata count.".to_string())?;
    budget -= 16;
    if kv_count > MAX_HEADER_BYTES {
        return Ok(None);
    }
    for _ in 0..kv_count {
        let key = read_string(&mut file).map_err(|_| "Malformed GGUF metadata key.".to_string())?;
        if key.len() as u64 > budget {
            return Ok(None);
        }
        budget -= key.len() as u64;
        let value_type = read_u32(&mut file).map_err(|_| "Malformed GGUF metadata value type.".to_string())?;
        budget = budget.checked_sub(4).ok_or("GGUF header exceeds its size budget.")?;
        if key == "general.architecture" && value_type == TYPE_STRING {
            let architecture = read_string(&mut file).map_err(|_| "Malformed GGUF architecture value.".to_string())?;
            return Ok(Some(architecture));
        }
        skip_value(&mut file, value_type, &mut budget).map_err(|error| error.to_string())?;
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_model_limits_even_when_architecture_follows_numeric_fields() {
        let mut kvs = Vec::new();
        for (key, value) in [("llama.context_length", 262144u32), ("llama.block_count", 32), ("llama.attention.head_count_kv", 8)] {
            kvs.extend_from_slice(&(key.len() as u64).to_le_bytes()); kvs.extend_from_slice(key.as_bytes());
            kvs.extend_from_slice(&TYPE_U32.to_le_bytes()); kvs.extend_from_slice(&value.to_le_bytes());
        }
        kvs.extend_from_slice(&string_kv("general.architecture", "llama"));
        let bytes = header(kvs, 4, 3);
        let metadata = parse_metadata(&mut bytes.as_slice(), 100).unwrap();
        assert_eq!(metadata.context_length, Some(262144)); assert_eq!(metadata.block_count, Some(32)); assert_eq!(metadata.head_count_kv, Some(8));
        assert!(parse_metadata(&mut &bytes[..bytes.len()-1], 100).is_err());
    }

    fn string_kv(key: &str, value: &str) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(key.len() as u64).to_le_bytes());
        out.extend_from_slice(key.as_bytes());
        out.extend_from_slice(&TYPE_STRING.to_le_bytes());
        out.extend_from_slice(&(value.len() as u64).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
        out
    }

    fn header(kvs: Vec<u8>, kv_count: u64, version: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"GGUF");
        out.extend_from_slice(&version.to_le_bytes());
        out.extend_from_slice(&0u64.to_le_bytes()); // tensor count
        out.extend_from_slice(&kv_count.to_le_bytes());
        out.extend_from_slice(&kvs);
        out
    }

    fn write_temp(name: &str, bytes: &[u8]) -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join(name), bytes).unwrap();
        directory
    }

    #[test]
    fn reads_the_architecture_key_from_a_valid_header() {
        let directory = write_temp(
            "model.gguf",
            &header(string_kv("general.architecture", "zaya"), 1, 3),
        );
        assert_eq!(
            read_architecture(&directory.path().join("model.gguf")).unwrap(),
            Some("zaya".to_string())
        );
    }

    #[test]
    fn skips_arrays_and_other_keys_without_confusion() {
        let mut kvs = Vec::new();
        // An i32 scalar before the target key.
        kvs.extend_from_slice(&(b"general.size_label".len() as u64).to_le_bytes());
        kvs.extend_from_slice(b"general.size_label");
        kvs.extend_from_slice(&TYPE_I32.to_le_bytes());
        kvs.extend_from_slice(&7i32.to_le_bytes());
        // A string array (tokenizer-style) before the target key.
        kvs.extend_from_slice(&(b"tokenizer.ggml.tokens".len() as u64).to_le_bytes());
        kvs.extend_from_slice(b"tokenizer.ggml.tokens");
        kvs.extend_from_slice(&TYPE_ARRAY.to_le_bytes());
        kvs.extend_from_slice(&TYPE_STRING.to_le_bytes());
        kvs.extend_from_slice(&2u64.to_le_bytes());
        for token in ["<bos>", "hello"] {
            kvs.extend_from_slice(&(token.len() as u64).to_le_bytes());
            kvs.extend_from_slice(token.as_bytes());
        }
        kvs.extend_from_slice(&string_kv("general.architecture", "gemma3"));
        let directory = write_temp("model.gguf", &header(kvs, 3, 3));
        assert_eq!(
            read_architecture(&directory.path().join("model.gguf")).unwrap(),
            Some("gemma3".to_string())
        );
    }

    #[test]
    fn unparseable_and_non_gguf_files_are_not_refused() {
        let directory = write_temp("model.gguf", b"not a gguf file at all");
        assert_eq!(read_architecture(&directory.path().join("model.gguf")).unwrap(), None);
        let directory = write_temp("model.gguf", &header(string_kv("general.architecture", "zaya"), 1, 9));
        assert_eq!(read_architecture(&directory.path().join("model.gguf")).unwrap(), None, "unsupported GGUF version is left to the runtime");
        assert!(read_architecture(Path::new("Z:/missing/model.gguf")).is_err());
    }
}
