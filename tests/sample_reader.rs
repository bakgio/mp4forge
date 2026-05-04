#![cfg(feature = "mux")]

use std::io::Cursor;

use mp4forge::mux::sample_reader::{
    AsyncPlannedSampleReader, AsyncProgressiveSampleReader, PlannedSampleReader,
    ProgressiveSampleReader, SampleReaderError,
};
use mp4forge::mux::{
    MuxInterleavePolicy, MuxStagedMediaItem, MuxTrackConfig, MuxTrackKind, plan_staged_media_items,
};

#[cfg(feature = "async")]
use tokio::io::AsyncWriteExt;

#[test]
fn planned_sample_reader_reads_seekable_samples_in_output_order() {
    let mut sources = [
        Cursor::new(b"AAAAhelloBBBBxy".to_vec()),
        Cursor::new(b"zzzzSYNCtail".to_vec()),
    ];
    let plan = plan_staged_media_items(
        vec![
            MuxStagedMediaItem::new(0, 2, 10, 4, 13, 2),
            MuxStagedMediaItem::new(1, 1, 0, 5, 4, 4).with_sync_sample(true),
            MuxStagedMediaItem::new(0, 2, 0, 4, 4, 5).with_composition_time_offset(2),
        ],
        MuxInterleavePolicy::DecodeTime,
    )
    .unwrap();

    let mut reader = PlannedSampleReader::new(&mut sources, &plan);

    let first = reader.next_sample().unwrap().unwrap();
    assert_eq!(first.bytes(), b"SYNC");
    assert_eq!(first.metadata().track_id(), 1);
    assert_eq!(first.metadata().output_offset(), 0);
    assert_eq!(first.metadata().output_end_offset(), 4);
    assert_eq!(first.metadata().decode_end_time(), 5);
    assert!(first.metadata().is_sync_sample());

    let second = reader.next_sample().unwrap().unwrap();
    assert_eq!(second.bytes(), b"hello");
    assert_eq!(second.metadata().track_id(), 2);
    assert_eq!(second.metadata().composition_time_offset(), 2);
    assert_eq!(second.metadata().output_offset(), 4);
    assert_eq!(second.metadata().output_end_offset(), 9);
    assert_eq!(second.metadata().decode_end_time(), 4);

    let third = reader.next_sample().unwrap().unwrap();
    assert_eq!(third.bytes(), b"xy");
    assert_eq!(third.metadata().track_id(), 2);
    assert_eq!(third.metadata().output_offset(), 9);
    assert_eq!(third.metadata().output_end_offset(), 11);
    assert_eq!(third.metadata().decode_end_time(), 14);

    assert!(reader.next_sample().unwrap().is_none());
}

#[test]
fn progressive_sample_reader_reads_non_seekable_samples_in_output_order() {
    let mut first_source: &[u8] = b"AAAAhelloBBBBxy";
    let mut second_source: &[u8] = b"zzzzSYNCtail";
    let mut sources = [&mut first_source, &mut second_source];
    let plan = plan_staged_media_items(
        vec![
            MuxStagedMediaItem::new(0, 1, 0, 4, 4, 5),
            MuxStagedMediaItem::new(1, 2, 5, 4, 4, 4).with_sync_sample(true),
            MuxStagedMediaItem::new(0, 1, 10, 4, 13, 2),
        ],
        MuxInterleavePolicy::DecodeTime,
    )
    .unwrap();

    let mut reader = ProgressiveSampleReader::new(&mut sources, &plan);

    let first = reader.next_sample().unwrap().unwrap();
    assert_eq!(first.bytes(), b"hello");
    assert_eq!(first.metadata().source_index(), 0);

    let second = reader.next_sample().unwrap().unwrap();
    assert_eq!(second.bytes(), b"SYNC");
    assert_eq!(second.metadata().source_index(), 1);
    assert!(second.metadata().is_sync_sample());

    let third = reader.next_sample().unwrap().unwrap();
    assert_eq!(third.bytes(), b"xy");
    assert_eq!(third.metadata().source_index(), 0);

    assert!(reader.next_sample().unwrap().is_none());
}

