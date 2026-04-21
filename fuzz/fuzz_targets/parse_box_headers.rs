#![no_main]

use std::io::Cursor;

use libfuzzer_sys::fuzz_target;
use mp4forge::BoxInfo;

fuzz_target!(|data: &[u8]| {
    let mut cursor = Cursor::new(data);
    if let Ok(info) = BoxInfo::read(&mut cursor) {
        let _ = info.payload_size();
        let encoded = info.encode();

        let mut encoded_cursor = Cursor::new(encoded);
        let _ = BoxInfo::read(&mut encoded_cursor);
        let _ = info.seek_to_start(&mut encoded_cursor);
        let _ = info.seek_to_payload(&mut encoded_cursor);
        let _ = info.seek_to_end(&mut encoded_cursor);

        let mut written = Cursor::new(Vec::new());
        if let Ok(normalized) = info.write(&mut written) {
            let mut reread = Cursor::new(written.into_inner());
            let _ = BoxInfo::read(&mut reread);
            let _ = normalized.seek_to_start(&mut reread);
            let _ = normalized.seek_to_payload(&mut reread);
            let _ = normalized.seek_to_end(&mut reread);
        }
    }
});
