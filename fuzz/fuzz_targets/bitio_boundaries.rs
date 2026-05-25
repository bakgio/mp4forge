#![no_main]

mod support;

use std::io::{Cursor, Read, Seek, SeekFrom, Write};

use libfuzzer_sys::fuzz_target;
use mp4forge::bitio::{BitReader, BitWriter};

use support::FuzzInput;

const MAX_BYTES: usize = 512;
const MAX_WIDTH: usize = 128;

fuzz_target!(|data: &[u8]| {
    let mut input = FuzzInput::new(data);
    let bytes = input.take_bytes(MAX_BYTES);

    exercise_reader(&mut input, &bytes);
    exercise_writer(&mut input);
    exercise_write_then_read(&mut input);
});

fn exercise_reader(input: &mut FuzzInput<'_>, bytes: &[u8]) {
    let mut reader = BitReader::new(Cursor::new(bytes));
    for _ in 0..input.take_usize(32) {
        match input.take_u8() % 5 {
            0 => {
                let _ = reader.read_bit();
            }
            1 => {
                let _ = reader.read_bits(input.take_usize(MAX_WIDTH));
            }
            2 => {
                let mut buf = vec![0_u8; input.take_usize(32)];
                let _ = reader.read(&mut buf);
            }
            3 => {
                let _ = reader.seek(SeekFrom::Start(input.take_u64() % 1024));
            }
            _ => {
                let offset = i64::from(input.take_u8()) - 128;
                let _ = reader.seek(SeekFrom::Current(offset));
            }
        }
    }
}

fn exercise_writer(input: &mut FuzzInput<'_>) {
    let mut writer = BitWriter::new(Cursor::new(Vec::new()));
    for _ in 0..input.take_usize(32) {
        match input.take_u8() % 4 {
            0 => {
                let _ = writer.write_bit(input.take_bool());
            }
            1 => {
                let bits = input.take_bytes(32);
                let width = input.take_usize(bits.len().saturating_mul(8).saturating_add(8));
                let _ = writer.write_bits(&bits, width);
            }
            2 => {
                let bytes = input.take_bytes(32);
                let _ = writer.write(&bytes);
            }
            _ => {
                let _ = writer.flush();
            }
        }
    }
    let _ = writer.into_inner();
}

fn exercise_write_then_read(input: &mut FuzzInput<'_>) {
    let bits = input.take_bytes(32);
    let width = input.take_usize(bits.len().saturating_mul(8));
    let mut writer = BitWriter::new(Vec::new());
    if writer.write_bits(&bits, width).is_ok()
        && writer.is_aligned()
        && let Ok(encoded) = writer.into_inner()
    {
        let mut reader = BitReader::new(Cursor::new(encoded));
        let _ = reader.read_bits(width);
    }
}
