#[cfg(feature = "mux")]
fn main() {
    use mp4forge::mux::{MuxInterleavePolicy, MuxStagedMediaItem, plan_staged_media_items};

    let plan = plan_staged_media_items(
        vec![
            MuxStagedMediaItem::new(0, 1, 0, 1024, 4096, 2048).with_sync_sample(true),
            MuxStagedMediaItem::new(1, 2, 512, 512, 2048, 1024),
        ],
        MuxInterleavePolicy::DecodeTime,
    )
    .unwrap();

    for item in plan.planned_items() {
        println!(
            "track {} decode [{}..{}) output [{}..{})",
            item.staged().track_id(),
            item.staged().decode_time(),
            item.decode_end_time(),
            item.output_offset(),
            item.output_end_offset()
        );
    }
}

#[cfg(not(feature = "mux"))]
fn main() {
    eprintln!("enable the `mux` feature to run this example");
}
