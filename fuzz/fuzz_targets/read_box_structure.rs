#![no_main]

use std::io::Cursor;

use libfuzzer_sys::fuzz_target;
use mp4forge::walk::{WalkControl, walk_structure};

fuzz_target!(|data: &[u8]| {
    let mut reader = Cursor::new(data);
    let _ = walk_structure(&mut reader, |handle| {
        if !handle.is_supported_type() {
            return Ok(WalkControl::Continue);
        }

        if handle.read_payload().is_ok() {
            Ok(WalkControl::Descend)
        } else {
            Ok(WalkControl::Continue)
        }
    });
});
