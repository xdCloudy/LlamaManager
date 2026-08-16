use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufReader, Read},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use crate::{error::{LlamaManagerError, Result}, llama::{now_ms, sha256_file}};

const MAX_STRING_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ARRAY_PREVIEW: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value")]
pub enum MetadataValue {
    UInt(u64),
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Array { element_type: u32, len: u64, preview: Vec<String> },
}

impl MetadataValue {
    pub fn display_compact(&self) -> String {
        match self {
            Self::UInt(value) => value.to_string(),
            Self::Int(value) => value.to_string(),
            Self::Float(value) => format!("{value:.4}"),
            Self::Bool(value) => value.to_string(),
            Self::String(value) => value.clone(),
            Self::Array { element_type, len, preview } => {
                let suffix = if *len as usize > preview.len() { ", …" } else { "" };
                format!("array(type={element_type}, len={len}) [{}{}]", preview.join(", "), suffix)
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub path: PathBuf,
    pub file_size: u64,
    pub sha256: String,
    pub gguf_version: u32,
    pub tensor_count: u64,
    pub metadata_count: u64,
    pub name: Option<String>,
    pub architecture: Option<String>,
    pub context_length: Option<u64>,
    pub quantization_version: Option<u64>,
    pub metadata: BTreeMap<String, MetadataValue>,
    pub inspected_at_unix_ms: u128,
}

pub fn inspect_gguf(path: &Path) -> Result<ModelInfo> {
    if !path.is_file() {
        return Err(LlamaManagerError::InvalidPath(path.to_path_buf()));
    }

    let file_size = std::fs::metadata(path)?.len();
    let mut reader = BufReader::new(File::open(path)?);

    let mut magic = [0_u8; 4];
    reader.read_exact(&mut magic)?;
    if &magic != b"GGUF" {
        return Err(LlamaManagerError::Gguf("file does not start with GGUF magic".into()));
    }

    let gguf_version = read_u32(&mut reader)?;
    if !(2..=3).contains(&gguf_version) {
        return Err(LlamaManagerError::Gguf(format!(
            "GGUF version {gguf_version} is not supported by this parser"
        )));
    }

    let tensor_count = read_u64(&mut reader)?;
    let metadata_count = read_u64(&mut reader)?;
    if metadata_count > 1_000_000 {
        return Err(LlamaManagerError::Gguf(format!(
            "metadata count {metadata_count} exceeds safety limit"
        )));
    }

    let mut metadata = BTreeMap::new();
    for _ in 0..metadata_count {
        let key = read_string(&mut reader)?;
        let value_type = read_u32(&mut reader)?;
        let value = read_value(&mut reader, value_type)?;
        metadata.insert(key, value);
    }

    let name = metadata_string(&metadata, "general.name");
    let architecture = metadata_string(&metadata, "general.architecture");
    let context_length = architecture.as_ref().and_then(|arch| {
        metadata_u64(&metadata, &format!("{arch}.context_length"))
    });
    let quantization_version = metadata_u64(&metadata, "general.quantization_version");
    let sha256 = sha256_file(path)?;

    Ok(ModelInfo {
        id: format!("model-{}", &sha256[..32]),
        path: path.to_path_buf(),
        file_size,
        sha256,
        gguf_version,
        tensor_count,
        metadata_count,
        name,
        architecture,
        context_length,
        quantization_version,
        metadata,
        inspected_at_unix_ms: now_ms(),
    })
}

fn metadata_string(map: &BTreeMap<String, MetadataValue>, key: &str) -> Option<String> {
    match map.get(key) {
        Some(MetadataValue::String(value)) => Some(value.clone()),
        _ => None,
    }
}

fn metadata_u64(map: &BTreeMap<String, MetadataValue>, key: &str) -> Option<u64> {
    match map.get(key) {
        Some(MetadataValue::UInt(value)) => Some(*value),
        Some(MetadataValue::Int(value)) if *value >= 0 => Some(*value as u64),
        _ => None,
    }
}

fn read_value<R: Read>(reader: &mut R, value_type: u32) -> Result<MetadataValue> {
    Ok(match value_type {
        0 => MetadataValue::UInt(read_u8(reader)? as u64),
        1 => MetadataValue::Int(read_i8(reader)? as i64),
        2 => MetadataValue::UInt(read_u16(reader)? as u64),
        3 => MetadataValue::Int(read_i16(reader)? as i64),
        4 => MetadataValue::UInt(read_u32(reader)? as u64),
        5 => MetadataValue::Int(read_i32(reader)? as i64),
        6 => MetadataValue::Float(read_f32(reader)? as f64),
        7 => MetadataValue::Bool(read_u8(reader)? != 0),
        8 => MetadataValue::String(read_string(reader)?),
        9 => read_array(reader)?,
        10 => MetadataValue::UInt(read_u64(reader)?),
        11 => MetadataValue::Int(read_i64(reader)?),
        12 => MetadataValue::Float(read_f64(reader)?),
        other => return Err(LlamaManagerError::Gguf(format!("unknown metadata value type {other}"))),
    })
}

fn read_array<R: Read>(reader: &mut R) -> Result<MetadataValue> {
    let element_type = read_u32(reader)?;
    let len = read_u64(reader)?;
    if len > 100_000_000 {
        return Err(LlamaManagerError::Gguf(format!("metadata array length {len} exceeds safety limit")));
    }

    let mut preview = Vec::with_capacity((len as usize).min(MAX_ARRAY_PREVIEW));
    for index in 0..len {
        let value = read_value(reader, element_type)?;
        if (index as usize) < MAX_ARRAY_PREVIEW {
            preview.push(value.display_compact());
        }
    }

    Ok(MetadataValue::Array { element_type, len, preview })
}

fn read_string<R: Read>(reader: &mut R) -> Result<String> {
    let len = read_u64(reader)?;
    if len > MAX_STRING_BYTES {
        return Err(LlamaManagerError::Gguf(format!("string length {len} exceeds safety limit")));
    }
    let mut bytes = vec![0_u8; len as usize];
    reader.read_exact(&mut bytes)?;
    String::from_utf8(bytes).map_err(|error| LlamaManagerError::Gguf(format!("invalid UTF-8 metadata string: {error}")))
}

fn read_exact<const N: usize, R: Read>(reader: &mut R) -> Result<[u8; N]> {
    let mut bytes = [0_u8; N];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn read_u8<R: Read>(reader: &mut R) -> Result<u8> { Ok(read_exact::<1, _>(reader)?[0]) }
fn read_i8<R: Read>(reader: &mut R) -> Result<i8> { Ok(read_u8(reader)? as i8) }
fn read_u16<R: Read>(reader: &mut R) -> Result<u16> { Ok(u16::from_le_bytes(read_exact(reader)?)) }
fn read_i16<R: Read>(reader: &mut R) -> Result<i16> { Ok(i16::from_le_bytes(read_exact(reader)?)) }
fn read_u32<R: Read>(reader: &mut R) -> Result<u32> { Ok(u32::from_le_bytes(read_exact(reader)?)) }
fn read_i32<R: Read>(reader: &mut R) -> Result<i32> { Ok(i32::from_le_bytes(read_exact(reader)?)) }
fn read_u64<R: Read>(reader: &mut R) -> Result<u64> { Ok(u64::from_le_bytes(read_exact(reader)?)) }
fn read_i64<R: Read>(reader: &mut R) -> Result<i64> { Ok(i64::from_le_bytes(read_exact(reader)?)) }
fn read_f32<R: Read>(reader: &mut R) -> Result<f32> { Ok(f32::from_le_bytes(read_exact(reader)?)) }
fn read_f64<R: Read>(reader: &mut R) -> Result<f64> { Ok(f64::from_le_bytes(read_exact(reader)?)) }

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn push_string(out: &mut Vec<u8>, value: &str) {
        out.extend_from_slice(&(value.len() as u64).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
    }

    #[test]
    fn parses_minimal_gguf_metadata() {
        let mut data = Vec::new();
        data.extend_from_slice(b"GGUF");
        data.extend_from_slice(&3_u32.to_le_bytes());
        data.extend_from_slice(&0_u64.to_le_bytes());
        data.extend_from_slice(&2_u64.to_le_bytes());

        push_string(&mut data, "general.architecture");
        data.extend_from_slice(&8_u32.to_le_bytes());
        push_string(&mut data, "qwen35");

        push_string(&mut data, "qwen35.context_length");
        data.extend_from_slice(&10_u32.to_le_bytes());
        data.extend_from_slice(&262_144_u64.to_le_bytes());

        let mut cursor = Cursor::new(&data[4..]);
        assert_eq!(read_u32(&mut cursor).unwrap(), 3);
        assert_eq!(read_u64(&mut cursor).unwrap(), 0);
        assert_eq!(read_u64(&mut cursor).unwrap(), 2);
        let key = read_string(&mut cursor).unwrap();
        assert_eq!(key, "general.architecture");
        let ty = read_u32(&mut cursor).unwrap();
        assert_eq!(read_value(&mut cursor, ty).unwrap().display_compact(), "qwen35");
    }
}
