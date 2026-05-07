#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use mp4forge::BoxInfo;
use mp4forge::FourCc;
use mp4forge::boxes::iso14496_12::{
    AudioSampleEntry, SampleEntry, VisualSampleEntry, XMLSubtitleSampleEntry,
};
use mp4forge::boxes::iso14496_30::{WVTTSampleEntry, WebVTTConfigurationBox, WebVTTSourceLabelBox};
use mp4forge::codec::{CodecBox, marshal};
use mp4forge::mux::{
    MuxFileConfig, MuxInterleavePolicy, MuxStagedMediaItem, MuxTrackConfig,
    plan_staged_media_items, write_mp4_mux_to_path,
};

#[derive(Clone, Copy)]
struct TestMuxSample<'a> {
    pub bytes: &'a [u8],
    pub duration: u32,
    pub composition_time_offset: i32,
    pub is_sync_sample: bool,
}

pub fn fourcc(value: &str) -> FourCc {
    FourCc::try_from(value).expect("valid fourcc")
}

pub fn write_temp_file(prefix: &str, extension: &str, data: &[u8]) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "mp4forge-{prefix}-{}-{unique}.{extension}",
        std::process::id()
    ));
    fs::write(&path, data).expect("write temp example file");
    path
}

pub fn write_test_flac_file(prefix: &str, frame_payload: &[u8]) -> PathBuf {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"fLaC");
    bytes.push(0x80);
    bytes.extend_from_slice(&34_u32.to_be_bytes()[1..]);
    bytes.extend_from_slice(&build_flac_streaminfo_block(48_000, 2, 16, 1_024));
    bytes.extend_from_slice(frame_payload);
    write_temp_file(prefix, "flac", &bytes)
}

pub fn write_test_av1_ivf_file(
    prefix: &str,
    width: u16,
    height: u16,
    frame_timestamps: &[u64],
    frame_payloads: &[&[u8]],
) -> PathBuf {
    write_test_ivf_file(
        prefix,
        *b"AV01",
        IvfHeaderFields {
            width,
            height,
            timescale: 1_000,
            timestamp_scale: 1,
        },
        frame_timestamps,
        frame_payloads,
    )
}

