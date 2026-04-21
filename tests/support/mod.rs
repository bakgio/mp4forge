#![allow(dead_code)]
#![allow(clippy::field_reassign_with_default)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use mp4forge::codec::{CodecBox, marshal};
use mp4forge::{BoxInfo, FourCc};

pub fn encode_supported_box<B>(box_value: &B, children: &[u8]) -> Vec<u8>
where
    B: CodecBox,
{
    let mut payload = Vec::new();
    marshal(&mut payload, box_value, None).unwrap();
    payload.extend_from_slice(children);
    encode_raw_box(box_value.box_type(), &payload)
}

pub fn encode_raw_box(box_type: FourCc, payload: &[u8]) -> Vec<u8> {
    let info = BoxInfo::new(box_type, 8 + payload.len() as u64);
    let mut bytes = info.encode();
    bytes.extend_from_slice(payload);
    bytes
}

pub fn fourcc(value: &str) -> FourCc {
    FourCc::try_from(value).unwrap()
}

pub fn write_temp_file(prefix: &str, data: &[u8]) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "mp4forge-{prefix}-{}-{unique}.mp4",
        std::process::id()
    ));
    fs::write(&path, data).unwrap();
    path
}

pub fn temp_output_dir(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("mp4forge-{prefix}-{}-{unique}", std::process::id()))
}

pub fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

pub fn read_text(path: &Path) -> String {
    normalize_text(&fs::read_to_string(path).unwrap())
}

pub fn read_golden(relative_path: &str) -> String {
    read_text(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("golden")
            .join(relative_path),
    )
}

pub fn normalize_text(value: &str) -> String {
    value.replace("\r\n", "\n")
}
