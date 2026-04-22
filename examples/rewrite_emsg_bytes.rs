use std::env;
use std::error::Error;
use std::fs;

use mp4forge::FourCc;
use mp4forge::boxes::iso14496_12::Emsg;
use mp4forge::rewrite::rewrite_box_as_bytes;
use mp4forge::walk::BoxPath;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let Some(output_path) = env::args().nth(1) else {
        return Err("usage: cargo run --example rewrite_emsg_bytes -- <output.mp4>".into());
    };

    let input = sample_emsg_file();
    let output = rewrite_box_as_bytes::<Emsg, _>(
        &input,
        BoxPath::from([FourCc::from_bytes(*b"emsg")]),
        |emsg| {
            emsg.message_data = b"hello world".to_vec();
        },
    )?;
    fs::write(output_path, output)?;

    Ok(())
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