fn write_single_track_mp4_input(
    prefix: &str,
    file_config: &MuxFileConfig,
    track_config: MuxTrackConfig,
    samples: &[TestMuxSample<'_>],
) -> PathBuf {
    let source_bytes = samples
        .iter()
        .flat_map(|sample| sample.bytes)
        .copied()
        .collect::<Vec<_>>();
    let source_path = write_temp_file(&format!("{prefix}-source"), "bin", &source_bytes);
    let output_path = write_temp_file(prefix, "mp4", &[]);

    let mut source_offset = 0_u64;
    let mut decode_time = 0_u64;
    let staged_items = samples
        .iter()
        .map(|sample| {
            let item = MuxStagedMediaItem::new(
                0,
                track_config.track_id(),
                decode_time,
                sample.duration,
                source_offset,
                u32::try_from(sample.bytes.len()).expect("sample size fits in u32"),
            )
            .with_composition_time_offset(sample.composition_time_offset)
            .with_sync_sample(sample.is_sync_sample);
            source_offset += u64::try_from(sample.bytes.len()).expect("sample size fits in u64");
            decode_time += u64::from(sample.duration);
            item
        })
        .collect::<Vec<_>>();
    let plan = plan_staged_media_items(staged_items, MuxInterleavePolicy::DecodeTime)
        .expect("plan staged media items");

    write_mp4_mux_to_path(
        &[&source_path],
        &output_path,
        file_config,
        &[track_config],
        &plan,
    )
    .expect("write one-track input mp4");
    output_path
}

pub fn build_audio_input_file(
    prefix: &str,
    major_brand: FourCc,
    sample_entry_type: &str,
    payloads: &[&[u8]],
) -> PathBuf {
    build_audio_input_file_with_timing(
        prefix,
        major_brand,
        sample_entry_type,
        48_000,
        1_024,
        payloads,
    )
}

pub fn build_audio_input_file_with_timing(
    prefix: &str,
    major_brand: FourCc,
    sample_entry_type: &str,
    track_timescale: u32,
    sample_duration: u32,
    payloads: &[&[u8]],
) -> PathBuf {
    let samples = payloads
        .iter()
        .copied()
        .map(|bytes| TestMuxSample {
            bytes,
            duration: sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        })
        .collect::<Vec<_>>();
    write_single_track_mp4_input(
        prefix,
        &MuxFileConfig::new(track_timescale)
            .with_major_brand(major_brand)
            .with_compatible_brand(fourcc("mp42")),
        MuxTrackConfig::new_audio(
            1,
            track_timescale,
            audio_sample_entry_box_with_type(sample_entry_type),
        )
        .with_language(*b"eng")
        .with_handler_name("ExampleAudioHandler"),
        &samples,
    )
}

#[derive(Clone, Copy)]
struct IvfHeaderFields {
    width: u16,
    height: u16,
    timescale: u32,
    timestamp_scale: u32,
}

fn write_test_ivf_file(
    prefix: &str,
    codec_fourcc: [u8; 4],
    header: IvfHeaderFields,
    frame_timestamps: &[u64],
    frame_payloads: &[&[u8]],
) -> PathBuf {
    assert_eq!(frame_timestamps.len(), frame_payloads.len());
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"DKIF");
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes.extend_from_slice(&32_u16.to_le_bytes());
    bytes.extend_from_slice(&codec_fourcc);
    bytes.extend_from_slice(&header.width.to_le_bytes());
    bytes.extend_from_slice(&header.height.to_le_bytes());
    bytes.extend_from_slice(&header.timescale.to_le_bytes());
    bytes.extend_from_slice(&header.timestamp_scale.to_le_bytes());
    bytes.extend_from_slice(
        &u32::try_from(frame_payloads.len())
            .expect("frame count fits")
            .to_le_bytes(),
    );
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    for (timestamp, payload) in frame_timestamps.iter().zip(frame_payloads.iter()) {
        bytes.extend_from_slice(
            &u32::try_from(payload.len())
                .expect("frame size fits")
                .to_le_bytes(),
        );
        bytes.extend_from_slice(&timestamp.to_le_bytes());
        bytes.extend_from_slice(payload);
    }
    write_temp_file(prefix, "ivf", &bytes)
}

fn build_flac_streaminfo_block(
    sample_rate: u32,
    channel_count: u8,
    bits_per_sample: u8,
    total_samples: u64,
) -> [u8; 34] {
    let mut block = [0_u8; 34];
    block[0..2].copy_from_slice(&0x0400_u16.to_be_bytes());
    block[2..4].copy_from_slice(&0x0400_u16.to_be_bytes());
    block[10] = u8::try_from((sample_rate >> 12) & 0xFF).expect("rate nibble fits");
    block[11] = u8::try_from((sample_rate >> 4) & 0xFF).expect("rate byte fits");
    block[12] = (u8::try_from(sample_rate & 0x0F).expect("rate low nibble fits") << 4)
        | (((channel_count - 1) & 0x07) << 1)
        | (((bits_per_sample - 1) >> 4) & 0x01);
    block[13] = (((bits_per_sample - 1) & 0x0F) << 4)
        | u8::try_from((total_samples >> 32) & 0x0F).expect("sample-count nibble fits");
    block[14] = u8::try_from((total_samples >> 24) & 0xFF).expect("sample-count byte fits");
    block[15] = u8::try_from((total_samples >> 16) & 0xFF).expect("sample-count byte fits");
    block[16] = u8::try_from((total_samples >> 8) & 0xFF).expect("sample-count byte fits");
    block[17] = u8::try_from(total_samples & 0xFF).expect("sample-count byte fits");
    block
}

