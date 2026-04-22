#![allow(dead_code)]

use mp4forge::FourCc;
use mp4forge::header::BoxInfo;
use mp4forge::walk::BoxPath;

const SAMPLE_MP4: &[u8] = include_bytes!("../../tests/fixtures/sample.mp4");
const SAMPLE_FRAGMENTED_MP4: &[u8] = include_bytes!("../../tests/fixtures/sample_fragmented.mp4");
const SAMPLE_INIT_ENCA_MP4: &[u8] = include_bytes!("../../tests/fixtures/sample_init.enca.mp4");
const SAMPLE_INIT_ENCV_MP4: &[u8] = include_bytes!("../../tests/fixtures/sample_init.encv.mp4");
const SAMPLE_QT_MP4: &[u8] = include_bytes!("../../tests/fixtures/sample_qt.mp4");
const AAC_AUDIO_MP4: &[u8] = include_bytes!("../../tests/fixtures/aac_audio.mp4");
const OPUS_AUDIO_MP4: &[u8] = include_bytes!("../../tests/fixtures/opus_audio.mp4");
const PCM_AUDIO_MP4: &[u8] = include_bytes!("../../tests/fixtures/pcm_audio.mp4");
const VP9_OPUS_MP4: &[u8] = include_bytes!("../../tests/fixtures/vp9_opus.mp4");
const AV1_OPUS_MP4: &[u8] = include_bytes!("../../tests/fixtures/av1_opus.mp4");

const ANY_FIXTURES: [&[u8]; 10] = [
    SAMPLE_MP4,
    SAMPLE_FRAGMENTED_MP4,
    SAMPLE_INIT_ENCA_MP4,
    SAMPLE_INIT_ENCV_MP4,
    SAMPLE_QT_MP4,
    AAC_AUDIO_MP4,
    OPUS_AUDIO_MP4,
    PCM_AUDIO_MP4,
    VP9_OPUS_MP4,
    AV1_OPUS_MP4,
];

const SMALL_FIXTURES: [&[u8]; 6] = [
    SAMPLE_MP4,
    SAMPLE_FRAGMENTED_MP4,
    SAMPLE_INIT_ENCA_MP4,
    SAMPLE_INIT_ENCV_MP4,
    AAC_AUDIO_MP4,
    OPUS_AUDIO_MP4,
];

const REWRITE_FIXTURES: [&[u8]; 4] = [
    SAMPLE_MP4,
    SAMPLE_FRAGMENTED_MP4,
    SAMPLE_INIT_ENCA_MP4,
    SAMPLE_INIT_ENCV_MP4,
];

pub struct FuzzInput<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> FuzzInput<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    pub fn take_u8(&mut self) -> u8 {
        let Some(byte) = self.data.get(self.offset).copied() else {
            return 0;
        };
        self.offset += 1;
        byte
    }

    pub fn take_u16(&mut self) -> u16 {
        u16::from_be_bytes(self.take_exact())
    }

    pub fn take_u32(&mut self) -> u32 {
        u32::from_be_bytes(self.take_exact())
    }

    pub fn take_u64(&mut self) -> u64 {
        u64::from_be_bytes(self.take_exact())
    }

    pub fn take_exact<const N: usize>(&mut self) -> [u8; N] {
        let mut bytes = [0_u8; N];
        for byte in &mut bytes {
            *byte = self.take_u8();
        }
        bytes
    }

    pub fn take_bool(&mut self) -> bool {
        self.take_u8() & 1 != 0
    }

    pub fn take_usize(&mut self, max_inclusive: usize) -> usize {
        if max_inclusive == 0 {
            return 0;
        }
        usize::from(self.take_u8()) % (max_inclusive + 1)
    }

    pub fn take_bytes(&mut self, max_len: usize) -> Vec<u8> {
        let len = self.take_usize(max_len);
        let mut bytes = Vec::with_capacity(len);
        for _ in 0..len {
            bytes.push(self.take_u8());
        }
        bytes
    }

    pub fn take_fourcc(&mut self) -> FourCc {
        FourCc::from_bytes(self.take_exact())
    }

    pub fn take_path(&mut self, max_depth: usize) -> BoxPath {
        let depth = self.take_usize(max_depth);
        let mut parts = Vec::with_capacity(depth);
        for _ in 0..depth {
            parts.push(if self.take_bool() {
                FourCc::ANY
            } else {
                self.take_fourcc()
            });
        }
        BoxPath::from(parts)
    }

    pub fn choose_fourcc(&mut self, table: &[FourCc]) -> FourCc {
        table[self.take_usize(table.len() - 1)]
    }

    pub fn take_path_from_table(&mut self, table: &[BoxPath]) -> BoxPath {
        table[self.take_usize(table.len() - 1)].clone()
    }

    pub fn take_paths_from_table(&mut self, table: &[BoxPath], max_len: usize) -> Vec<BoxPath> {
        let len = self.take_usize(max_len);
        let mut paths = Vec::with_capacity(len);
        for _ in 0..len {
            paths.push(self.take_path_from_table(table));
        }
        paths
    }
}

