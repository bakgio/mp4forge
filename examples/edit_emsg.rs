use std::env;
use std::error::Error;
use std::fs::File;
use std::io::{self, Cursor};

use mp4forge::FourCc;
use mp4forge::boxes::iso14496_12::Emsg;
use mp4forge::codec::marshal_dyn;
use mp4forge::walk::{WalkControl, WalkError, walk_structure};
use mp4forge::writer::Writer;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let Some(output_path) = env::args().nth(1) else {
        return Err("usage: cargo run --example edit_emsg -- <output.mp4>".into());
    };

    let input = sample_emsg_file();
    let mut walk_reader = Cursor::new(input.clone());
    let mut copy_reader = Cursor::new(input);
    let output = File::create(output_path)?;
    let mut writer = Writer::new(output);

    walk_structure(&mut walk_reader, |handle| {
        if handle.info().box_type() == FourCc::from_bytes(*b"emsg") {
            writer.start_box(*handle.info()).map_err(walk_error)?;
            let (mut payload, _) = handle.read_payload()?;
            let Some(emsg) = payload.as_any_mut().downcast_mut::<Emsg>() else {
                return Err(walk_error("expected emsg payload"));
            };
            emsg.message_data = b"hello world".to_vec();
            marshal_dyn(&mut writer, payload.as_ref(), None)?;
            writer.end_box().map_err(walk_error)?;
        } else {
            writer
                .copy_box(&mut copy_reader, handle.info())
                .map_err(walk_error)?;
        }
        Ok(WalkControl::Continue)
    })?;

    Ok(())
}

fn walk_error(error: impl ToString) -> WalkError {
    WalkError::from(io::Error::other(error.to_string()))
}

fn sample_emsg_file() -> Vec<u8> {
    let mut emsg_payload = vec![0x00, 0x00, 0x00, 0x00];
    append_null_terminated_string(&mut emsg_payload, "urn:test");
    append_null_terminated_string(&mut emsg_payload, "demo");
    append_u32(&mut emsg_payload, 1000);
    append_u32(&mut emsg_payload, 0);
    append_u32(&mut emsg_payload, 5);
    append_u32(&mut emsg_payload, 1);
    emsg_payload.extend_from_slice(b"hello");

    let mut file = Vec::new();
    file.extend_from_slice(&box_bytes("free", &[0x01, 0x02, 0x03]));
    file.extend_from_slice(&box_bytes("emsg", &emsg_payload));
    file.extend_from_slice(&box_bytes("free", &[0x04, 0x05]));
    file
}

fn append_null_terminated_string(dst: &mut Vec<u8>, value: &str) {
    dst.extend_from_slice(value.as_bytes());
    dst.push(0x00);
}

fn append_u32(dst: &mut Vec<u8>, value: u32) {
    dst.extend_from_slice(&value.to_be_bytes());
}

fn box_bytes(box_type: &str, payload: &[u8]) -> Vec<u8> {
    let mut box_bytes = Vec::with_capacity(8 + payload.len());
    box_bytes.extend_from_slice(&((payload.len() + 8) as u32).to_be_bytes());
    box_bytes.extend_from_slice(box_type.as_bytes());
    box_bytes.extend_from_slice(payload);
    box_bytes
}
