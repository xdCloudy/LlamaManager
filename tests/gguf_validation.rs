use std::{fs, io::ErrorKind, path::Path};

use llamamanager::{
    error::LlamaManagerError,
    gguf::{MetadataValue, inspect_gguf},
};
use sha2::{Digest, Sha256};
use tempfile::tempdir;

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

fn push_header(out: &mut Vec<u8>, version: u32, metadata_count: u64) {
    out.extend_from_slice(b"GGUF");
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&0_u64.to_le_bytes());
    out.extend_from_slice(&metadata_count.to_le_bytes());
}

fn push_string(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u64).to_le_bytes());
    out.extend_from_slice(value.as_bytes());
}

fn push_key_and_type(out: &mut Vec<u8>, key: &str, value_type: u32) {
    push_string(out, key);
    out.extend_from_slice(&value_type.to_le_bytes());
}

fn push_string_entry(out: &mut Vec<u8>, key: &str, value: &str) {
    push_key_and_type(out, key, TYPE_STRING);
    push_string(out, value);
}

fn build_metadata_fixture(version: u32) -> Vec<u8> {
    const METADATA_COUNT: u64 = 19;

    let mut out = Vec::new();
    push_header(&mut out, version, METADATA_COUNT);

    push_string_entry(&mut out, "general.name", "Validation Fixture");
    push_string_entry(&mut out, "general.architecture", "qwen35");

    push_key_and_type(&mut out, "qwen35.context_length", TYPE_U64);
    out.extend_from_slice(&262_144_u64.to_le_bytes());

    push_key_and_type(&mut out, "general.quantization_version", TYPE_U32);
    out.extend_from_slice(&2_u32.to_le_bytes());

    push_string_entry(&mut out, "validation.unknown_key", "preserved");

    push_key_and_type(&mut out, "validation.u8", TYPE_U8);
    out.push(255);

    push_key_and_type(&mut out, "validation.i8", TYPE_I8);
    out.push((-8_i8) as u8);

    push_key_and_type(&mut out, "validation.u16", TYPE_U16);
    out.extend_from_slice(&65_530_u16.to_le_bytes());

    push_key_and_type(&mut out, "validation.i16", TYPE_I16);
    out.extend_from_slice(&(-1_234_i16).to_le_bytes());

    push_key_and_type(&mut out, "validation.u32", TYPE_U32);
    out.extend_from_slice(&4_000_000_000_u32.to_le_bytes());

    push_key_and_type(&mut out, "validation.i32", TYPE_I32);
    out.extend_from_slice(&(-123_456_i32).to_le_bytes());

    push_key_and_type(&mut out, "validation.f32", TYPE_F32);
    out.extend_from_slice(&1.25_f32.to_le_bytes());

    push_key_and_type(&mut out, "validation.bool", TYPE_BOOL);
    out.push(1);

    push_string_entry(&mut out, "validation.string", "hello, GGUF");

    push_key_and_type(&mut out, "validation.array", TYPE_ARRAY);
    out.extend_from_slice(&TYPE_U32.to_le_bytes());
    out.extend_from_slice(&40_u64.to_le_bytes());
    for value in 0_u32..40 {
        out.extend_from_slice(&value.to_le_bytes());
    }

    push_key_and_type(&mut out, "validation.u64", TYPE_U64);
    out.extend_from_slice(&9_000_000_000_u64.to_le_bytes());

    push_key_and_type(&mut out, "validation.i64", TYPE_I64);
    out.extend_from_slice(&(-9_000_000_000_i64).to_le_bytes());

    push_key_and_type(&mut out, "validation.f64", TYPE_F64);
    out.extend_from_slice(&-42.5_f64.to_le_bytes());

    push_key_and_type(&mut out, "validation.nested_array", TYPE_ARRAY);
    out.extend_from_slice(&TYPE_ARRAY.to_le_bytes());
    out.extend_from_slice(&1_u64.to_le_bytes());
    out.extend_from_slice(&TYPE_STRING.to_le_bytes());
    out.extend_from_slice(&2_u64.to_le_bytes());
    push_string(&mut out, "one");
    push_string(&mut out, "two");

    out
}

