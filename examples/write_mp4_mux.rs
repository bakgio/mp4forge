#[cfg(feature = "mux")]
use std::io::Cursor;

#[cfg(feature = "mux")]
use mp4forge::boxes::iso14496_12::{AudioSampleEntry, SampleEntry, VisualSampleEntry};
#[cfg(feature = "mux")]
use mp4forge::codec::{CodecBox, marshal};
#[cfg(feature = "mux")]
use mp4forge::mux::{
    MuxFileConfig, MuxInterleavePolicy, MuxStagedMediaItem, MuxTrackConfig,
    plan_staged_media_items, write_mp4_mux,
};
#[cfg(feature = "mux")]
use mp4forge::{BoxInfo, FourCc};

#[cfg(not(feature = "mux"))]
fn main() {}

#[cfg(feature = "mux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut sources = [
        Cursor::new(b"AAAAhelloBBBBxy".to_vec()),
        Cursor::new(b"zzzzSYNCtail".to_vec()),
    ];
    let plan = plan_staged_media_items(
        vec![
            MuxStagedMediaItem::new(0, 2, 10, 4, 13, 2),
            MuxStagedMediaItem::new(1, 1, 0, 5, 4, 4).with_sync_sample(true),
            MuxStagedMediaItem::new(0, 2, 0, 4, 4, 5)
                .with_composition_time_offset(2)
                .with_sync_sample(true),
        ],
        MuxInterleavePolicy::DecodeTime,
    )?;
    let file_config = MuxFileConfig::new(1_000)
        .with_major_brand(fourcc("isom"))
        .with_compatible_brand(fourcc("mp42"));
    let track_configs = vec![
        MuxTrackConfig::new_audio(1, 1_000, audio_sample_entry_box()?),
        MuxTrackConfig::new_video(2, 1_000, 640, 360, video_sample_entry_box()?),
    ];

    let mut output = Cursor::new(Vec::new());
    write_mp4_mux(
        &mut sources,
        &mut output,
        &file_config,
        &track_configs,
        &plan,
    )?;

    println!(
        "built one MP4 file with {} bytes and {} planned samples",
        output.get_ref().len(),
        plan.planned_items().len()
    );
    Ok(())
}

#[cfg(feature = "mux")]
fn audio_sample_entry_box() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    encode_typed_box(
        &AudioSampleEntry {
            sample_entry: SampleEntry {
                box_type: fourcc("mp4a"),
                data_reference_index: 1,
            },
            channel_count: 2,
            sample_size: 16,
            sample_rate: 48_000_u32 << 16,
            ..AudioSampleEntry::default()
        },
        &[],
    )
}

#[cfg(feature = "mux")]
fn video_sample_entry_box() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    encode_typed_box(
        &VisualSampleEntry {
            sample_entry: SampleEntry {
                box_type: fourcc("avc1"),
                data_reference_index: 1,
            },
            width: 640,
            height: 360,
            horizresolution: 72_u32 << 16,
            vertresolution: 72_u32 << 16,
            frame_count: 1,
            depth: 0x0018,
            pre_defined3: -1,
            ..VisualSampleEntry::default()
        },
        &[],
    )
}

#[cfg(feature = "mux")]
fn encode_typed_box<B>(
    box_value: &B,
    children: &[u8],
) -> Result<Vec<u8>, Box<dyn std::error::Error>>
where
    B: CodecBox,
{
    let mut payload = Vec::new();
    marshal(&mut payload, box_value, None)?;
    payload.extend_from_slice(children);

    let mut bytes = Cursor::new(Vec::new());
    BoxInfo::new(box_value.box_type(), 8 + u64::try_from(payload.len())?).write(&mut bytes)?;
    bytes.get_mut().extend_from_slice(&payload);
    Ok(bytes.into_inner())
}

#[cfg(feature = "mux")]
fn fourcc(value: &str) -> FourCc {
    FourCc::try_from(value).expect("valid fourcc literal")
}
