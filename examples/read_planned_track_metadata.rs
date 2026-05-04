#[cfg(feature = "mux")]
fn main() {
    use std::io::Cursor;

    use mp4forge::mux::sample_reader::PlannedSampleReader;
    use mp4forge::mux::{
        MuxInterleavePolicy, MuxStagedMediaItem, MuxTrackConfig, plan_staged_media_items,
    };

    let plan = plan_staged_media_items(
        vec![MuxStagedMediaItem::new(0, 7, 0, 1_000, 4, 4).with_sync_sample(true)],
        MuxInterleavePolicy::DecodeTime,
    )
    .unwrap();
    let track_configs = [MuxTrackConfig::new_text(7, 1_000, 0, 0, Vec::new())
        .with_language(*b"eng")
        .with_handler_name("CaptionTrack")];
    let mut sources = [Cursor::new(b"HEADwvttTAIL".to_vec())];
    let mut reader =
        PlannedSampleReader::new_with_track_configs(&mut sources, &plan, &track_configs);

    while let Some(sample) = reader.next_sample().unwrap() {
        let track = sample.metadata().track().unwrap();
        println!(
            "track {} {:?} {} at output {} -> {} bytes",
            sample.metadata().track_id(),
            track.kind(),
            std::str::from_utf8(&track.language()).unwrap(),
            sample.metadata().output_offset(),
            sample.bytes().len()
        );
    }
}

#[cfg(not(feature = "mux"))]
fn main() {
    eprintln!("enable the `mux` feature to run this example");
}