fn write_fixture(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).expect("fixture should be writable");
}

#[test]
fn inspects_v2_and_v3_metadata_from_contents_on_unicode_space_paths() {
    let temp = tempdir().expect("temporary directory should be created");
    let fixture_dir = temp.path().join("GGUF fixtures ünicode");
    fs::create_dir(&fixture_dir).expect("fixture directory should be created");

    for version in [2_u32, 3_u32] {
        let path = fixture_dir.join(format!("mistral-filename-decoy v{version} α β.gguf"));
        let bytes = build_metadata_fixture(version);
        write_fixture(&path, &bytes);

        let info = inspect_gguf(&path).expect("valid fixture should inspect successfully");
        let expected_sha = hex::encode(Sha256::digest(&bytes));

        assert_eq!(info.path, path);
        assert_eq!(info.gguf_version, version);
        assert_eq!(info.tensor_count, 0);
        assert_eq!(info.metadata_count, 19);
        assert_eq!(info.name.as_deref(), Some("Validation Fixture"));
        assert_eq!(info.architecture.as_deref(), Some("qwen35"));
        assert_eq!(info.context_length, Some(262_144));
        assert_eq!(info.quantization_version, Some(2));
        assert_eq!(info.sha256, expected_sha);
        assert_eq!(info.id, format!("model-{}", &info.sha256[..32]));

        assert!(matches!(
            info.metadata.get("validation.unknown_key"),
            Some(MetadataValue::String(value)) if value == "preserved"
        ));
        assert!(matches!(
            info.metadata.get("validation.u8"),
            Some(MetadataValue::UInt(255))
        ));
        assert!(matches!(
            info.metadata.get("validation.i8"),
            Some(MetadataValue::Int(-8))
        ));
        assert!(matches!(
            info.metadata.get("validation.u16"),
            Some(MetadataValue::UInt(65_530))
        ));
        assert!(matches!(
            info.metadata.get("validation.i16"),
            Some(MetadataValue::Int(-1_234))
        ));
        assert!(matches!(
            info.metadata.get("validation.u32"),
            Some(MetadataValue::UInt(4_000_000_000))
        ));
        assert!(matches!(
            info.metadata.get("validation.i32"),
            Some(MetadataValue::Int(-123_456))
        ));
        assert!(matches!(
            info.metadata.get("validation.bool"),
            Some(MetadataValue::Bool(true))
        ));
        assert!(matches!(
            info.metadata.get("validation.string"),
            Some(MetadataValue::String(value)) if value == "hello, GGUF"
        ));
        assert!(matches!(
            info.metadata.get("validation.u64"),
            Some(MetadataValue::UInt(9_000_000_000))
        ));
        assert!(matches!(
            info.metadata.get("validation.i64"),
            Some(MetadataValue::Int(-9_000_000_000))
        ));

        match info.metadata.get("validation.f32") {
            Some(MetadataValue::Float(value)) => {
                assert!((*value - 1.25).abs() < f64::EPSILON);
            }
            other => panic!("unexpected f32 metadata value: {other:?}"),
        }
        match info.metadata.get("validation.f64") {
            Some(MetadataValue::Float(value)) => {
                assert!((*value + 42.5).abs() < f64::EPSILON);
            }
            other => panic!("unexpected f64 metadata value: {other:?}"),
        }
        match info.metadata.get("validation.array") {
            Some(MetadataValue::Array {
                element_type,
                len,
                preview,
            }) => {
                assert_eq!(*element_type, TYPE_U32);
                assert_eq!(*len, 40);
                assert_eq!(preview.len(), 32);
                assert_eq!(preview.first().map(String::as_str), Some("0"));
                assert_eq!(preview.last().map(String::as_str), Some("31"));
            }
            other => panic!("unexpected array metadata value: {other:?}"),
        }
        match info.metadata.get("validation.nested_array") {
            Some(MetadataValue::Array {
                element_type,
                len,
                preview,
            }) => {
                assert_eq!(*element_type, TYPE_ARRAY);
                assert_eq!(*len, 1);
                assert_eq!(
                    preview,
                    &["array(type=8, len=2) [one, two]".to_string()]
                );
            }
            other => panic!("unexpected nested-array metadata value: {other:?}"),
        }
    }
}