pub fn seeded_any_mp4_bytes(input: &mut FuzzInput<'_>) -> Vec<u8> {
    seeded_mp4_bytes_from(input, &ANY_FIXTURES, 384 * 1024)
}

pub fn seeded_small_mp4_bytes(input: &mut FuzzInput<'_>) -> Vec<u8> {
    seeded_mp4_bytes_from(input, &SMALL_FIXTURES, 64 * 1024)
}

pub fn seeded_rewrite_mp4_bytes(input: &mut FuzzInput<'_>) -> Vec<u8> {
    seeded_mp4_bytes_from(input, &REWRITE_FIXTURES, 96 * 1024)
}

fn seeded_mp4_bytes_from(input: &mut FuzzInput<'_>, fixtures: &[&[u8]], max_len: usize) -> Vec<u8> {
    let mut bytes = select_seed_bytes(input, fixtures, max_len);
    mutate_seed_bytes(input, &mut bytes, max_len);
    if bytes.is_empty() {
        bytes = malformed_truncated_mvhd_payload();
    }
    bytes
}

fn select_seed_bytes(input: &mut FuzzInput<'_>, fixtures: &[&[u8]], max_len: usize) -> Vec<u8> {
    let malformed_seed_count = 4;
    match input.take_usize(fixtures.len() + malformed_seed_count) {
        index if index < fixtures.len() => fixtures[index].to_vec(),
        index if index == fixtures.len() => malformed_truncated_child_header(),
        index if index == fixtures.len() + 1 => malformed_huge_supported_payload(),
        index if index == fixtures.len() + 2 => malformed_oversized_child_box(),
        index if index == fixtures.len() + 3 => malformed_truncated_mvhd_payload(),
        _ => input.take_bytes(max_len.min(4096)),
    }
}

fn mutate_seed_bytes(input: &mut FuzzInput<'_>, bytes: &mut Vec<u8>, max_len: usize) {
    let steps = input.take_usize(8);
    for _ in 0..steps {
        match input.take_u8() % 6 {
            0 => {
                if !bytes.is_empty() {
                    let index = input.take_usize(bytes.len() - 1);
                    bytes[index] ^= input.take_u8();
                }
            }
            1 => {
                if bytes.len() < max_len {
                    let index = input.take_usize(bytes.len());
                    bytes.insert(index, input.take_u8());
                }
            }
            2 => {
                if !bytes.is_empty() {
                    let index = input.take_usize(bytes.len() - 1);
                    bytes.remove(index);
                }
            }
            3 => {
                if !bytes.is_empty() {
                    let truncate_to = input.take_usize(bytes.len() - 1);
                    bytes.truncate(truncate_to);
                }
            }
            4 => {
                let available = max_len.saturating_sub(bytes.len()).min(32);
                if available != 0 {
                    bytes.extend(input.take_bytes(available));
                }
            }
            _ => {
                if bytes.len() >= 2 {
                    let lhs = input.take_usize(bytes.len() - 1);
                    let rhs = input.take_usize(bytes.len() - 1);
                    bytes.swap(lhs, rhs);
                }
            }
        }
    }

    if bytes.len() > max_len {
        bytes.truncate(max_len);
    }
}

fn malformed_truncated_child_header() -> Vec<u8> {
    let mut bytes = BoxInfo::new(FourCc::from_bytes(*b"moov"), 16).encode();
    bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x0c]);
    bytes
}

fn malformed_huge_supported_payload() -> Vec<u8> {
    let mut bytes = BoxInfo::new(FourCc::from_bytes(*b"styp"), u64::from(u32::MAX)).encode();
    bytes.extend_from_slice(b"isom");
    bytes.extend_from_slice(&0_u32.to_be_bytes());
    bytes
}

fn malformed_oversized_child_box() -> Vec<u8> {
    let mut bytes = BoxInfo::new(FourCc::from_bytes(*b"moov"), 16).encode();
    bytes.extend_from_slice(&BoxInfo::new(FourCc::from_bytes(*b"free"), 12).encode());
    bytes
}

fn malformed_truncated_mvhd_payload() -> Vec<u8> {
    let mut bytes = BoxInfo::new(FourCc::from_bytes(*b"mvhd"), 12).encode();
    bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    bytes
}
