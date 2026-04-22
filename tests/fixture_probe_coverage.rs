mod support;

use std::fs;

use mp4forge::cli::probe as cli_probe;
use mp4forge::probe::{TrackCodec, TrackCodecFamily, probe, probe_media_characteristics};

use support::fixture_path;

struct FixtureExpectation {
    file_name: &'static str,
    major_brand: &'static str,
    tracks: &'static [TrackExpectation],
}

struct TrackExpectation {
    coarse_codec: TrackCodec,
    report_codec: &'static str,
    codec_family: TrackCodecFamily,
    sample_entry_type: &'static str,
    width: Option<u16>,
    height: Option<u16>,
    channel_count: Option<u16>,
    sample_rate: Option<u16>,
}

#[test]
fn fixture_probe_surfaces_cover_added_codec_families() {
    let cases = [
        FixtureExpectation {
            file_name: "vp9_opus.mp4",
            major_brand: "isom",
            tracks: &[
                TrackExpectation {
                    coarse_codec: TrackCodec::Unknown,
                    report_codec: "vp09",
                    codec_family: TrackCodecFamily::Vp9,
                    sample_entry_type: "vp09",
                    width: Some(1920),
                    height: Some(1080),
                    channel_count: None,
                    sample_rate: None,
                },
                TrackExpectation {
                    coarse_codec: TrackCodec::Unknown,
                    report_codec: "Opus",
                    codec_family: TrackCodecFamily::Opus,
                    sample_entry_type: "Opus",
                    width: None,
                    height: None,
                    channel_count: Some(2),
                    sample_rate: Some(48_000),
                },
            ],
        },
        FixtureExpectation {
            file_name: "av1_opus.mp4",
            major_brand: "isom",
            tracks: &[
                TrackExpectation {
                    coarse_codec: TrackCodec::Unknown,
                    report_codec: "av01",
                    codec_family: TrackCodecFamily::Av1,
                    sample_entry_type: "av01",
                    width: Some(1280),
                    height: Some(720),
                    channel_count: None,
                    sample_rate: None,
                },
                TrackExpectation {
                    coarse_codec: TrackCodec::Unknown,
                    report_codec: "Opus",
                    codec_family: TrackCodecFamily::Opus,
                    sample_entry_type: "Opus",
                    width: None,
                    height: None,
                    channel_count: Some(2),
                    sample_rate: Some(48_000),
                },
            ],
        },
        FixtureExpectation {
            file_name: "aac_audio.mp4",
            major_brand: "isom",
            tracks: &[TrackExpectation {
                coarse_codec: TrackCodec::Mp4a,
                report_codec: "mp4a.40.2",
                codec_family: TrackCodecFamily::Mp4Audio,
                sample_entry_type: "mp4a",
                width: None,
                height: None,
                channel_count: Some(2),
                sample_rate: Some(48_000),
            }],
        },
        FixtureExpectation {
            file_name: "opus_audio.mp4",
            major_brand: "isom",
            tracks: &[TrackExpectation {
                coarse_codec: TrackCodec::Unknown,
                report_codec: "Opus",
                codec_family: TrackCodecFamily::Opus,
                sample_entry_type: "Opus",
                width: None,
                height: None,
                channel_count: Some(2),
                sample_rate: Some(48_000),
            }],
        },
        FixtureExpectation {
            file_name: "pcm_audio.mp4",
            major_brand: "isom",
            tracks: &[TrackExpectation {
                coarse_codec: TrackCodec::Unknown,
                report_codec: "ipcm",
                codec_family: TrackCodecFamily::Pcm,
                sample_entry_type: "ipcm",
                width: None,
                height: None,
                channel_count: Some(2),
                sample_rate: Some(48_000),
            }],
        },
    ];

    for case in cases {
        let path = fixture_path(case.file_name);

        let mut summary_file = fs::File::open(&path).unwrap();
        let summary = probe(&mut summary_file).unwrap();
        assert_eq!(summary.major_brand.to_string(), case.major_brand);
        assert_eq!(
            summary.tracks.len(),
            case.tracks.len(),
            "fixture={}",
            case.file_name
        );

        let mut rich_file = fs::File::open(&path).unwrap();
        let rich = probe_media_characteristics(&mut rich_file).unwrap();
        assert_eq!(
            rich.major_brand.to_string(),
            case.major_brand,
            "fixture={}",
            case.file_name
        );
        assert_eq!(
            rich.tracks.len(),
            case.tracks.len(),
            "fixture={}",
            case.file_name
        );

        let mut report_file = fs::File::open(&path).unwrap();
        let report = cli_probe::build_media_characteristics_report(&mut report_file).unwrap();
        assert_eq!(
            report.major_brand, case.major_brand,
            "fixture={}",
            case.file_name
        );
        assert_eq!(
            report.tracks.len(),
            case.tracks.len(),
            "fixture={}",
            case.file_name
        );

        for (((summary_track, rich_track), report_track), expected_track) in summary
            .tracks
            .iter()
            .zip(rich.tracks.iter())
            .zip(report.tracks.iter())
            .zip(case.tracks.iter())
        {
            assert_eq!(
                summary_track.codec, expected_track.coarse_codec,
                "fixture={} track={}",
                case.file_name, summary_track.track_id
            );
            assert_eq!(
                rich_track.summary.codec_family, expected_track.codec_family,
                "fixture={} track={}",
                case.file_name, summary_track.track_id
            );
            assert_eq!(
                rich_track.summary.sample_entry_type,
                Some(mp4forge::FourCc::try_from(expected_track.sample_entry_type).unwrap()),
                "fixture={} track={}",
                case.file_name,
                summary_track.track_id
            );
            assert_eq!(
                report_track.codec, expected_track.report_codec,
                "fixture={} track={}",
                case.file_name, summary_track.track_id
            );
            assert_eq!(
                report_track.codec_family,
                codec_family_name(expected_track.codec_family),
                "fixture={} track={}",
                case.file_name,
                summary_track.track_id
            );
            assert_eq!(
                report_track.sample_entry_type.as_deref(),
                Some(expected_track.sample_entry_type),
                "fixture={} track={}",
                case.file_name,
                summary_track.track_id
            );
            assert_eq!(
                report_track.width, expected_track.width,
                "fixture={} track={}",
                case.file_name, summary_track.track_id
            );
            assert_eq!(
                report_track.height, expected_track.height,
                "fixture={} track={}",
                case.file_name, summary_track.track_id
            );
            assert_eq!(
                report_track.channel_count, expected_track.channel_count,
                "fixture={} track={}",
                case.file_name, summary_track.track_id
            );
            assert_eq!(
                report_track.sample_rate, expected_track.sample_rate,
                "fixture={} track={}",
                case.file_name, summary_track.track_id
            );
        }
    }
}

fn codec_family_name(value: TrackCodecFamily) -> &'static str {
    match value {
        TrackCodecFamily::Unknown => "unknown",
        TrackCodecFamily::Avc => "avc",
        TrackCodecFamily::Hevc => "hevc",
        TrackCodecFamily::Av1 => "av1",
        TrackCodecFamily::Vp8 => "vp8",
        TrackCodecFamily::Vp9 => "vp9",
        TrackCodecFamily::Mp4Audio => "mp4_audio",
        TrackCodecFamily::Opus => "opus",
        TrackCodecFamily::Ac3 => "ac3",
        TrackCodecFamily::Pcm => "pcm",
        TrackCodecFamily::XmlSubtitle => "xml_subtitle",
        TrackCodecFamily::TextSubtitle => "text_subtitle",
        TrackCodecFamily::WebVtt => "webvtt",
    }
}
