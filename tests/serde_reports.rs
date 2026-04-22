#![cfg(feature = "serde")]

mod support;

use std::fmt::Debug;

use mp4forge::cli::dump::{
    DumpPayloadStatus, FieldStructuredDumpBoxReport, FieldStructuredDumpReport,
    StructuredDumpBoxReport, StructuredDumpFieldReport, StructuredDumpReport,
};
use mp4forge::cli::probe::{
    CodecDetailedProbeReport, CodecDetailedProbeTrackReport, DetailedProbeReport,
    DetailedProbeTrackReport, MediaCharacteristicsProbeReport,
    MediaCharacteristicsProbeTrackReport, ProbeReport, ProbeTrackReport,
};
use mp4forge::codec::FieldValue;
use mp4forge::probe::{
    Av1CodecDetails, ColorInfo, DeclaredBitrateInfo, FieldOrderInfo, PixelAspectRatioInfo,
    TrackCodecDetails, TrackMediaCharacteristics,
};

use support::fourcc;

#[test]
fn probe_report_types_roundtrip_with_serde_json() {
    assert_json_roundtrip(ProbeReport {
        major_brand: "isom".to_string(),
        minor_version: 512,
        compatible_brands: vec!["isom".to_string(), "iso2".to_string()],
        fast_start: true,
        timescale: 1_000,
        duration: 2_000,
        duration_seconds: 2.0,
        tracks: vec![ProbeTrackReport {
            track_id: 1,
            timescale: 90_000,
            duration: 3_072,
            duration_seconds: 0.034133334,
            codec: "avc1.64001F".to_string(),
            encrypted: false,
            width: Some(320),
            height: Some(180),
            sample_num: Some(3),
            chunk_num: Some(2),
            idr_frame_num: Some(1),
            bitrate: Some(20_000),
            max_bitrate: Some(32_000),
        }],
    });

    assert_json_roundtrip(DetailedProbeReport {
        major_brand: "isom".to_string(),
        minor_version: 512,
        compatible_brands: vec!["isom".to_string(), "iso8".to_string()],
        fast_start: true,
        timescale: 1_000,
        duration: 2_000,
        duration_seconds: 2.0,
        tracks: vec![DetailedProbeTrackReport {
            track_id: 1,
            timescale: 1_000,
            duration: 1_000,
            duration_seconds: 1.0,
            codec: "av01".to_string(),
            codec_family: "av1".to_string(),
            encrypted: false,
            handler_type: Some("vide".to_string()),
            language: Some("eng".to_string()),
            sample_entry_type: Some("av01".to_string()),
            original_format: None,
            protection_scheme_type: None,
            protection_scheme_version: None,
            width: Some(640),
            height: Some(360),
            channel_count: None,
            sample_rate: None,
            sample_num: Some(1),
            chunk_num: Some(1),
            idr_frame_num: None,
            bitrate: Some(32_000),
            max_bitrate: Some(32_000),
        }],
    });

    let codec_report = CodecDetailedProbeReport {
        major_brand: "isom".to_string(),
        minor_version: 512,
        compatible_brands: vec!["isom".to_string(), "iso8".to_string()],
        fast_start: true,
        timescale: 1_000,
        duration: 2_000,
        duration_seconds: 2.0,
        tracks: vec![CodecDetailedProbeTrackReport {
            track_id: 1,
            timescale: 1_000,
            duration: 1_000,
            duration_seconds: 1.0,
            codec: "av01".to_string(),
            codec_family: "av1".to_string(),
            codec_details: TrackCodecDetails::Av1(Av1CodecDetails {
                seq_profile: 0,
                seq_level_idx_0: 13,
                seq_tier_0: 1,
                bit_depth: 10,
                monochrome: false,
                chroma_subsampling_x: 1,
                chroma_subsampling_y: 0,
                chroma_sample_position: 2,
                initial_presentation_delay_minus_one: Some(3),
            }),
            encrypted: false,
            handler_type: Some("vide".to_string()),
            language: Some("eng".to_string()),
            sample_entry_type: Some("av01".to_string()),
            original_format: None,
            protection_scheme_type: None,
            protection_scheme_version: None,
            width: Some(640),
            height: Some(360),
            channel_count: None,
            sample_rate: None,
            sample_num: Some(1),
            chunk_num: Some(1),
            idr_frame_num: None,
            bitrate: Some(32_000),
            max_bitrate: Some(32_000),
        }],
    };
    let codec_json = serde_json::to_value(&codec_report).unwrap();
    assert_eq!(codec_json["tracks"][0]["codec_details"]["kind"], "av1");
    assert_eq!(
        codec_json["tracks"][0]["codec_details"]["value"]["bit_depth"],
        10
    );
    assert_eq!(
        serde_json::from_value::<CodecDetailedProbeReport>(codec_json).unwrap(),
        codec_report
    );

    let media_report = MediaCharacteristicsProbeReport {
        major_brand: "isom".to_string(),
        minor_version: 512,
        compatible_brands: vec!["isom".to_string(), "iso8".to_string()],
        fast_start: true,
        timescale: 1_000,
        duration: 2_000,
        duration_seconds: 2.0,
        tracks: vec![MediaCharacteristicsProbeTrackReport {
            track_id: 1,
            timescale: 1_000,
            duration: 1_000,
            duration_seconds: 1.0,
            codec: "avc1.64001F".to_string(),
            codec_family: "avc".to_string(),
            codec_details: TrackCodecDetails::Unknown,
            media_characteristics: TrackMediaCharacteristics {
                declared_bitrate: Some(DeclaredBitrateInfo {
                    buffer_size_db: 32_768,
                    max_bitrate: 4_000_000,
                    avg_bitrate: 2_500_000,
                }),
                color: Some(ColorInfo {
                    colour_type: fourcc("nclx"),
                    colour_primaries: Some(9),
                    transfer_characteristics: Some(16),
                    matrix_coefficients: Some(9),
                    full_range: Some(true),
                    profile_size: None,
                    unknown_size: None,
                }),
                pixel_aspect_ratio: Some(PixelAspectRatioInfo {
                    h_spacing: 4,
                    v_spacing: 3,
                }),
                field_order: Some(FieldOrderInfo {
                    field_count: 2,
                    field_ordering: 6,
                    interlaced: true,
                }),
            },
            encrypted: false,
            handler_type: Some("vide".to_string()),
            language: Some("eng".to_string()),
            sample_entry_type: Some("avc1".to_string()),
            original_format: None,
            protection_scheme_type: None,
            protection_scheme_version: None,
            width: Some(640),
            height: Some(360),
            channel_count: None,
            sample_rate: None,
            sample_num: Some(1),
            chunk_num: Some(1),
            idr_frame_num: None,
            bitrate: Some(32_000),
            max_bitrate: Some(32_000),
        }],
    };
    let media_json = serde_json::to_value(&media_report).unwrap();
    assert_eq!(
        media_json["tracks"][0]["media_characteristics"]["color"]["colour_type"],
        "nclx"
    );
    assert_eq!(
        serde_json::from_value::<MediaCharacteristicsProbeReport>(media_json).unwrap(),
        media_report
    );
}

