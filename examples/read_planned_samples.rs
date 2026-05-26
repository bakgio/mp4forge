#[cfg(feature = "mux")]
fn main() {
    use std::io::Cursor;

    use mp4forge::mux::sample_reader::PlannedSampleReader;
    use mp4forge::mux::{MuxInterleavePolicy, MuxStagedMediaItem, plan_staged_media_items};

    let plan = plan_staged_media_items(
        vec![
            MuxStagedMediaItem::new(0, 1, 0, 1024, 4, 5).with_sync_sample(true),
            MuxStagedMediaItem::new(1, 2, 512, 512, 4, 4),
        ],
        MuxInterleavePolicy::DecodeTime,
    )
    .unwrap();

    let mut sources = [
        Cursor::new(b"HEADvideoTAIL".to_vec()),
        Cursor::new(b"PREMaudPOST".to_vec()),
    ];
    let mut reader = PlannedSampleReader::new(&mut sources, &plan);
    let mut sample_bytes = Vec::new();

    while let Some(metadata) = reader.next_sample_into(&mut sample_bytes).unwrap() {
        println!(
            "track {} at output {} -> {} bytes",
            metadata.track_id(),
            metadata.output_offset(),
            sample_bytes.len()
        );
    }
}

#[cfg(not(feature = "mux"))]
fn main() {
    eprintln!("enable the `mux` feature to run this example");
}