#[test]
fn progressive_sample_reader_rejects_backward_offsets() {
    let mut source: &[u8] = b"AAAAhelloBBBBxy";
    let mut sources = [&mut source];
    let plan = plan_staged_media_items(
        vec![
            MuxStagedMediaItem::new(0, 1, 0, 4, 13, 2),
            MuxStagedMediaItem::new(0, 1, 10, 4, 4, 5),
        ],
        MuxInterleavePolicy::DecodeTime,
    )
    .unwrap();

    let mut reader = ProgressiveSampleReader::new(&mut sources, &plan);

    let first = reader.next_sample().unwrap().unwrap();
    assert_eq!(first.bytes(), b"xy");

    let error = reader.next_sample().unwrap_err();
    assert_eq!(
        error.to_string(),
        "source index 0 would need to move backward from offset 15 to 4"
    );
    assert!(matches!(
        error,
        SampleReaderError::NonMonotonicSourceOffset {
            source_index: 0,
            previous_offset: 15,
            next_offset: 4,
        }
    ));
}

#[test]
fn planned_sample_reader_exposes_text_track_identity_when_track_configs_are_supplied() {
    let mut sources = [
        Cursor::new(b"AAAAwvttBBBBstpp".to_vec()),
        Cursor::new(b"zzzzcaptiontail".to_vec()),
    ];
    let plan = plan_staged_media_items(
        vec![
            MuxStagedMediaItem::new(0, 1, 0, 4, 4, 4).with_sync_sample(true),
            MuxStagedMediaItem::new(0, 2, 10, 4, 12, 4).with_sync_sample(true),
            MuxStagedMediaItem::new(1, 3, 20, 4, 4, 7),
        ],
        MuxInterleavePolicy::DecodeTime,
    )
    .unwrap();
    let track_configs = [
        MuxTrackConfig::new_text(1, 1_000, 0, 0, Vec::new()).with_language(*b"eng"),
        MuxTrackConfig::new_subtitle(2, 1_000, 0, 0, Vec::new()).with_language(*b"fra"),
    ];

    let mut reader =
        PlannedSampleReader::new_with_track_configs(&mut sources, &plan, &track_configs);

    let first = reader.next_sample().unwrap().unwrap();
    assert_eq!(first.bytes(), b"wvtt");
    assert_eq!(
        first.metadata().track().map(|track| track.kind()),
        Some(MuxTrackKind::Text)
    );
    assert_eq!(
        first.metadata().track().map(|track| track.language()),
        Some(*b"eng")
    );

    let second = reader.next_sample().unwrap().unwrap();
    assert_eq!(second.bytes(), b"stpp");
    assert_eq!(
        second.metadata().track().map(|track| track.kind()),
        Some(MuxTrackKind::Subtitle)
    );
    assert_eq!(
        second.metadata().track().map(|track| track.language()),
        Some(*b"fra")
    );

    let third = reader.next_sample().unwrap().unwrap();
    assert_eq!(third.bytes(), b"caption");
    assert_eq!(third.metadata().track(), None);
    assert!(reader.next_sample().unwrap().is_none());
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_planned_sample_reader_exposes_text_track_identity_when_track_configs_are_supplied() {
    let mut sources = [
        Cursor::new(b"AAAAwvttBBBBstpp".to_vec()),
        Cursor::new(b"zzzzcaptiontail".to_vec()),
    ];
    let plan = plan_staged_media_items(
        vec![
            MuxStagedMediaItem::new(0, 1, 0, 4, 4, 4).with_sync_sample(true),
            MuxStagedMediaItem::new(0, 2, 10, 4, 12, 4).with_sync_sample(true),
            MuxStagedMediaItem::new(1, 3, 20, 4, 4, 7),
        ],
        MuxInterleavePolicy::DecodeTime,
    )
    .unwrap();
    let track_configs = [
        MuxTrackConfig::new_text(1, 1_000, 0, 0, Vec::new()).with_language(*b"eng"),
        MuxTrackConfig::new_subtitle(2, 1_000, 0, 0, Vec::new()).with_language(*b"fra"),
    ];

    let mut reader =
        AsyncPlannedSampleReader::new_with_track_configs(&mut sources, &plan, &track_configs);

    let first = reader.next_sample().await.unwrap().unwrap();
    assert_eq!(first.bytes(), b"wvtt");
    assert_eq!(
        first.metadata().track().map(|track| track.kind()),
        Some(MuxTrackKind::Text)
    );
    assert_eq!(
        first.metadata().track().map(|track| track.language()),
        Some(*b"eng")
    );

    let second = reader.next_sample().await.unwrap().unwrap();
    assert_eq!(second.bytes(), b"stpp");
    assert_eq!(
        second.metadata().track().map(|track| track.kind()),
        Some(MuxTrackKind::Subtitle)
    );
    assert_eq!(
        second.metadata().track().map(|track| track.language()),
        Some(*b"fra")
    );

    let third = reader.next_sample().await.unwrap().unwrap();
    assert_eq!(third.bytes(), b"caption");
    assert_eq!(third.metadata().track(), None);
    assert!(reader.next_sample().await.unwrap().is_none());
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_planned_sample_reader_reads_seekable_samples_in_output_order() {
    let mut sources = [
        Cursor::new(b"AAAAhelloBBBBxy".to_vec()),
        Cursor::new(b"zzzzSYNCtail".to_vec()),
    ];
    let plan = plan_staged_media_items(
        vec![
            MuxStagedMediaItem::new(0, 2, 10, 4, 13, 2),
            MuxStagedMediaItem::new(1, 1, 0, 5, 4, 4).with_sync_sample(true),
            MuxStagedMediaItem::new(0, 2, 0, 4, 4, 5).with_composition_time_offset(2),
        ],
        MuxInterleavePolicy::DecodeTime,
    )
    .unwrap();

    let mut reader = AsyncPlannedSampleReader::new(&mut sources, &plan);

    assert_eq!(
        reader.next_sample().await.unwrap().unwrap().bytes(),
        b"SYNC"
    );
    assert_eq!(
        reader.next_sample().await.unwrap().unwrap().bytes(),
        b"hello"
    );
    assert_eq!(reader.next_sample().await.unwrap().unwrap().bytes(), b"xy");
    assert!(reader.next_sample().await.unwrap().is_none());
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_progressive_sample_reader_reads_non_seekable_samples_in_output_order() {
    let (mut first_writer, first_source) = tokio::io::duplex(64);
    let (mut second_writer, second_source) = tokio::io::duplex(64);
    first_writer.write_all(b"AAAAhelloBBBBxy").await.unwrap();
    first_writer.shutdown().await.unwrap();
    second_writer.write_all(b"zzzzSYNCtail").await.unwrap();
    second_writer.shutdown().await.unwrap();

    let plan = plan_staged_media_items(
        vec![
            MuxStagedMediaItem::new(0, 1, 0, 4, 4, 5),
            MuxStagedMediaItem::new(1, 2, 5, 4, 4, 4).with_sync_sample(true),
            MuxStagedMediaItem::new(0, 1, 10, 4, 13, 2),
        ],
        MuxInterleavePolicy::DecodeTime,
    )
    .unwrap();

    let mut sources = [first_source, second_source];
    let mut reader = AsyncProgressiveSampleReader::new(&mut sources, &plan);

    assert_eq!(
        reader.next_sample().await.unwrap().unwrap().bytes(),
        b"hello"
    );
    assert_eq!(
        reader.next_sample().await.unwrap().unwrap().bytes(),
        b"SYNC"
    );
    assert_eq!(reader.next_sample().await.unwrap().unwrap().bytes(), b"xy");
    assert!(reader.next_sample().await.unwrap().is_none());
}
