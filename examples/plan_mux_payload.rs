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

    println!("planned {} items", plan.planned_items().len());
    println!("payload bytes: {}", plan.total_payload_size());
}

#[cfg(not(feature = "mux"))]
fn main() {
    eprintln!("enable the `mux` feature to run this example");
}