pub fn build_video_input_file(
    prefix: &str,
    major_brand: FourCc,
    sample_entry_type: &str,
    payloads: &[&[u8]],
) -> PathBuf {
    let samples = payloads
        .iter()
        .copied()
        .map(|bytes| TestMuxSample {
            bytes,
            duration: 1_000,
            composition_time_offset: 0,
            is_sync_sample: true,
        })
        .collect::<Vec<_>>();
    write_single_track_mp4_input(
        prefix,
        &MuxFileConfig::new(1_000)
            .with_major_brand(major_brand)
            .with_compatible_brand(fourcc("mp42")),
        MuxTrackConfig::new_video(
            1,
            1_000,
            640,
            360,
            video_sample_entry_box_with_type(sample_entry_type),
        )
        .with_language(*b"und")
        .with_handler_name("ExampleVideoHandler"),
        &samples,
    )
}

pub fn build_text_input_file(prefix: &str, major_brand: FourCc) -> PathBuf {
    let first_source = write_temp_file(&format!("{prefix}-source-text"), "txt", b"wvtt");
    let second_source = write_temp_file(&format!("{prefix}-source-subtitle"), "xml", b"stpp");
    let output_path = write_temp_file(prefix, "mp4", &[]);
    let plan = plan_staged_media_items(
        vec![
            MuxStagedMediaItem::new(0, 1, 0, 1_000, 0, 4).with_sync_sample(true),
            MuxStagedMediaItem::new(1, 2, 0, 1_000, 0, 4).with_sync_sample(true),
        ],
        MuxInterleavePolicy::DecodeTime,
    )
    .expect("plan text/subtitle items");
    let file_config = MuxFileConfig::new(1_000)
        .with_major_brand(major_brand)
        .with_compatible_brand(fourcc("mp42"));
    let track_configs = vec![
        MuxTrackConfig::new_text(1, 1_000, 0, 0, wvtt_sample_entry_box())
            .with_language(*b"eng")
            .with_handler_name("EnglishCaptionHandler"),
        MuxTrackConfig::new_subtitle(2, 1_000, 0, 0, stpp_sample_entry_box())
            .with_language(*b"fra")
            .with_handler_name("FrenchSubtitleHandler"),
    ];

    write_mp4_mux_to_path(
        &[&first_source, &second_source],
        &output_path,
        &file_config,
        &track_configs,
        &plan,
    )
    .expect("write mixed text input mp4");
    output_path
}

fn audio_sample_entry_box_with_type(box_type: &str) -> Vec<u8> {
    encode_supported_box(
        &AudioSampleEntry {
            sample_entry: SampleEntry {
                box_type: fourcc(box_type),
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

fn video_sample_entry_box_with_type(box_type: &str) -> Vec<u8> {
    encode_supported_box(
        &VisualSampleEntry {
            sample_entry: SampleEntry {
                box_type: fourcc(box_type),
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

fn wvtt_sample_entry_box() -> Vec<u8> {
    let children = [
        encode_supported_box(
            &WebVTTConfigurationBox {
                config: "WEBVTT".to_string(),
            },
            &[],
        ),
        encode_supported_box(
            &WebVTTSourceLabelBox {
                source_label: "example".to_string(),
            },
            &[],
        ),
    ]
    .concat();

    encode_supported_box(
        &WVTTSampleEntry {
            sample_entry: SampleEntry {
                box_type: fourcc("wvtt"),
                data_reference_index: 1,
            },
        },
        &children,
    )
}

fn stpp_sample_entry_box() -> Vec<u8> {
    encode_supported_box(
        &XMLSubtitleSampleEntry {
            sample_entry: SampleEntry {
                box_type: fourcc("stpp"),
                data_reference_index: 1,
            },
            namespace: "http://www.w3.org/ns/ttml".to_string(),
            schema_location: String::new(),
            auxiliary_mime_types: String::new(),
        },
        &[],
    )
}

fn encode_supported_box<B>(box_value: &B, children: &[u8]) -> Vec<u8>
where
    B: CodecBox,
{
    let mut payload = Vec::new();
    marshal(&mut payload, box_value, None).expect("encode supported box payload");
    payload.extend_from_slice(children);
    let info = BoxInfo::new(box_value.box_type(), 8 + payload.len() as u64);
    let mut bytes = info.encode();
    bytes.extend_from_slice(&payload);
    bytes
}
