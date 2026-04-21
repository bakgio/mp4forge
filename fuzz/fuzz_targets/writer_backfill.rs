#![no_main]

mod support;

use std::io::{self, Cursor, Seek, SeekFrom, Write};

use libfuzzer_sys::fuzz_target;
use mp4forge::FourCc;
use mp4forge::header::BoxInfo;
use mp4forge::walk::{WalkControl, walk_structure};
use mp4forge::writer::Writer;

use support::FuzzInput;

const BOX_TYPES: [FourCc; 8] = [
    FourCc::from_bytes(*b"free"),
    FourCc::from_bytes(*b"skip"),
    FourCc::from_bytes(*b"moov"),
    FourCc::from_bytes(*b"trak"),
    FourCc::from_bytes(*b"udta"),
    FourCc::from_bytes(*b"moof"),
    FourCc::from_bytes(*b"traf"),
    FourCc::from_bytes(*b"mdat"),
];

fuzz_target!(|data: &[u8]| {
    let mut input = FuzzInput::new(data);
    if input.take_bool() {
        fuzz_nested_writer(&mut input);
    } else {
        fuzz_large_seek_close(&mut input);
    }
});

fn fuzz_nested_writer(input: &mut FuzzInput<'_>) {
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    for _ in 0..(1 + input.take_usize(32)) {
        match input.take_u8() % 4 {
            0 => {
                let _ = writer.start_box_type(input.choose_fourcc(&BOX_TYPES));
            }
            1 => {
                let chunk = input.take_bytes(32);
                let _ = writer.write_all(&chunk);
            }
            2 => {
                let _ = writer.end_box();
            }
            _ => {
                let source = build_source_box(input);
                let mut reader = Cursor::new(source.bytes);
                let _ = writer.copy_box(&mut reader, &source.info);
            }
        }
    }

    while writer.end_box().is_ok() {}

    let bytes = writer.into_inner().into_inner();
    let _ = walk_structure(&mut Cursor::new(bytes), |handle| {
        Ok(
            if matches!(
                handle.info().box_type(),
                box_type
                    if box_type == FourCc::from_bytes(*b"moov")
                        || box_type == FourCc::from_bytes(*b"trak")
                        || box_type == FourCc::from_bytes(*b"udta")
                        || box_type == FourCc::from_bytes(*b"moof")
                        || box_type == FourCc::from_bytes(*b"traf")
            ) {
                WalkControl::Descend
            } else {
                WalkControl::Continue
            },
        )
    });
}

fn fuzz_large_seek_close(input: &mut FuzzInput<'_>) {
    let mut writer = Writer::new(SparseBuffer::default());
    if writer
        .start_box_type(input.choose_fourcc(&BOX_TYPES))
        .is_err()
    {
        return;
    }

    let far_offset = u64::from(u32::MAX) + 1 + u64::from(input.take_u16());
    let _ = writer.seek(SeekFrom::Start(far_offset));
    let _ = writer.end_box();
}

struct SourceBox {
    bytes: Vec<u8>,
    info: BoxInfo,
}

fn build_source_box(input: &mut FuzzInput<'_>) -> SourceBox {
    let box_type = input.choose_fourcc(&BOX_TYPES);
    let payload = input.take_bytes(32);
    let bytes = encode_raw_box(box_type, &payload);
    let inflate = input.take_bool();
    let extra = u64::from(input.take_u8() % 4);
    let info = BoxInfo::new(
        box_type,
        (bytes.len() as u64).saturating_add(if inflate { extra } else { 0 }),
    );

    SourceBox { bytes, info }
}

fn encode_raw_box(box_type: FourCc, payload: &[u8]) -> Vec<u8> {
    let info = BoxInfo::new(box_type, 8 + payload.len() as u64);
    let mut bytes = info.encode();
    bytes.extend_from_slice(payload);
    bytes
}

#[derive(Default)]
struct SparseBuffer {
    position: u64,
    len: u64,
}

impl Write for SparseBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.position = self
            .position
            .checked_add(buf.len() as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "position overflow"))?;
        self.len = self.len.max(self.position);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for SparseBuffer {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let next = match pos {
            SeekFrom::Start(offset) => i128::from(offset),
            SeekFrom::End(offset) => i128::from(self.len) + i128::from(offset),
            SeekFrom::Current(offset) => i128::from(self.position) + i128::from(offset),
        };

        if !(0..=i128::from(u64::MAX)).contains(&next) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid seek"));
        }

        let next = next as u64;
        self.position = next;
        self.len = self.len.max(next);
        Ok(next)
    }
}
