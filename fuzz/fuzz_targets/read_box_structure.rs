#![no_main]

use std::io::Cursor;

use libfuzzer_sys::fuzz_target;
use mp4forge::walk::{WalkControl, WalkError, walk_structure};

const MAX_VISITED_BOXES: usize = 1024;
const MAX_DECODED_PAYLOAD: u64 = 1024;

fuzz_target!(|data: &[u8]| {
    let mut reader = Cursor::new(data);
    let input_len = data.len() as u64;
    let mut visited = 0usize;

    let _ = walk_structure(&mut reader, |handle| {
        visited += 1;
        if visited > MAX_VISITED_BOXES {
            return Err(WalkError::UnexpectedEof);
        }

        if !handle.is_supported_type() {
            return Ok(WalkControl::Continue);
        }

        let Ok(payload_size) = handle.info().payload_size() else {
            return Ok(WalkControl::Continue);
        };
        if handle.info().size() > input_len || payload_size > MAX_DECODED_PAYLOAD {
            return Ok(WalkControl::Continue);
        }

        if handle.read_payload().is_ok() {
            Ok(WalkControl::Descend)
        } else {
            Ok(WalkControl::Continue)
        }
    });
});