#[test]
fn dump_report_types_roundtrip_with_serde_json() {
    assert_json_roundtrip(StructuredDumpReport {
        boxes: vec![StructuredDumpBoxReport {
            box_type: "ftyp".to_string(),
            path: "ftyp".to_string(),
            offset: 0,
            size: 20,
            supported: true,
            payload_status: DumpPayloadStatus::Summary,
            payload_summary: Some("MajorBrand=\"isom\"".to_string()),
            payload_bytes: None,
            children: Vec::new(),
        }],
    });

    let report = FieldStructuredDumpReport {
        boxes: vec![FieldStructuredDumpBoxReport {
            box_type: "ftyp".to_string(),
            path: "ftyp".to_string(),
            offset: 0,
            size: 20,
            supported: true,
            payload_status: DumpPayloadStatus::Summary,
            payload_fields: vec![
                StructuredDumpFieldReport {
                    name: "MajorBrand".to_string(),
                    value: FieldValue::String("isom".to_string()),
                    display_value: None,
                },
                StructuredDumpFieldReport {
                    name: "CompatibleBrands".to_string(),
                    value: FieldValue::Bytes(vec![105, 115, 111, 109]),
                    display_value: Some("[{CompatibleBrand=\"isom\"}]".to_string()),
                },
            ],
            payload_summary: Some("MajorBrand=\"isom\"".to_string()),
            payload_bytes: None,
            children: Vec::new(),
        }],
    };
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["boxes"][0]["payload_status"], "summary");
    assert_eq!(
        json["boxes"][0]["payload_fields"][1]["value"]["kind"],
        "bytes"
    );
    assert_eq!(
        json["boxes"][0]["payload_fields"][1]["value"]["value"][0],
        105
    );
    assert_eq!(
        serde_json::from_value::<FieldStructuredDumpReport>(json).unwrap(),
        report
    );
}

fn assert_json_roundtrip<T>(value: T)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + Debug,
{
    let json = serde_json::to_value(&value).unwrap();
    assert_eq!(serde_json::from_value::<T>(json).unwrap(), value);
}