#[test]
fn corrupt_and_truncated_inputs_remain_typed_failures() {
    let temp = tempdir().expect("temporary directory should be created");

    let directory_error = inspect_gguf(temp.path()).expect_err("a directory is not a GGUF file");
    assert!(matches!(directory_error, LlamaManagerError::InvalidPath(_)));

    let bad_magic_path = temp.path().join("bad magic.gguf");
    write_fixture(&bad_magic_path, b"NOPE");
    let bad_magic_error = inspect_gguf(&bad_magic_path).expect_err("bad magic should fail");
    assert!(matches!(bad_magic_error, LlamaManagerError::Gguf(_)));

    let unsupported_version_path = temp.path().join("unsupported version.gguf");
    let mut unsupported_version = Vec::new();
    push_header(&mut unsupported_version, 4, 0);
    write_fixture(&unsupported_version_path, &unsupported_version);
    let unsupported_version_error =
        inspect_gguf(&unsupported_version_path).expect_err("unsupported version should fail");
    assert!(matches!(unsupported_version_error, LlamaManagerError::Gguf(_)));

    let truncated_path = temp.path().join("truncated metadata.gguf");
    let mut truncated = Vec::new();
    push_header(&mut truncated, 3, 1);
    push_key_and_type(&mut truncated, "general.name", TYPE_STRING);
    truncated.extend_from_slice(&5_u64.to_le_bytes());
    truncated.extend_from_slice(b"xy");
    write_fixture(&truncated_path, &truncated);

    let truncated_error = inspect_gguf(&truncated_path).expect_err("truncated value should fail");
    match truncated_error {
        LlamaManagerError::Io(error) => assert_eq!(error.kind(), ErrorKind::UnexpectedEof),
        other => panic!("unexpected truncated-input error: {other:?}"),
    }
}

#[test]
fn declared_metadata_sizes_are_bounded_before_allocation_or_iteration() {
    let temp = tempdir().expect("temporary directory should be created");

    let excessive_count_path = temp.path().join("excessive metadata count.gguf");
    let mut excessive_count = Vec::new();
    push_header(&mut excessive_count, 3, 1_000_001);
    write_fixture(&excessive_count_path, &excessive_count);
    let count_error =
        inspect_gguf(&excessive_count_path).expect_err("metadata count limit should fail");
    assert!(matches!(
        count_error,
        LlamaManagerError::Gguf(message) if message.contains("metadata count")
    ));

    let excessive_string_path = temp.path().join("excessive string.gguf");
    let mut excessive_string = Vec::new();
    push_header(&mut excessive_string, 3, 1);
    let excessive_len = 16_u64 * 1024 * 1024 + 1;
    excessive_string.extend_from_slice(&excessive_len.to_le_bytes());
    write_fixture(&excessive_string_path, &excessive_string);
    let string_error =
        inspect_gguf(&excessive_string_path).expect_err("string allocation limit should fail");
    assert!(matches!(
        string_error,
        LlamaManagerError::Gguf(message) if message.contains("string length")
    ));

    let excessive_array_path = temp.path().join("excessive array.gguf");
    let mut excessive_array = Vec::new();
    push_header(&mut excessive_array, 3, 1);
    push_key_and_type(&mut excessive_array, "validation.array", TYPE_ARRAY);
    excessive_array.extend_from_slice(&TYPE_U8.to_le_bytes());
    excessive_array.extend_from_slice(&100_000_001_u64.to_le_bytes());
    write_fixture(&excessive_array_path, &excessive_array);
    let array_error =
        inspect_gguf(&excessive_array_path).expect_err("array iteration limit should fail");
    assert!(matches!(
        array_error,
        LlamaManagerError::Gguf(message) if message.contains("metadata array length")
    ));
}
