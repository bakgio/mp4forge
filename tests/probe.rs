#![allow(clippy::field_reassign_with_default)]

#[cfg(feature = "async")]
use std::fs;
use std::io::Cursor;

use mp4forge::boxes::AnyTypeBox;
use mp4forge::boxes::av1::AV1CodecConfiguration;
use mp4forge::boxes::avs3::Av3c;
use mp4forge::boxes::etsi_ts_102_366::{Dac3, Dec3, Ec3Substream};
use mp4forge::boxes::etsi_ts_103_190::Dac4;
use mp4forge::boxes::flac::{DfLa, FlacMetadataBlock};
use mp4forge::boxes::iso14496_12::{
    AVCDecoderConfiguration, AlbumLoudnessInfo, AudioSampleEntry, Btrt, Clap, CoLL, Colr, Ctts,
    CttsEntry, Edts, Elng, Elst, ElstEntry, Fiel, Frma, Ftyp, HEVCDecoderConfiguration, Hdlr,
    LoudnessEntry, LoudnessMeasurement, Ludt, Mdhd, Mdia, Meta, Minf, Moof, Moov, Mvhd, Nmhd, Pasp,
    Prft, SampleEntry, Schm, Sinf, SmDm, SphericalVideoV1Metadata, Stbl, Stco, Sthd, Stsc,
    StscEntry, Stsd, Stsz, Stts, SttsEntry, TFHD_DEFAULT_SAMPLE_DURATION_PRESENT,
    TFHD_DEFAULT_SAMPLE_SIZE_PRESENT, TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT,
    TRUN_SAMPLE_DURATION_PRESENT, TRUN_SAMPLE_SIZE_PRESENT, TextSubtitleSampleEntry, Tfdt, Tfhd,
    Tkhd, TrackLoudnessInfo, Traf, Trak, Trun, TrunEntry, UUID_FRAGMENT_ABSOLUTE_TIMING,
    UUID_FRAGMENT_RUN_TABLE, UUID_SAMPLE_ENCRYPTION, UUID_SPHERICAL_VIDEO_V1, Udta, Uuid,
    UuidFragmentAbsoluteTiming, UuidFragmentRunEntry, UuidFragmentRunTable, UuidPayload,
    VisualSampleEntry, XMLSubtitleSampleEntry,
};
use mp4forge::boxes::iso14496_14::{
    DECODER_CONFIG_DESCRIPTOR_TAG, DECODER_SPECIFIC_INFO_TAG, DecoderConfigDescriptor, Descriptor,
    Esds,
};
use mp4forge::boxes::iso14496_15::VVCDecoderConfiguration;
use mp4forge::boxes::iso14496_30::{WVTTSampleEntry, WebVTTConfigurationBox, WebVTTSourceLabelBox};
use mp4forge::boxes::iso23001_5::PcmC;
use mp4forge::boxes::iso23001_7::{SENC_USE_SUBSAMPLE_ENCRYPTION, Senc, SencSample, SencSubsample};
use mp4forge::boxes::metadata::Id32;
use mp4forge::boxes::mpeg_h::MhaC;
use mp4forge::boxes::opus::DOps;
use mp4forge::boxes::vp::VpCodecConfiguration;
use mp4forge::codec::{CodecBox, MutableBox, marshal};
use mp4forge::probe::{
    AacProfileInfo, EditListEntry, ProbeOptions, TrackCodec, TrackCodecDetails, TrackCodecFamily,
    average_sample_bitrate, average_segment_bitrate, detect_aac_profile, find_idr_frames,
    max_sample_bitrate, max_segment_bitrate, normalized_codec_family_name, probe, probe_bytes,
    probe_bytes_with_options, probe_codec_detailed, probe_codec_detailed_bytes,
    probe_codec_detailed_bytes_with_options, probe_codec_detailed_with_options, probe_detailed,
    probe_detailed_bytes, probe_detailed_bytes_with_options, probe_detailed_with_options,
    probe_extended_media_characteristics, probe_extended_media_characteristics_bytes, probe_fra,
    probe_fra_bytes, probe_fra_codec_detailed, probe_fra_codec_detailed_bytes, probe_fra_detailed,
    probe_fra_detailed_bytes, probe_media_characteristics, probe_media_characteristics_bytes,
    probe_media_characteristics_bytes_with_options, probe_media_characteristics_with_options,
    probe_with_options,
};
#[cfg(feature = "async")]
use mp4forge::probe::{
    find_idr_frames_async, probe_async, probe_codec_detailed_async,
    probe_codec_detailed_with_options_async, probe_detailed_async,
    probe_detailed_with_options_async, probe_extended_media_characteristics_async,
    probe_extended_media_characteristics_with_options,
    probe_extended_media_characteristics_with_options_async, probe_fra_async,
    probe_fra_codec_detailed_async, probe_fra_detailed_async, probe_fra_media_characteristics,
    probe_fra_media_characteristics_async, probe_media_characteristics_async,
    probe_media_characteristics_with_options_async, probe_with_options_async,
};
use mp4forge::{BoxInfo, FourCc};
#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

mod support;

#[cfg(feature = "async")]
use support::fixture_path;
use support::{build_encrypted_fragmented_video_file, build_event_message_movie_file};

#[test]
fn probe_summarizes_movie_tracks_samples_and_codecs() {
    let file = build_movie_file();
    let mut reader = Cursor::new(file);

    let info = probe(&mut reader).unwrap();

    assert_eq!(info.major_brand, fourcc("isom"));
    assert_eq!(info.minor_version, 0x0200);
    assert_eq!(
        info.compatible_brands,
        vec![fourcc("isom"), fourcc("iso2"), fourcc("avc1")]
    );
    assert!(info.fast_start);
    assert_eq!(info.timescale, 1_000);
    assert_eq!(info.duration, 2_000);
    assert!(info.segments.is_empty());
    assert_eq!(info.tracks.len(), 2);

    let video = &info.tracks[0];
    assert_eq!(video.track_id, 1);
    assert_eq!(video.timescale, 90_000);
    assert_eq!(video.duration, 3_072);
    assert_eq!(video.codec, TrackCodec::Avc1);
    assert!(!video.encrypted);
    assert_eq!(
        video.edit_list,
        vec![EditListEntry {
            media_time: 2_048,
            segment_duration: 1_024,
        }]
    );
    assert_eq!(
        video
            .samples
            .iter()
            .map(|sample| sample.size)
            .collect::<Vec<_>>(),
        vec![5, 5, 5]
    );
    assert_eq!(
        video
            .samples
            .iter()
            .map(|sample| sample.time_delta)
            .collect::<Vec<_>>(),
        vec![1_024, 1_024, 1_024]
    );
    assert_eq!(
        video
            .samples
            .iter()
            .map(|sample| sample.composition_time_offset)
            .collect::<Vec<_>>(),
        vec![256, 256, 128]
    );
    assert_eq!(
        video
            .chunks
            .iter()
            .map(|chunk| chunk.samples_per_chunk)
            .collect::<Vec<_>>(),
        vec![2, 1]
    );
    let avc = video.avc.as_ref().unwrap();
    assert_eq!(avc.configuration_version, 1);
    assert_eq!(avc.profile, 0x64);
    assert_eq!(avc.profile_compatibility, 0);
    assert_eq!(avc.level, 0x1f);
    assert_eq!(avc.length_size, 4);
    assert_eq!(avc.width, 320);
    assert_eq!(avc.height, 180);

    let audio = &info.tracks[1];
    assert_eq!(audio.track_id, 2);
    assert_eq!(audio.timescale, 48_000);
    assert_eq!(audio.duration, 2_048);
    assert_eq!(audio.codec, TrackCodec::Mp4a);
    assert!(!audio.encrypted);
    assert!(audio.edit_list.is_empty());
    assert_eq!(
        audio
            .samples
            .iter()
            .map(|sample| sample.size)
            .collect::<Vec<_>>(),
        vec![3, 4]
    );
    assert_eq!(audio.chunks.len(), 2);
    let mp4a = audio.mp4a.as_ref().unwrap();
    assert_eq!(mp4a.object_type_indication, 0x40);
    assert_eq!(mp4a.audio_object_type, 2);
    assert_eq!(mp4a.channel_count, 2);

    let idr_frames = find_idr_frames(&mut reader, video).unwrap();
    assert_eq!(idr_frames, vec![0]);
}

#[test]
fn probe_bytes_matches_cursor_based_probe() {
    let file = build_movie_file();
    let expected = probe(&mut Cursor::new(file.clone())).unwrap();
    let actual = probe_bytes(&file).unwrap();
    assert_eq!(actual, expected);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_probe_surfaces_match_sync_cursor_probe_surfaces() {
    let movie_file = build_movie_file();
    let expected_probe = probe(&mut Cursor::new(movie_file.clone())).unwrap();
    let actual_probe = probe_async(&mut Cursor::new(movie_file.clone()))
        .await
        .unwrap();
    assert_eq!(actual_probe, expected_probe);

    let expected_lightweight = probe_with_options(
        &mut Cursor::new(movie_file.clone()),
        ProbeOptions::lightweight(),
    )
    .unwrap();
    let actual_lightweight = probe_with_options_async(
        &mut Cursor::new(movie_file.clone()),
        ProbeOptions::lightweight(),
    )
    .await
    .unwrap();
    assert_eq!(actual_lightweight, expected_lightweight);

    let expected_detailed = probe_detailed(&mut Cursor::new(movie_file.clone())).unwrap();
    let actual_detailed = probe_detailed_async(&mut Cursor::new(movie_file.clone()))
        .await
        .unwrap();
    assert_eq!(actual_detailed, expected_detailed);

    let expected_detailed_lightweight = probe_detailed_with_options(
        &mut Cursor::new(movie_file.clone()),
        ProbeOptions::lightweight(),
    )
    .unwrap();
    let actual_detailed_lightweight = probe_detailed_with_options_async(
        &mut Cursor::new(movie_file.clone()),
        ProbeOptions::lightweight(),
    )
    .await
    .unwrap();
    assert_eq!(actual_detailed_lightweight, expected_detailed_lightweight);

    let video_track = expected_probe.tracks.first().unwrap();
    let expected_idr = find_idr_frames(&mut Cursor::new(movie_file.clone()), video_track).unwrap();
    let actual_idr = find_idr_frames_async(&mut Cursor::new(movie_file), video_track)
        .await
        .unwrap();
    assert_eq!(actual_idr, expected_idr);

    let hevc_file = build_hevc_movie_file();
    let expected_codec = probe_codec_detailed(&mut Cursor::new(hevc_file.clone())).unwrap();
    let actual_codec = probe_codec_detailed_async(&mut Cursor::new(hevc_file.clone()))
        .await
        .unwrap();
    assert_eq!(actual_codec, expected_codec);

    let expected_codec_lightweight = probe_codec_detailed_with_options(
        &mut Cursor::new(hevc_file.clone()),
        ProbeOptions::lightweight(),
    )
    .unwrap();
    let actual_codec_lightweight = probe_codec_detailed_with_options_async(
        &mut Cursor::new(hevc_file),
        ProbeOptions::lightweight(),
    )
    .await
    .unwrap();
    assert_eq!(actual_codec_lightweight, expected_codec_lightweight);

    let media_file = build_media_characteristics_movie_file();
    let expected_media = probe_media_characteristics(&mut Cursor::new(media_file.clone())).unwrap();
    let actual_media = probe_media_characteristics_async(&mut Cursor::new(media_file.clone()))
        .await
        .unwrap();
    assert_eq!(actual_media, expected_media);

    let expected_media_lightweight = probe_media_characteristics_with_options(
        &mut Cursor::new(media_file.clone()),
        ProbeOptions::lightweight(),
    )
    .unwrap();
    let actual_media_lightweight = probe_media_characteristics_with_options_async(
        &mut Cursor::new(media_file.clone()),
        ProbeOptions::lightweight(),
    )
    .await
    .unwrap();
    assert_eq!(actual_media_lightweight, expected_media_lightweight);

    let expected_extended =
        probe_extended_media_characteristics(&mut Cursor::new(media_file.clone())).unwrap();
    let actual_extended =
        probe_extended_media_characteristics_async(&mut Cursor::new(media_file.clone()))
            .await
            .unwrap();
    assert_eq!(actual_extended, expected_extended);

    let expected_extended_lightweight = probe_extended_media_characteristics_with_options(
        &mut Cursor::new(media_file.clone()),
        ProbeOptions::lightweight(),
    )
    .unwrap();
    let actual_extended_lightweight = probe_extended_media_characteristics_with_options_async(
        &mut Cursor::new(media_file),
        ProbeOptions::lightweight(),
    )
    .await
    .unwrap();
    assert_eq!(actual_extended_lightweight, expected_extended_lightweight);
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_probe_helpers_can_run_on_tokio_worker_threads() {
    let movie_file = build_movie_file();
    let summary_handle = tokio::spawn(async move {
        let mut reader = Cursor::new(movie_file);
        let summary = probe_async(&mut reader).await.unwrap();
        (summary.tracks.len(), summary.tracks[0].track_id)
    });
    assert_eq!(summary_handle.await.unwrap(), (2, 1));

    let media_file = build_media_characteristics_movie_file();
    let expected_media =
        probe_extended_media_characteristics(&mut Cursor::new(media_file.clone())).unwrap();
    let media_handle = tokio::spawn(async move {
        let mut reader = Cursor::new(media_file);
        let summary = probe_extended_media_characteristics_async(&mut reader)
            .await
            .unwrap();
        (
            summary.tracks.len(),
            summary.tracks[0].summary.summary.track_id,
        )
    });
    assert_eq!(
        media_handle.await.unwrap(),
        (
            expected_media.tracks.len(),
            expected_media.tracks[0].summary.summary.track_id,
        )
    );

    let hevc_file = build_hevc_movie_file();
    let expected_codec = probe_codec_detailed(&mut Cursor::new(hevc_file.clone())).unwrap();
    let codec_handle = tokio::spawn(async move {
        let mut reader = Cursor::new(hevc_file);
        let summary = probe_codec_detailed_async(&mut reader).await.unwrap();
        (
            summary.tracks.len(),
            summary.tracks[0].summary.summary.track_id,
        )
    });
    assert_eq!(
        codec_handle.await.unwrap(),
        (
            expected_codec.tracks.len(),
            expected_codec.tracks[0].summary.summary.track_id,
        )
    );

    let idr_file = build_movie_file();
    let mut sync_summary = probe(&mut Cursor::new(idr_file.clone())).unwrap();
    let track = sync_summary.tracks.remove(0);
    let idr_handle = tokio::spawn(async move {
        let mut reader = Cursor::new(idr_file);
        find_idr_frames_async(&mut reader, &track).await.unwrap()
    });
    assert_eq!(idr_handle.await.unwrap(), vec![0]);
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn async_probe_independent_file_tasks_can_run_concurrently_on_tokio_worker_threads() {
    let sample_path = fixture_path("sample.mp4");
    let fragmented_path = fixture_path("sample_fragmented.mp4");
    let qt_path = fixture_path("sample_qt.mp4");

    let expected_sample = probe(&mut Cursor::new(fs::read(&sample_path).unwrap())).unwrap();
    let expected_fragmented =
        probe_detailed(&mut Cursor::new(fs::read(&fragmented_path).unwrap())).unwrap();
    let expected_qt = probe_fra(&mut Cursor::new(fs::read(&qt_path).unwrap())).unwrap();

    let sample_handle = tokio::spawn(async move {
        let mut file = TokioFile::open(sample_path).await.unwrap();
        let summary = probe_async(&mut file).await.unwrap();
        (summary.tracks.len(), summary.tracks[0].track_id)
    });

    let fragmented_handle = tokio::spawn(async move {
        let mut file = TokioFile::open(fragmented_path).await.unwrap();
        let summary = probe_detailed_async(&mut file).await.unwrap();
        (summary.tracks.len(), summary.tracks[0].summary.track_id)
    });

    let qt_handle = tokio::spawn(async move {
        let mut file = TokioFile::open(qt_path).await.unwrap();
        let summary = probe_fra_async(&mut file).await.unwrap();
        (summary.tracks.len(), summary.tracks[0].track_id)
    });

    assert_eq!(
        sample_handle.await.unwrap(),
        (
            expected_sample.tracks.len(),
            expected_sample.tracks[0].track_id
        )
    );
    assert_eq!(
        fragmented_handle.await.unwrap(),
        (
            expected_fragmented.tracks.len(),
            expected_fragmented.tracks[0].summary.track_id,
        )
    );
    assert_eq!(
        qt_handle.await.unwrap(),
        (expected_qt.tracks.len(), expected_qt.tracks[0].track_id)
    );
}

#[test]
fn probe_with_options_skips_expensive_expansions_but_preserves_core_summary() {
    let file = build_movie_file();
    let mut reader = Cursor::new(file.clone());

    let full = probe(&mut Cursor::new(file)).unwrap();
    let info = probe_with_options(&mut reader, ProbeOptions::lightweight()).unwrap();

    assert_eq!(info.major_brand, full.major_brand);
    assert_eq!(info.minor_version, full.minor_version);
    assert_eq!(info.compatible_brands, full.compatible_brands);
    assert_eq!(info.fast_start, full.fast_start);
    assert_eq!(info.timescale, full.timescale);
    assert_eq!(info.duration, full.duration);
    assert_eq!(info.segments, Vec::new());
    assert_eq!(info.tracks.len(), full.tracks.len());

    for (light_track, full_track) in info.tracks.iter().zip(full.tracks.iter()) {
        assert_eq!(light_track.track_id, full_track.track_id);
        assert_eq!(light_track.timescale, full_track.timescale);
        assert_eq!(light_track.duration, full_track.duration);
        assert_eq!(light_track.codec, full_track.codec);
        assert_eq!(light_track.encrypted, full_track.encrypted);
        assert_eq!(light_track.edit_list, full_track.edit_list);
        assert_eq!(light_track.avc, full_track.avc);
        assert_eq!(light_track.mp4a, full_track.mp4a);
        assert!(light_track.samples.is_empty());
        assert!(light_track.chunks.is_empty());
    }
}

#[test]
fn probe_with_options_bytes_matches_cursor_based_probe() {
    let file = build_movie_file();
    let options = ProbeOptions::lightweight();
    let expected = probe_with_options(&mut Cursor::new(file.clone()), options).unwrap();
    let actual = probe_bytes_with_options(&file, options).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn probe_detailed_exposes_handler_language_sample_entry_and_codec_family() {
    let file = build_movie_file();
    let mut reader = Cursor::new(file);

    let info = probe_detailed(&mut reader).unwrap();

    assert_eq!(info.tracks.len(), 2);

    let video = &info.tracks[0];
    assert_eq!(video.summary.track_id, 1);
    assert_eq!(video.codec_family, TrackCodecFamily::Avc);
    assert_eq!(video.handler_type, Some(fourcc("vide")));
    assert_eq!(video.language.as_deref(), Some("eng"));
    assert_eq!(video.sample_entry_type, Some(fourcc("avc1")));
    assert_eq!(video.original_format, None);
    assert_eq!(video.display_width, Some(320));
    assert_eq!(video.display_height, Some(180));
    assert_eq!(video.channel_count, None);
    assert_eq!(video.sample_rate, None);

    let audio = &info.tracks[1];
    assert_eq!(audio.summary.track_id, 2);
    assert_eq!(audio.codec_family, TrackCodecFamily::Mp4Audio);
    assert_eq!(audio.handler_type, Some(fourcc("soun")));
    assert_eq!(audio.language.as_deref(), Some("eng"));
    assert_eq!(audio.sample_entry_type, Some(fourcc("mp4a")));
    assert_eq!(audio.original_format, None);
    assert_eq!(audio.display_width, None);
    assert_eq!(audio.display_height, None);
    assert_eq!(audio.channel_count, Some(2));
    assert_eq!(audio.sample_rate, Some(48_000));
}

#[test]
fn probe_detailed_prefers_extended_language_box_when_present() {
    let file = build_movie_file_with_extended_language();
    let mut reader = Cursor::new(file);

    let info = probe_detailed(&mut reader).unwrap();

    assert_eq!(info.tracks.len(), 1);
    assert_eq!(info.tracks[0].summary.track_id, 1);
    assert_eq!(info.tracks[0].language.as_deref(), Some("en-US"));
}

#[test]
fn probe_detailed_lightweight_is_stable_when_movie_carries_user_metadata_boxes() {
    let options = ProbeOptions::lightweight();
    let expected =
        probe_detailed_with_options(&mut Cursor::new(build_movie_file()), options).unwrap();
    let actual = probe_detailed_with_options(
        &mut Cursor::new(build_movie_file_with_user_metadata()),
        options,
    )
    .unwrap();

    assert_eq!(actual, expected);
}

#[test]
fn probe_detailed_lightweight_is_stable_when_movie_carries_legacy_uuid_boxes() {
    let options = ProbeOptions::lightweight();
    let expected =
        probe_detailed_with_options(&mut Cursor::new(build_movie_file()), options).unwrap();
    let actual = probe_detailed_with_options(
        &mut Cursor::new(build_movie_file_with_legacy_uuid_boxes()),
        options,
    )
    .unwrap();

    assert_eq!(actual, expected);
}

#[test]
fn probe_detailed_bytes_matches_cursor_based_probe_detailed() {
    let file = build_movie_file();
    let expected = probe_detailed(&mut Cursor::new(file.clone())).unwrap();
    let actual = probe_detailed_bytes(&file).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn probe_detailed_with_options_preserves_metadata_without_sample_tables() {
    let file = build_av01_movie_file();
    let mut reader = Cursor::new(file.clone());

    let full = probe_detailed(&mut Cursor::new(file)).unwrap();
    let info = probe_detailed_with_options(&mut reader, ProbeOptions::lightweight()).unwrap();

    assert_eq!(info.major_brand, full.major_brand);
    assert_eq!(info.timescale, full.timescale);
    assert_eq!(info.duration, full.duration);
    assert_eq!(info.segments, Vec::new());
    assert_eq!(info.tracks.len(), 1);
    assert_eq!(info.tracks[0].codec_family, full.tracks[0].codec_family);
    assert_eq!(info.tracks[0].handler_type, full.tracks[0].handler_type);
    assert_eq!(info.tracks[0].language, full.tracks[0].language);
    assert_eq!(
        info.tracks[0].sample_entry_type,
        full.tracks[0].sample_entry_type
    );
    assert_eq!(info.tracks[0].display_width, full.tracks[0].display_width);
    assert_eq!(info.tracks[0].display_height, full.tracks[0].display_height);
    assert!(info.tracks[0].summary.samples.is_empty());
    assert!(info.tracks[0].summary.chunks.is_empty());
}

#[test]
fn probe_detailed_bytes_with_options_matches_cursor_based_probe_detailed() {
    let file = build_movie_file();
    let options = ProbeOptions::lightweight();
    let expected = probe_detailed_with_options(&mut Cursor::new(file.clone()), options).unwrap();
    let actual = probe_detailed_bytes_with_options(&file, options).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn probe_codec_detailed_bytes_matches_cursor_based_probe_codec_detailed() {
    let file = build_hevc_movie_file();
    let expected = probe_codec_detailed(&mut Cursor::new(file.clone())).unwrap();
    let actual = probe_codec_detailed_bytes(&file).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn probe_codec_detailed_with_options_skips_fragment_segments() {
    let file = build_fragment_file();
    let mut reader = Cursor::new(file.clone());

    let full = probe_codec_detailed(&mut Cursor::new(file)).unwrap();
    let info = probe_codec_detailed_with_options(&mut reader, ProbeOptions::lightweight()).unwrap();

    assert_eq!(info.major_brand, full.major_brand);
    assert_eq!(info.timescale, full.timescale);
    assert_eq!(info.duration, full.duration);
    assert!(!full.segments.is_empty());
    assert!(info.segments.is_empty());
}

#[test]
fn probe_codec_detailed_bytes_with_options_matches_cursor_based_probe_codec_detailed() {
    let file = build_hevc_movie_file();
    let options = ProbeOptions::lightweight();
    let expected =
        probe_codec_detailed_with_options(&mut Cursor::new(file.clone()), options).unwrap();
    let actual = probe_codec_detailed_bytes_with_options(&file, options).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn probe_and_probe_fra_summarize_fragment_runs() {
    let file = build_fragment_file();

    let mut reader = Cursor::new(file.clone());
    let info = probe(&mut reader).unwrap();

    let mut reader = Cursor::new(file);
    let fra_info = probe_fra(&mut reader).unwrap();

    assert_eq!(fra_info, info);
    assert!(info.tracks.is_empty());
    assert_eq!(info.segments.len(), 2);

    let first = &info.segments[0];
    assert_eq!(first.track_id, 7);
    assert_eq!(first.moof_offset, 24);
    assert_eq!(first.base_media_decode_time, 9_000);
    assert_eq!(first.default_sample_duration, 1_000);
    assert_eq!(first.sample_count, 2);
    assert_eq!(first.duration, 3_000);
    assert_eq!(first.composition_time_offset, 500);
    assert_eq!(first.size, 10);

    let second = &info.segments[1];
    assert_eq!(second.track_id, 7);
    assert_eq!(
        second.moof_offset,
        24 + build_fragment_moof_one().len() as u64
    );
    assert_eq!(second.base_media_decode_time, 12_000);
    assert_eq!(second.default_sample_duration, 1_024);
    assert_eq!(second.sample_count, 3);
    assert_eq!(second.duration, 3_072);
    assert_eq!(second.composition_time_offset, 0);
    assert_eq!(second.size, 36);
}

#[test]
fn probe_fragment_summary_stays_stable_when_prft_precedes_each_moof() {
    let file = build_fragment_file_with_prft();

    let mut reader = Cursor::new(file.clone());
    let info = probe(&mut reader).unwrap();

    let mut reader = Cursor::new(file);
    let fra_info = probe_fra(&mut reader).unwrap();

    assert_eq!(fra_info, info);
    assert!(info.tracks.is_empty());
    assert_eq!(info.segments.len(), 2);

    let prft_size = build_prft_box_v0(7, 0x0000_0001_0203_0405, 9_000).len() as u64;

    let first = &info.segments[0];
    assert_eq!(first.track_id, 7);
    assert_eq!(first.moof_offset, 24 + prft_size);
    assert_eq!(first.base_media_decode_time, 9_000);
    assert_eq!(first.default_sample_duration, 1_000);
    assert_eq!(first.sample_count, 2);
    assert_eq!(first.duration, 3_000);
    assert_eq!(first.composition_time_offset, 500);
    assert_eq!(first.size, 10);

    let second = &info.segments[1];
    assert_eq!(second.track_id, 7);
    assert_eq!(
        second.moof_offset,
        24 + prft_size + build_fragment_moof_one().len() as u64 + prft_size
    );
    assert_eq!(second.base_media_decode_time, 12_000);
    assert_eq!(second.default_sample_duration, 1_024);
    assert_eq!(second.sample_count, 3);
    assert_eq!(second.duration, 3_072);
    assert_eq!(second.composition_time_offset, 0);
    assert_eq!(second.size, 36);
}

#[test]
fn probe_fra_detailed_bytes_matches_cursor_based_probe_fra_detailed() {
    let file = build_fragment_file();
    let expected = probe_fra_detailed(&mut Cursor::new(file.clone())).unwrap();
    let actual = probe_fra_detailed_bytes(&file).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn probe_fra_codec_detailed_bytes_matches_cursor_based_probe_fra_codec_detailed() {
    let file = build_fragment_file();
    let expected = probe_fra_codec_detailed(&mut Cursor::new(file.clone())).unwrap();
    let actual = probe_fra_codec_detailed_bytes(&file).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn probe_fra_bytes_matches_cursor_based_probe_fra() {
    let file = build_fragment_file();
    let expected = probe_fra(&mut Cursor::new(file.clone())).unwrap();
    let actual = probe_fra_bytes(&file).unwrap();
    assert_eq!(actual, expected);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_fragment_probe_surfaces_match_sync_cursor_probe_surfaces() {
    let fragment_file = build_fragment_file_with_prft();
    let expected_probe = probe(&mut Cursor::new(fragment_file.clone())).unwrap();
    let actual_probe = probe_async(&mut Cursor::new(fragment_file.clone()))
        .await
        .unwrap();
    assert_eq!(actual_probe, expected_probe);

    let expected_fra = probe_fra(&mut Cursor::new(fragment_file.clone())).unwrap();
    let actual_fra = probe_fra_async(&mut Cursor::new(fragment_file.clone()))
        .await
        .unwrap();
    assert_eq!(actual_fra, expected_fra);

    let expected_fra_detailed =
        probe_fra_detailed(&mut Cursor::new(fragment_file.clone())).unwrap();
    let actual_fra_detailed = probe_fra_detailed_async(&mut Cursor::new(fragment_file.clone()))
        .await
        .unwrap();
    assert_eq!(actual_fra_detailed, expected_fra_detailed);

    let expected_fra_codec =
        probe_fra_codec_detailed(&mut Cursor::new(fragment_file.clone())).unwrap();
    let actual_fra_codec = probe_fra_codec_detailed_async(&mut Cursor::new(fragment_file.clone()))
        .await
        .unwrap();
    assert_eq!(actual_fra_codec, expected_fra_codec);

    let expected_fra_media =
        probe_fra_media_characteristics(&mut Cursor::new(fragment_file.clone())).unwrap();
    let actual_fra_media = probe_fra_media_characteristics_async(&mut Cursor::new(fragment_file))
        .await
        .unwrap();
    assert_eq!(actual_fra_media, expected_fra_media);

    let encrypted_fragment_file = build_encrypted_fragmented_video_file();
    let expected_encrypted =
        probe_detailed(&mut Cursor::new(encrypted_fragment_file.clone())).unwrap();
    let actual_encrypted = probe_detailed_async(&mut Cursor::new(encrypted_fragment_file.clone()))
        .await
        .unwrap();
    assert_eq!(actual_encrypted, expected_encrypted);

    let event_file = build_event_message_movie_file();
    let expected_event = probe_media_characteristics(&mut Cursor::new(event_file.clone())).unwrap();
    let actual_event = probe_media_characteristics_async(&mut Cursor::new(event_file))
        .await
        .unwrap();
    assert_eq!(actual_event, expected_event);
}

#[test]
fn probe_detailed_recognizes_av01_track_family() {
    let file = build_av01_movie_file();
    let mut reader = Cursor::new(file);

    let info = probe_detailed(&mut reader).unwrap();

    assert_eq!(info.tracks.len(), 1);
    let track = &info.tracks[0];
    assert_eq!(track.summary.codec, TrackCodec::Unknown);
    assert_eq!(track.codec_family, TrackCodecFamily::Av1);
    assert_eq!(track.handler_type, Some(fourcc("vide")));
    assert_eq!(track.language.as_deref(), Some("eng"));
    assert_eq!(track.sample_entry_type, Some(fourcc("av01")));
    assert_eq!(track.original_format, None);
    assert_eq!(track.display_width, Some(640));
    assert_eq!(track.display_height, Some(360));
    assert_eq!(track.summary.samples.len(), 1);
}

#[test]
fn probe_detailed_surfaces_new_sample_entry_types_without_new_family_variants() {
    {
        let mut reader = Cursor::new(build_ec3_movie_file());
        let info = probe_detailed(&mut reader).unwrap();
        let track = &info.tracks[0];
        assert_eq!(track.summary.codec, TrackCodec::Unknown);
        assert_eq!(track.codec_family, TrackCodecFamily::Unknown);
        assert_eq!(track.sample_entry_type, Some(fourcc("ec-3")));
        assert_eq!(track.channel_count, Some(6));
        assert_eq!(track.sample_rate, Some(48_000));
        assert_eq!(
            normalized_codec_family_name(
                track.codec_family,
                track.sample_entry_type,
                track.original_format,
            ),
            "unknown"
        );
    }

    {
        let mut reader = Cursor::new(build_ac4_movie_file());
        let info = probe_detailed(&mut reader).unwrap();
        let track = &info.tracks[0];
        assert_eq!(track.summary.codec, TrackCodec::Unknown);
        assert_eq!(track.codec_family, TrackCodecFamily::Unknown);
        assert_eq!(track.sample_entry_type, Some(fourcc("ac-4")));
        assert_eq!(track.channel_count, Some(2));
        assert_eq!(track.sample_rate, Some(48_000));
        assert_eq!(
            normalized_codec_family_name(
                track.codec_family,
                track.sample_entry_type,
                track.original_format,
            ),
            "unknown"
        );
    }

    {
        let mut reader = Cursor::new(build_vvc_movie_file());
        let info = probe_detailed(&mut reader).unwrap();
        let track = &info.tracks[0];
        assert_eq!(track.summary.codec, TrackCodec::Unknown);
        assert_eq!(track.codec_family, TrackCodecFamily::Unknown);
        assert_eq!(track.sample_entry_type, Some(fourcc("vvc1")));
        assert_eq!(track.display_width, Some(640));
        assert_eq!(track.display_height, Some(360));
        assert_eq!(
            normalized_codec_family_name(
                track.codec_family,
                track.sample_entry_type,
                track.original_format,
            ),
            "unknown"
        );
    }

    {
        let mut reader = Cursor::new(build_avs3_movie_file());
        let info = probe_detailed(&mut reader).unwrap();
        let track = &info.tracks[0];
        assert_eq!(track.summary.codec, TrackCodec::Unknown);
        assert_eq!(track.codec_family, TrackCodecFamily::Unknown);
        assert_eq!(track.sample_entry_type, Some(fourcc("avs3")));
        assert_eq!(track.display_width, Some(640));
        assert_eq!(track.display_height, Some(360));
        assert_eq!(
            normalized_codec_family_name(
                track.codec_family,
                track.sample_entry_type,
                track.original_format,
            ),
            "avs3"
        );
    }

    {
        let mut reader = Cursor::new(build_flac_movie_file());
        let info = probe_detailed(&mut reader).unwrap();
        let track = &info.tracks[0];
        assert_eq!(track.summary.codec, TrackCodec::Unknown);
        assert_eq!(track.codec_family, TrackCodecFamily::Unknown);
        assert_eq!(track.sample_entry_type, Some(fourcc("fLaC")));
        assert_eq!(track.channel_count, Some(2));
        assert_eq!(track.sample_rate, Some(48_000));
        assert_eq!(
            normalized_codec_family_name(
                track.codec_family,
                track.sample_entry_type,
                track.original_format,
            ),
            "flac"
        );
    }

    {
        let mut reader = Cursor::new(build_mha1_movie_file());
        let info = probe_detailed(&mut reader).unwrap();
        let track = &info.tracks[0];
        assert_eq!(track.summary.codec, TrackCodec::Unknown);
        assert_eq!(track.codec_family, TrackCodecFamily::Unknown);
        assert_eq!(track.sample_entry_type, Some(fourcc("mha1")));
        assert_eq!(track.channel_count, Some(2));
        assert_eq!(track.sample_rate, Some(48_000));
        assert_eq!(
            normalized_codec_family_name(
                track.codec_family,
                track.sample_entry_type,
                track.original_format,
            ),
            "mpeg_h"
        );
    }
}

#[test]
fn probe_codec_detailed_keeps_unknown_codec_details_for_new_family_strings() {
    for file in [
        build_avs3_movie_file(),
        build_flac_movie_file(),
        build_mha1_movie_file(),
    ] {
        let info = probe_codec_detailed(&mut Cursor::new(file)).unwrap();
        let track = &info.tracks[0];
        assert_eq!(track.codec_details, TrackCodecDetails::Unknown);
    }
}

#[test]
fn probe_codec_detailed_exposes_richer_landed_codec_details() {
    {
        let mut reader = Cursor::new(build_hevc_movie_file());
        let info = probe_codec_detailed(&mut reader).unwrap();
        let track = &info.tracks[0];
        assert_eq!(track.summary.codec_family, TrackCodecFamily::Hevc);
        match &track.codec_details {
            TrackCodecDetails::Hevc(details) => {
                assert_eq!(details.configuration_version, 1);
                assert_eq!(details.profile_space, 1);
                assert!(details.tier_flag);
                assert_eq!(details.profile_idc, 2);
                assert_eq!(details.profile_compatibility_mask, 0x4000_0000);
                assert_eq!(details.constraint_indicator, [1, 2, 3, 4, 5, 6]);
                assert_eq!(details.level_idc, 120);
                assert_eq!(details.chroma_format_idc, 1);
                assert_eq!(details.bit_depth_luma, 10);
                assert_eq!(details.bit_depth_chroma, 10);
                assert_eq!(details.avg_frame_rate, 30_000);
                assert_eq!(details.length_size, 4);
            }
            other => panic!("expected HEVC details, got {other:?}"),
        }
    }

    {
        let mut reader = Cursor::new(build_av01_movie_file());
        let info = probe_codec_detailed(&mut reader).unwrap();
        let track = &info.tracks[0];
        assert_eq!(track.summary.codec_family, TrackCodecFamily::Av1);
        match &track.codec_details {
            TrackCodecDetails::Av1(details) => {
                assert_eq!(details.seq_profile, 0);
                assert_eq!(details.seq_level_idx_0, 13);
                assert_eq!(details.seq_tier_0, 1);
                assert_eq!(details.bit_depth, 10);
                assert!(!details.monochrome);
                assert_eq!(details.chroma_subsampling_x, 1);
                assert_eq!(details.chroma_subsampling_y, 0);
                assert_eq!(details.chroma_sample_position, 2);
                assert_eq!(details.initial_presentation_delay_minus_one, Some(3));
            }
            other => panic!("expected AV1 details, got {other:?}"),
        }
    }

    {
        let mut reader = Cursor::new(build_vp09_movie_file());
        let info = probe_codec_detailed(&mut reader).unwrap();
        let track = &info.tracks[0];
        assert_eq!(track.summary.codec_family, TrackCodecFamily::Vp9);
        match &track.codec_details {
            TrackCodecDetails::Vp9(details) => {
                assert_eq!(details.profile, 2);
                assert_eq!(details.level, 31);
                assert_eq!(details.bit_depth, 10);
                assert_eq!(details.chroma_subsampling, 1);
                assert!(details.full_range);
                assert_eq!(details.colour_primaries, 9);
                assert_eq!(details.transfer_characteristics, 16);
                assert_eq!(details.matrix_coefficients, 9);
                assert_eq!(details.codec_initialization_data_size, 3);
            }
            other => panic!("expected VP9 details, got {other:?}"),
        }
    }

    {
        let mut reader = Cursor::new(build_opus_movie_file());
        let info = probe_codec_detailed(&mut reader).unwrap();
        let track = &info.tracks[0];
        assert_eq!(track.summary.codec_family, TrackCodecFamily::Opus);
        match &track.codec_details {
            TrackCodecDetails::Opus(details) => {
                assert_eq!(details.output_channel_count, 2);
                assert_eq!(details.pre_skip, 312);
                assert_eq!(details.input_sample_rate, 48_000);
                assert_eq!(details.output_gain, 0);
                assert_eq!(details.channel_mapping_family, 1);
                assert_eq!(details.stream_count, Some(2));
                assert_eq!(details.coupled_count, Some(1));
                assert_eq!(details.channel_mapping, vec![0, 1]);
            }
            other => panic!("expected Opus details, got {other:?}"),
        }
    }

    {
        let mut reader = Cursor::new(build_ac3_movie_file());
        let info = probe_codec_detailed(&mut reader).unwrap();
        let track = &info.tracks[0];
        assert_eq!(track.summary.codec_family, TrackCodecFamily::Ac3);
        match &track.codec_details {
            TrackCodecDetails::Ac3(details) => {
                assert_eq!(details.sample_rate_code, 1);
                assert_eq!(details.bit_stream_identification, 8);
                assert_eq!(details.bit_stream_mode, 3);
                assert_eq!(details.audio_coding_mode, 7);
                assert!(details.lfe_on);
                assert_eq!(details.bit_rate_code, 10);
            }
            other => panic!("expected AC-3 details, got {other:?}"),
        }
    }

    {
        let mut reader = Cursor::new(build_pcm_movie_file());
        let info = probe_codec_detailed(&mut reader).unwrap();
        let track = &info.tracks[0];
        assert_eq!(track.summary.codec_family, TrackCodecFamily::Pcm);
        match &track.codec_details {
            TrackCodecDetails::Pcm(details) => {
                assert_eq!(details.format_flags, 1);
                assert_eq!(details.sample_size, 24);
            }
            other => panic!("expected PCM details, got {other:?}"),
        }
    }

    {
        let mut reader = Cursor::new(build_stpp_movie_file());
        let info = probe_codec_detailed(&mut reader).unwrap();
        let track = &info.tracks[0];
        assert_eq!(track.summary.codec_family, TrackCodecFamily::XmlSubtitle);
        match &track.codec_details {
            TrackCodecDetails::XmlSubtitle(details) => {
                assert_eq!(details.namespace, "urn:ebu:tt:metadata");
                assert_eq!(details.schema_location, "urn:ebu:tt:schema");
                assert_eq!(details.auxiliary_mime_types, "application/ttml+xml");
            }
            other => panic!("expected XML subtitle details, got {other:?}"),
        }
    }

    {
        let mut reader = Cursor::new(build_sbtt_movie_file());
        let info = probe_codec_detailed(&mut reader).unwrap();
        let track = &info.tracks[0];
        assert_eq!(track.summary.codec_family, TrackCodecFamily::TextSubtitle);
        match &track.codec_details {
            TrackCodecDetails::TextSubtitle(details) => {
                assert_eq!(details.content_encoding, "utf-8");
                assert_eq!(details.mime_format, "text/plain");
            }
            other => panic!("expected text subtitle details, got {other:?}"),
        }
    }

    {
        let mut reader = Cursor::new(build_wvtt_movie_file());
        let info = probe_codec_detailed(&mut reader).unwrap();
        let track = &info.tracks[0];
        assert_eq!(track.summary.codec_family, TrackCodecFamily::WebVtt);
        match &track.codec_details {
            TrackCodecDetails::WebVtt(details) => {
                assert_eq!(details.config.as_deref(), Some("WEBVTT"));
                assert_eq!(details.source_label.as_deref(), Some("eng"));
            }
            other => panic!("expected WebVTT details, got {other:?}"),
        }
    }
}

#[test]
fn probe_media_characteristics_reports_event_message_track_metadata() {
    let mut reader = Cursor::new(build_event_message_movie_file());
    let info = probe_media_characteristics(&mut reader).unwrap();
    let track = &info.tracks[0];

    assert_eq!(track.summary.codec_family, TrackCodecFamily::Unknown);
    assert_eq!(track.summary.sample_entry_type, Some(fourcc("evte")));
    assert_eq!(track.summary.handler_type, Some(fourcc("subt")));
    assert_eq!(
        track
            .media_characteristics
            .declared_bitrate
            .as_ref()
            .map(|value| (value.buffer_size_db, value.max_bitrate, value.avg_bitrate)),
        Some((32_768, 4_000_000, 2_500_000))
    );
    assert_eq!(track.codec_details, TrackCodecDetails::Unknown);
}

#[test]
fn probe_detailed_reports_protected_sample_entry_metadata() {
    let file = build_encrypted_video_movie_file();
    let mut reader = Cursor::new(file);

    let info = probe_detailed(&mut reader).unwrap();

    assert_eq!(info.tracks.len(), 1);
    let track = &info.tracks[0];
    assert_eq!(track.summary.codec, TrackCodec::Avc1);
    assert!(track.summary.encrypted);
    assert_eq!(track.codec_family, TrackCodecFamily::Avc);
    assert_eq!(track.handler_type, Some(fourcc("vide")));
    assert_eq!(track.language.as_deref(), Some("eng"));
    assert_eq!(track.sample_entry_type, Some(fourcc("encv")));
    assert_eq!(track.original_format, Some(fourcc("avc1")));
    assert_eq!(
        track
            .protection_scheme
            .as_ref()
            .map(|value| (value.scheme_type, value.scheme_version)),
        Some((fourcc("cenc"), 0x0001_0000))
    );
    assert_eq!(track.display_width, Some(320));
    assert_eq!(track.display_height, Some(180));
}

#[test]
fn probe_codec_detailed_reports_protected_hevc_codec_details() {
    let file = build_encrypted_hevc_movie_file();
    let mut reader = Cursor::new(file);

    let info = probe_codec_detailed(&mut reader).unwrap();

    assert_eq!(info.tracks.len(), 1);
    let track = &info.tracks[0];
    assert_eq!(track.summary.summary.codec, TrackCodec::Avc1);
    assert!(track.summary.summary.encrypted);
    assert_eq!(track.summary.codec_family, TrackCodecFamily::Hevc);
    assert_eq!(track.summary.sample_entry_type, Some(fourcc("encv")));
    assert_eq!(track.summary.original_format, Some(fourcc("hvc1")));
    match &track.codec_details {
        TrackCodecDetails::Hevc(details) => {
            assert_eq!(details.profile_idc, 2);
            assert_eq!(details.length_size, 4);
        }
        other => panic!("expected HEVC details, got {other:?}"),
    }
}

#[test]
fn probe_detailed_handles_fragmented_encrypted_metadata_boxes() {
    let file = build_encrypted_fragmented_video_file();
    let mut reader = Cursor::new(file);

    let info = probe_detailed(&mut reader).unwrap();

    assert_eq!(info.tracks.len(), 1);
    assert_eq!(info.segments.len(), 1);

    let track = &info.tracks[0];
    assert_eq!(track.summary.track_id, 1);
    assert_eq!(track.summary.codec, TrackCodec::Avc1);
    assert!(track.summary.encrypted);
    assert_eq!(track.codec_family, TrackCodecFamily::Avc);
    assert_eq!(track.sample_entry_type, Some(fourcc("encv")));
    assert_eq!(track.original_format, Some(fourcc("avc1")));
    assert_eq!(
        track
            .protection_scheme
            .as_ref()
            .map(|value| (value.scheme_type, value.scheme_version)),
        Some((fourcc("cenc"), 0x0001_0000))
    );

    let segment = &info.segments[0];
    assert_eq!(segment.track_id, 1);
    assert_eq!(segment.sample_count, 1);
    assert_eq!(segment.default_sample_duration, 1_000);
    assert_eq!(segment.duration, 1_000);
    assert_eq!(segment.size, 4);
}

#[test]
fn probe_media_characteristics_exposes_sample_entry_side_metadata() {
    let file = build_media_characteristics_movie_file();
    let mut reader = Cursor::new(file);

    let info = probe_media_characteristics(&mut reader).unwrap();

    assert_eq!(info.tracks.len(), 1);
    let track = &info.tracks[0];
    assert_eq!(track.summary.summary.track_id, 1);
    assert_eq!(track.summary.codec_family, TrackCodecFamily::Avc);
    assert_eq!(track.summary.sample_entry_type, Some(fourcc("avc1")));
    assert_eq!(
        track.media_characteristics.declared_bitrate,
        Some(mp4forge::probe::DeclaredBitrateInfo {
            buffer_size_db: 32_768,
            max_bitrate: 4_000_000,
            avg_bitrate: 2_500_000,
        })
    );
    assert_eq!(
        track.media_characteristics.color,
        Some(mp4forge::probe::ColorInfo {
            colour_type: fourcc("nclx"),
            colour_primaries: Some(9),
            transfer_characteristics: Some(16),
            matrix_coefficients: Some(9),
            full_range: Some(true),
            profile_size: None,
            unknown_size: None,
        })
    );
    assert_eq!(
        track.media_characteristics.pixel_aspect_ratio,
        Some(mp4forge::probe::PixelAspectRatioInfo {
            h_spacing: 4,
            v_spacing: 3,
        })
    );
    assert_eq!(
        track.media_characteristics.field_order,
        Some(mp4forge::probe::FieldOrderInfo {
            field_count: 2,
            field_ordering: 6,
            interlaced: true,
        })
    );
}

#[test]
fn probe_extended_media_characteristics_exposes_visual_sample_entry_side_metadata() {
    let file = build_media_characteristics_movie_file();
    let mut reader = Cursor::new(file);

    let info = probe_extended_media_characteristics(&mut reader).unwrap();

    assert_eq!(info.tracks.len(), 1);
    let track = &info.tracks[0];
    assert_eq!(
        track.visual_metadata.clean_aperture,
        Some(mp4forge::probe::CleanApertureInfo {
            width_numerator: 1_920,
            width_denominator: 1,
            height_numerator: 1_080,
            height_denominator: 1,
            horizontal_offset_numerator: 0,
            horizontal_offset_denominator: 1,
            vertical_offset_numerator: 0,
            vertical_offset_denominator: 1,
        })
    );
    assert_eq!(
        track.visual_metadata.content_light_level,
        Some(mp4forge::probe::ContentLightLevelInfo {
            max_cll: 1_000,
            max_fall: 400,
        })
    );
    assert_eq!(
        track.visual_metadata.mastering_display,
        Some(mp4forge::probe::MasteringDisplayInfo {
            primary_r_chromaticity_x: 34_000,
            primary_r_chromaticity_y: 16_000,
            primary_g_chromaticity_x: 13_250,
            primary_g_chromaticity_y: 34_500,
            primary_b_chromaticity_x: 7_500,
            primary_b_chromaticity_y: 3_000,
            white_point_chromaticity_x: 15_635,
            white_point_chromaticity_y: 16_450,
            luminance_max: 1_000_000,
            luminance_min: 50,
        })
    );
}

#[test]
fn probe_extended_media_characteristics_bytes_matches_cursor_based_probe() {
    let file = build_media_characteristics_movie_file();
    let expected = probe_extended_media_characteristics(&mut Cursor::new(file.clone())).unwrap();
    let actual = probe_extended_media_characteristics_bytes(&file).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn probe_media_characteristics_bytes_matches_cursor_based_probe() {
    let file = build_media_characteristics_movie_file();
    let expected = probe_media_characteristics(&mut Cursor::new(file.clone())).unwrap();
    let actual = probe_media_characteristics_bytes(&file).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn probe_media_characteristics_with_options_preserves_media_fields_without_sample_tables() {
    let file = build_media_characteristics_movie_file();
    let options = ProbeOptions::lightweight();
    let expected =
        probe_media_characteristics_with_options(&mut Cursor::new(file.clone()), options).unwrap();
    let actual = probe_media_characteristics_bytes_with_options(&file, options).unwrap();

    assert_eq!(actual, expected);
    assert_eq!(actual.tracks.len(), 1);
    assert!(actual.tracks[0].summary.summary.samples.is_empty());
    assert!(actual.tracks[0].summary.summary.chunks.is_empty());
    assert!(
        actual.tracks[0]
            .media_characteristics
            .declared_bitrate
            .is_some()
    );
}

#[test]
fn probe_bytes_propagates_decode_errors() {
    let file = encode_raw_box(fourcc("ftyp"), &[0x69, 0x73]);
    let expected = probe(&mut Cursor::new(file.clone())).unwrap_err();
    let actual = probe_bytes(&file).unwrap_err();

    assert_eq!(
        std::mem::discriminant(&actual),
        std::mem::discriminant(&expected)
    );
    assert_eq!(actual.to_string(), expected.to_string());
}

#[test]
fn detect_aac_profile_matches_expected_cases() {
    let cases = [
        (
            aac_profile_esds(0x40, &[0x10, 0x00]),
            Some(AacProfileInfo {
                object_type_indication: 0x40,
                audio_object_type: 2,
            }),
        ),
        (
            aac_profile_esds(0x40, &[0x10, 0x02, 0xb7, 0x2c, 0x00]),
            Some(AacProfileInfo {
                object_type_indication: 0x40,
                audio_object_type: 5,
            }),
        ),
        (
            aac_profile_esds(
                0x40,
                &[0x10, 0x02, 0xb7, 0x2f, 0xc0, 0x00, 0x00, 0x2a, 0x44],
            ),
            Some(AacProfileInfo {
                object_type_indication: 0x40,
                audio_object_type: 29,
            }),
        ),
        (
            aac_profile_esds(0x6b, &[0x10, 0x00]),
            Some(AacProfileInfo {
                object_type_indication: 0x6b,
                audio_object_type: 0,
            }),
        ),
    ];

    for (esds, expected) in cases {
        assert_eq!(detect_aac_profile(&esds).unwrap(), expected);
    }
}

#[test]
fn bitrate_helpers_match_expected_math() {
    let samples = [
        sample_info(100, 10, 0),
        sample_info(200, 10, 0),
        sample_info(300, 10, 0),
        sample_info(100, 10, 0),
        sample_info(200, 10, 0),
    ];
    assert_eq!(average_sample_bitrate(&samples, 100), 14_400);
    assert_eq!(max_sample_bitrate(&samples, 100, 20), 20_000);
    assert_eq!(average_sample_bitrate(&[], 100), 0);
    assert_eq!(max_sample_bitrate(&[], 100, 20), 0);

    let segments = [
        segment_info(1, 300, 10),
        segment_info(2, 100, 10),
        segment_info(2, 200, 10),
        segment_info(1, 200, 10),
        segment_info(2, 300, 10),
        segment_info(3, 700, 10),
        segment_info(2, 100, 10),
        segment_info(1, 800, 10),
        segment_info(2, 200, 10),
    ];
    assert_eq!(average_segment_bitrate(&segments, 2, 100), 14_400);
    assert_eq!(max_segment_bitrate(&segments, 2, 100), 24_000);
    assert_eq!(average_segment_bitrate(&[], 2, 100), 0);
    assert_eq!(max_segment_bitrate(&[], 2, 100), 0);
}

fn build_movie_file() -> Vec<u8> {
    let ftyp = encode_supported_box(
        &Ftyp {
            major_brand: fourcc("isom"),
            minor_version: 0x0200,
            compatible_brands: vec![fourcc("isom"), fourcc("iso2"), fourcc("avc1")],
        },
        &[],
    );

    let placeholder_moov = build_movie_moov(&[0, 0], &[0, 0]);
    let mdat_payload = movie_mdat_payload();
    let mdat_data_offset = ftyp.len() as u64 + placeholder_moov.len() as u64 + 8;
    let video_offsets = [mdat_data_offset, mdat_data_offset + 10];
    let audio_offsets = [mdat_data_offset + 15, mdat_data_offset + 18];

    let moov = build_movie_moov(&video_offsets, &audio_offsets);
    let mdat = encode_raw_box(fourcc("mdat"), &mdat_payload);
    [ftyp, moov, mdat].concat()
}

fn build_movie_file_with_extended_language() -> Vec<u8> {
    let ftyp = encode_supported_box(
        &Ftyp {
            major_brand: fourcc("isom"),
            minor_version: 0x0200,
            compatible_brands: vec![fourcc("isom"), fourcc("iso2"), fourcc("avc1")],
        },
        &[],
    );

    let placeholder_moov = build_movie_moov_with_extended_language(&[0, 0]);
    let mdat_payload = movie_mdat_payload();
    let mdat_data_offset = ftyp.len() as u64 + placeholder_moov.len() as u64 + 8;
    let video_offsets = [mdat_data_offset, mdat_data_offset + 10];

    let moov = build_movie_moov_with_extended_language(&video_offsets);
    let mdat = encode_raw_box(fourcc("mdat"), &mdat_payload);
    [ftyp, moov, mdat].concat()
}

fn build_movie_file_with_user_metadata() -> Vec<u8> {
    let ftyp = encode_supported_box(
        &Ftyp {
            major_brand: fourcc("isom"),
            minor_version: 0x0200,
            compatible_brands: vec![fourcc("isom"), fourcc("iso2"), fourcc("avc1")],
        },
        &[],
    );

    let placeholder_moov = build_movie_moov_with_user_metadata(&[0, 0], &[0, 0]);
    let spherical_uuid = build_spherical_uuid_box();
    let raw_uuid = build_raw_uuid_box();
    let mdat_payload = movie_mdat_payload();
    let mdat_data_offset = ftyp.len() as u64
        + placeholder_moov.len() as u64
        + spherical_uuid.len() as u64
        + raw_uuid.len() as u64
        + 8;
    let video_offsets = [mdat_data_offset, mdat_data_offset + 10];
    let audio_offsets = [mdat_data_offset + 15, mdat_data_offset + 18];

    let moov = build_movie_moov_with_user_metadata(&video_offsets, &audio_offsets);
    let mdat = encode_raw_box(fourcc("mdat"), &mdat_payload);
    [ftyp, moov, spherical_uuid, raw_uuid, mdat].concat()
}

fn build_movie_file_with_legacy_uuid_boxes() -> Vec<u8> {
    let ftyp = encode_supported_box(
        &Ftyp {
            major_brand: fourcc("isom"),
            minor_version: 0x0200,
            compatible_brands: vec![fourcc("isom"), fourcc("iso2"), fourcc("avc1")],
        },
        &[],
    );

    let placeholder_moov = build_movie_moov(&[0, 0], &[0, 0]);
    let fragment_timing_uuid = build_fragment_timing_uuid_box();
    let fragment_run_uuid = build_fragment_run_uuid_box();
    let sample_encryption_uuid = build_sample_encryption_uuid_box();
    let mdat_payload = movie_mdat_payload();
    let mdat_data_offset = ftyp.len() as u64
        + placeholder_moov.len() as u64
        + fragment_timing_uuid.len() as u64
        + fragment_run_uuid.len() as u64
        + sample_encryption_uuid.len() as u64
        + 8;
    let video_offsets = [mdat_data_offset, mdat_data_offset + 10];
    let audio_offsets = [mdat_data_offset + 15, mdat_data_offset + 18];

    let moov = build_movie_moov(&video_offsets, &audio_offsets);
    let mdat = encode_raw_box(fourcc("mdat"), &mdat_payload);
    [
        ftyp,
        moov,
        fragment_timing_uuid,
        fragment_run_uuid,
        sample_encryption_uuid,
        mdat,
    ]
    .concat()
}

fn build_movie_moov(video_offsets: &[u64; 2], audio_offsets: &[u64; 2]) -> Vec<u8> {
    let mut mvhd = Mvhd::default();
    mvhd.timescale = 1_000;
    mvhd.duration_v0 = 2_000;
    mvhd.rate = 1 << 16;
    mvhd.volume = 1 << 8;
    mvhd.next_track_id = 3;
    let mvhd = encode_supported_box(&mvhd, &[]);
    let video = build_video_trak(video_offsets);
    let audio = build_audio_trak(audio_offsets);
    encode_supported_box(&Moov, &[mvhd, video, audio].concat())
}

fn build_movie_moov_with_user_metadata(
    video_offsets: &[u64; 2],
    audio_offsets: &[u64; 2],
) -> Vec<u8> {
    let mut mvhd = Mvhd::default();
    mvhd.timescale = 1_000;
    mvhd.duration_v0 = 2_000;
    mvhd.rate = 1 << 16;
    mvhd.volume = 1 << 8;
    mvhd.next_track_id = 3;
    let mvhd = encode_supported_box(&mvhd, &[]);
    let video = build_video_trak(video_offsets);
    let audio = build_audio_trak(audio_offsets);
    let udta = build_movie_user_metadata_box();
    encode_supported_box(&Moov, &[mvhd, video, audio, udta].concat())
}

fn build_movie_moov_with_extended_language(video_offsets: &[u64; 2]) -> Vec<u8> {
    let mut mvhd = Mvhd::default();
    mvhd.timescale = 1_000;
    mvhd.duration_v0 = 2_000;
    mvhd.rate = 1 << 16;
    mvhd.volume = 1 << 8;
    mvhd.next_track_id = 2;
    let mvhd = encode_supported_box(&mvhd, &[]);
    let video = build_video_trak_with_extended_language(video_offsets);
    encode_supported_box(&Moov, &[mvhd, video].concat())
}

fn build_video_trak(chunk_offsets: &[u64; 2]) -> Vec<u8> {
    let mut tkhd = Tkhd::default();
    tkhd.track_id = 1;
    tkhd.duration_v0 = 3_072;
    tkhd.width = u32::from(320_u16) << 16;
    tkhd.height = u32::from(180_u16) << 16;
    let tkhd = encode_supported_box(&tkhd, &[]);

    let mut elst = Elst::default();
    elst.entry_count = 1;
    elst.entries = vec![ElstEntry {
        segment_duration_v0: 1_024,
        media_time_v0: 2_048,
        media_rate_integer: 1,
        ..ElstEntry::default()
    }];
    let edts = encode_supported_box(&Edts, &encode_supported_box(&elst, &[]));

    let mut mdhd = Mdhd::default();
    mdhd.timescale = 90_000;
    mdhd.duration_v0 = 3_072;
    mdhd.language = [5, 14, 7];
    let mdhd = encode_supported_box(&mdhd, &[]);
    let hdlr = handler_box("vide", "VideoHandler");

    let mut stsd = Stsd::default();
    stsd.entry_count = 1;
    let avc1 = encode_supported_box(
        &video_sample_entry(),
        &encode_supported_box(&avc_config(), &[]),
    );
    let stsd = encode_supported_box(&stsd, &avc1);

    let mut stco = Stco::default();
    stco.entry_count = 2;
    stco.chunk_offset = chunk_offsets.to_vec();
    let stco = encode_supported_box(&stco, &[]);

    let mut stts = Stts::default();
    stts.entry_count = 1;
    stts.entries = vec![SttsEntry {
        sample_count: 3,
        sample_delta: 1_024,
    }];
    let stts = encode_supported_box(&stts, &[]);

    let mut ctts = Ctts::default();
    ctts.entry_count = 2;
    ctts.entries = vec![
        CttsEntry {
            sample_count: 2,
            sample_offset_v0: 256,
            ..CttsEntry::default()
        },
        CttsEntry {
            sample_count: 1,
            sample_offset_v0: 128,
            ..CttsEntry::default()
        },
    ];
    let ctts = encode_supported_box(&ctts, &[]);

    let mut stsc = Stsc::default();
    stsc.entry_count = 2;
    stsc.entries = vec![
        StscEntry {
            first_chunk: 1,
            samples_per_chunk: 2,
            sample_description_index: 1,
        },
        StscEntry {
            first_chunk: 2,
            samples_per_chunk: 1,
            sample_description_index: 1,
        },
    ];
    let stsc = encode_supported_box(&stsc, &[]);

    let mut stsz = Stsz::default();
    stsz.sample_count = 3;
    stsz.entry_size = vec![5, 5, 5];
    let stsz = encode_supported_box(&stsz, &[]);

    let stbl = encode_supported_box(&Stbl, &[stsd, stco, stts, ctts, stsc, stsz].concat());
    let minf = encode_supported_box(&Minf, &stbl);
    let mdia = encode_supported_box(&Mdia, &[mdhd, hdlr, minf].concat());
    encode_supported_box(&Trak, &[tkhd, edts, mdia].concat())
}

fn build_video_trak_with_extended_language(chunk_offsets: &[u64; 2]) -> Vec<u8> {
    let mut tkhd = Tkhd::default();
    tkhd.track_id = 1;
    tkhd.duration_v0 = 3_072;
    tkhd.width = u32::from(320_u16) << 16;
    tkhd.height = u32::from(180_u16) << 16;
    let tkhd = encode_supported_box(&tkhd, &[]);

    let mut mdhd = Mdhd::default();
    mdhd.timescale = 90_000;
    mdhd.duration_v0 = 3_072;
    mdhd.language = [5, 14, 7];
    let mdhd = encode_supported_box(&mdhd, &[]);

    let mut elng = Elng::default();
    elng.extended_language = "en-US".into();
    let elng = encode_supported_box(&elng, &[]);

    let hdlr = handler_box("vide", "VideoHandler");

    let mut stsd = Stsd::default();
    stsd.entry_count = 1;
    let avc1 = encode_supported_box(
        &video_sample_entry(),
        &encode_supported_box(&avc_config(), &[]),
    );
    let stsd = encode_supported_box(&stsd, &avc1);

    let mut stco = Stco::default();
    stco.entry_count = 2;
    stco.chunk_offset = chunk_offsets.to_vec();
    let stco = encode_supported_box(&stco, &[]);

    let mut stts = Stts::default();
    stts.entry_count = 1;
    stts.entries = vec![SttsEntry {
        sample_count: 3,
        sample_delta: 1_024,
    }];
    let stts = encode_supported_box(&stts, &[]);

    let mut ctts = Ctts::default();
    ctts.entry_count = 2;
    ctts.entries = vec![
        CttsEntry {
            sample_count: 2,
            sample_offset_v0: 256,
            ..CttsEntry::default()
        },
        CttsEntry {
            sample_count: 1,
            sample_offset_v0: 128,
            ..CttsEntry::default()
        },
    ];
    let ctts = encode_supported_box(&ctts, &[]);

    let mut stsc = Stsc::default();
    stsc.entry_count = 2;
    stsc.entries = vec![
        StscEntry {
            first_chunk: 1,
            samples_per_chunk: 2,
            sample_description_index: 1,
        },
        StscEntry {
            first_chunk: 2,
            samples_per_chunk: 1,
            sample_description_index: 1,
        },
    ];
    let stsc = encode_supported_box(&stsc, &[]);

    let mut stsz = Stsz::default();
    stsz.sample_count = 3;
    stsz.entry_size = vec![5, 5, 5];
    let stsz = encode_supported_box(&stsz, &[]);

    let stbl = encode_supported_box(&Stbl, &[stsd, stco, stts, ctts, stsc, stsz].concat());
    let minf = encode_supported_box(&Minf, &stbl);
    let mdia = encode_supported_box(&Mdia, &[mdhd, elng, hdlr, minf].concat());
    encode_supported_box(&Trak, &[tkhd, mdia].concat())
}

fn build_audio_trak(chunk_offsets: &[u64; 2]) -> Vec<u8> {
    let mut tkhd = Tkhd::default();
    tkhd.track_id = 2;
    tkhd.duration_v0 = 2_048;
    let tkhd = encode_supported_box(&tkhd, &[]);

    let mut mdhd = Mdhd::default();
    mdhd.timescale = 48_000;
    mdhd.duration_v0 = 2_048;
    mdhd.language = [5, 14, 7];
    let mdhd = encode_supported_box(&mdhd, &[]);
    let hdlr = handler_box("soun", "SoundHandler");

    let mut stsd = Stsd::default();
    stsd.entry_count = 1;
    let mp4a = encode_supported_box(
        &audio_sample_entry(),
        &encode_supported_box(&aac_profile_esds(0x40, &[0x10, 0x00]), &[]),
    );
    let stsd = encode_supported_box(&stsd, &mp4a);

    let mut stco = Stco::default();
    stco.entry_count = 2;
    stco.chunk_offset = chunk_offsets.to_vec();
    let stco = encode_supported_box(&stco, &[]);

    let mut stts = Stts::default();
    stts.entry_count = 1;
    stts.entries = vec![SttsEntry {
        sample_count: 2,
        sample_delta: 1_024,
    }];
    let stts = encode_supported_box(&stts, &[]);

    let mut stsc = Stsc::default();
    stsc.entry_count = 1;
    stsc.entries = vec![StscEntry {
        first_chunk: 1,
        samples_per_chunk: 1,
        sample_description_index: 1,
    }];
    let stsc = encode_supported_box(&stsc, &[]);

    let mut stsz = Stsz::default();
    stsz.sample_count = 2;
    stsz.entry_size = vec![3, 4];
    let stsz = encode_supported_box(&stsz, &[]);

    let stbl = encode_supported_box(&Stbl, &[stsd, stco, stts, stsc, stsz].concat());
    let minf = encode_supported_box(&Minf, &stbl);
    let mdia = encode_supported_box(&Mdia, &[mdhd, hdlr, minf].concat());
    encode_supported_box(&Trak, &[tkhd, mdia].concat())
}

fn build_movie_user_metadata_box() -> Vec<u8> {
    let mut id32 = Id32::default();
    id32.language = "eng".into();
    id32.id3v2_data = b"ID3\x04".to_vec();
    let id32 = encode_supported_box(&id32, &[]);
    let meta = encode_supported_box(&Meta::default(), &id32);

    let mut track_loudness = TrackLoudnessInfo::default();
    track_loudness.set_version(1);
    track_loudness.entries = vec![LoudnessEntry {
        eq_set_id: 7,
        downmix_id: 12,
        drc_set_id: 18,
        bs_sample_peak_level: 528,
        bs_true_peak_level: 801,
        measurement_system_for_tp: 4,
        reliability_for_tp: 6,
        measurements: vec![LoudnessMeasurement {
            method_definition: 7,
            method_value: 8,
            measurement_system: 9,
            reliability: 10,
        }],
    }];
    let track_loudness = encode_supported_box(&track_loudness, &[]);

    let mut album_loudness = AlbumLoudnessInfo::default();
    album_loudness.set_version(0);
    album_loudness.entries = vec![LoudnessEntry {
        downmix_id: 9,
        drc_set_id: 17,
        bs_sample_peak_level: 274,
        bs_true_peak_level: 291,
        measurement_system_for_tp: 2,
        reliability_for_tp: 3,
        measurements: vec![LoudnessMeasurement {
            method_definition: 1,
            method_value: 2,
            measurement_system: 4,
            reliability: 5,
        }],
        ..LoudnessEntry::default()
    }];
    let album_loudness = encode_supported_box(&album_loudness, &[]);
    let loudness = encode_supported_box(&Ludt, &[track_loudness, album_loudness].concat());

    encode_supported_box(&Udta, &[meta, loudness].concat())
}

fn build_spherical_uuid_box() -> Vec<u8> {
    encode_supported_box(
        &Uuid {
            user_type: UUID_SPHERICAL_VIDEO_V1,
            payload: UuidPayload::SphericalVideoV1(SphericalVideoV1Metadata {
                xml_data: b"<rdf>S</rdf>".to_vec(),
            }),
        },
        &[],
    )
}

fn build_raw_uuid_box() -> Vec<u8> {
    encode_supported_box(
        &Uuid {
            user_type: [
                0x10, 0x32, 0x54, 0x76, 0x98, 0xba, 0xdc, 0xfe, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
                0xcd, 0xef,
            ],
            payload: UuidPayload::Raw(vec![0xde, 0xad, 0xbe]),
        },
        &[],
    )
}

fn build_fragment_timing_uuid_box() -> Vec<u8> {
    encode_supported_box(
        &Uuid {
            user_type: UUID_FRAGMENT_ABSOLUTE_TIMING,
            payload: UuidPayload::FragmentAbsoluteTiming(UuidFragmentAbsoluteTiming {
                version: 1,
                flags: 0,
                fragment_absolute_time: 0x0001_05c6_49bd_a400,
                fragment_absolute_duration: 0x0000_0000_0005_4600,
            }),
        },
        &[],
    )
}

fn build_fragment_run_uuid_box() -> Vec<u8> {
    encode_supported_box(
        &Uuid {
            user_type: UUID_FRAGMENT_RUN_TABLE,
            payload: UuidPayload::FragmentRunTable(UuidFragmentRunTable {
                version: 1,
                flags: 0,
                fragment_count: 1,
                entries: vec![UuidFragmentRunEntry {
                    fragment_absolute_time: 0x0001_05c6_49c2_ea00,
                    fragment_absolute_duration: 0x0000_0000_0005_4600,
                }],
            }),
        },
        &[],
    )
}

fn build_sample_encryption_uuid_box() -> Vec<u8> {
    let mut sample_encryption = Senc::default();
    sample_encryption.set_version(0);
    sample_encryption.set_flags(SENC_USE_SUBSAMPLE_ENCRYPTION);
    sample_encryption.sample_count = 1;
    sample_encryption.samples = vec![SencSample {
        initialization_vector: vec![1, 2, 3, 4, 5, 6, 7, 8],
        subsamples: vec![SencSubsample {
            bytes_of_clear_data: 5,
            bytes_of_protected_data: 16,
        }],
    }];

    encode_supported_box(
        &Uuid {
            user_type: UUID_SAMPLE_ENCRYPTION,
            payload: UuidPayload::SampleEncryption(sample_encryption),
        },
        &[],
    )
}

fn build_fragment_file() -> Vec<u8> {
    let ftyp = encode_supported_box(
        &Ftyp {
            major_brand: fourcc("iso6"),
            minor_version: 1,
            compatible_brands: vec![fourcc("iso6"), fourcc("dash")],
        },
        &[],
    );
    let moof_one = build_fragment_moof_one();
    let moof_two = build_fragment_moof_two();
    [ftyp, moof_one, moof_two].concat()
}

fn build_fragment_file_with_prft() -> Vec<u8> {
    let ftyp = encode_supported_box(
        &Ftyp {
            major_brand: fourcc("iso6"),
            minor_version: 1,
            compatible_brands: vec![fourcc("iso6"), fourcc("dash")],
        },
        &[],
    );
    let prft_one = build_prft_box_v0(7, 0x0000_0001_0203_0405, 9_000);
    let prft_two = build_prft_box_v0(7, 0x0000_0006_0708_090a, 12_000);
    let moof_one = build_fragment_moof_one();
    let moof_two = build_fragment_moof_two();
    [ftyp, prft_one, moof_one, prft_two, moof_two].concat()
}

fn build_prft_box_v0(reference_track_id: u32, ntp_timestamp: u64, media_time_v0: u32) -> Vec<u8> {
    let mut prft = Prft::default();
    prft.reference_track_id = reference_track_id;
    prft.ntp_timestamp = ntp_timestamp;
    prft.media_time_v0 = media_time_v0;
    encode_supported_box(&prft, &[])
}

fn build_fragment_moof_one() -> Vec<u8> {
    let tfhd = {
        let mut tfhd = Tfhd::default();
        tfhd.track_id = 7;
        tfhd.default_sample_duration = 1_000;
        tfhd.default_sample_size = 9;
        tfhd.set_flags(TFHD_DEFAULT_SAMPLE_DURATION_PRESENT | TFHD_DEFAULT_SAMPLE_SIZE_PRESENT);
        encode_supported_box(&tfhd, &[])
    };

    let mut tfdt = Tfdt::default();
    tfdt.base_media_decode_time_v0 = 9_000;
    let tfdt = encode_supported_box(&tfdt, &[]);

    let trun = {
        let mut trun = Trun::default();
        trun.sample_count = 2;
        trun.entries = vec![
            TrunEntry {
                sample_duration: 1_000,
                sample_size: 4,
                sample_composition_time_offset_v0: 500,
                ..TrunEntry::default()
            },
            TrunEntry {
                sample_duration: 2_000,
                sample_size: 6,
                sample_composition_time_offset_v0: 100,
                ..TrunEntry::default()
            },
        ];
        trun.set_flags(
            TRUN_SAMPLE_DURATION_PRESENT
                | TRUN_SAMPLE_SIZE_PRESENT
                | TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT,
        );
        encode_supported_box(&trun, &[])
    };

    let traf = encode_supported_box(&Traf, &[tfhd, tfdt, trun].concat());
    encode_supported_box(&Moof, &traf)
}

fn build_fragment_moof_two() -> Vec<u8> {
    let tfhd = {
        let mut tfhd = Tfhd::default();
        tfhd.track_id = 7;
        tfhd.default_sample_duration = 1_024;
        tfhd.default_sample_size = 12;
        tfhd.set_flags(TFHD_DEFAULT_SAMPLE_DURATION_PRESENT | TFHD_DEFAULT_SAMPLE_SIZE_PRESENT);
        encode_supported_box(&tfhd, &[])
    };

    let mut tfdt = Tfdt::default();
    tfdt.base_media_decode_time_v0 = 12_000;
    let tfdt = encode_supported_box(&tfdt, &[]);

    let mut trun = Trun::default();
    trun.sample_count = 3;
    let trun = encode_supported_box(&trun, &[]);

    let traf = encode_supported_box(&Traf, &[tfhd, tfdt, trun].concat());
    encode_supported_box(&Moof, &traf)
}

fn build_av01_movie_file() -> Vec<u8> {
    let ftyp = encode_supported_box(
        &Ftyp {
            major_brand: fourcc("isom"),
            minor_version: 0x0200,
            compatible_brands: vec![fourcc("isom"), fourcc("iso8"), fourcc("av01")],
        },
        &[],
    );

    let placeholder_moov = build_av01_moov(&[0]);
    let mdat_payload = vec![0x12, 0x34, 0x56, 0x78];
    let mdat_data_offset = ftyp.len() as u64 + placeholder_moov.len() as u64 + 8;
    let moov = build_av01_moov(&[mdat_data_offset]);
    let mdat = encode_raw_box(fourcc("mdat"), &mdat_payload);
    [ftyp, moov, mdat].concat()
}

fn build_av01_moov(chunk_offsets: &[u64; 1]) -> Vec<u8> {
    let mut mvhd = Mvhd::default();
    mvhd.timescale = 1_000;
    mvhd.duration_v0 = 1_000;
    mvhd.rate = 1 << 16;
    mvhd.volume = 1 << 8;
    mvhd.next_track_id = 2;
    let mvhd = encode_supported_box(&mvhd, &[]);
    let video = build_av01_trak(chunk_offsets);
    encode_supported_box(&Moov, &[mvhd, video].concat())
}

fn build_av01_trak(chunk_offsets: &[u64; 1]) -> Vec<u8> {
    let mut tkhd = Tkhd::default();
    tkhd.track_id = 1;
    tkhd.duration_v0 = 1_000;
    tkhd.width = u32::from(640_u16) << 16;
    tkhd.height = u32::from(360_u16) << 16;
    let tkhd = encode_supported_box(&tkhd, &[]);

    let mut mdhd = Mdhd::default();
    mdhd.timescale = 1_000;
    mdhd.duration_v0 = 1_000;
    mdhd.language = [5, 14, 7];
    let mdhd = encode_supported_box(&mdhd, &[]);
    let hdlr = handler_box("vide", "VideoHandler");

    let mut stsd = Stsd::default();
    stsd.entry_count = 1;
    let av01 = encode_supported_box(
        &video_sample_entry_with_type("av01", 640, 360),
        &encode_supported_box(&av1_config(), &[]),
    );
    let stsd = encode_supported_box(&stsd, &av01);

    let mut stco = Stco::default();
    stco.entry_count = 1;
    stco.chunk_offset = chunk_offsets.to_vec();
    let stco = encode_supported_box(&stco, &[]);

    let mut stts = Stts::default();
    stts.entry_count = 1;
    stts.entries = vec![SttsEntry {
        sample_count: 1,
        sample_delta: 1_000,
    }];
    let stts = encode_supported_box(&stts, &[]);

    let mut stsc = Stsc::default();
    stsc.entry_count = 1;
    stsc.entries = vec![StscEntry {
        first_chunk: 1,
        samples_per_chunk: 1,
        sample_description_index: 1,
    }];
    let stsc = encode_supported_box(&stsc, &[]);

    let mut stsz = Stsz::default();
    stsz.sample_count = 1;
    stsz.entry_size = vec![4];
    let stsz = encode_supported_box(&stsz, &[]);

    let stbl = encode_supported_box(&Stbl, &[stsd, stco, stts, stsc, stsz].concat());
    let minf = encode_supported_box(&Minf, &stbl);
    let mdia = encode_supported_box(&Mdia, &[mdhd, hdlr, minf].concat());
    encode_supported_box(&Trak, &[tkhd, mdia].concat())
}

fn build_encrypted_video_movie_file() -> Vec<u8> {
    let ftyp = encode_supported_box(
        &Ftyp {
            major_brand: fourcc("iso6"),
            minor_version: 1,
            compatible_brands: vec![fourcc("iso6"), fourcc("dash"), fourcc("cenc")],
        },
        &[],
    );

    let placeholder_moov = build_encrypted_video_moov(&[0]);
    let mdat_payload = [avc_sample(5)].concat();
    let mdat_data_offset = ftyp.len() as u64 + placeholder_moov.len() as u64 + 8;
    let moov = build_encrypted_video_moov(&[mdat_data_offset]);
    let mdat = encode_raw_box(fourcc("mdat"), &mdat_payload);
    [ftyp, moov, mdat].concat()
}

fn build_encrypted_video_moov(chunk_offsets: &[u64; 1]) -> Vec<u8> {
    let mut mvhd = Mvhd::default();
    mvhd.timescale = 1_000;
    mvhd.duration_v0 = 1_000;
    mvhd.rate = 1 << 16;
    mvhd.volume = 1 << 8;
    mvhd.next_track_id = 2;
    let mvhd = encode_supported_box(&mvhd, &[]);
    let video = build_encrypted_video_trak(chunk_offsets);
    encode_supported_box(&Moov, &[mvhd, video].concat())
}

fn build_encrypted_video_trak(chunk_offsets: &[u64; 1]) -> Vec<u8> {
    let mut tkhd = Tkhd::default();
    tkhd.track_id = 1;
    tkhd.duration_v0 = 1_000;
    tkhd.width = u32::from(320_u16) << 16;
    tkhd.height = u32::from(180_u16) << 16;
    let tkhd = encode_supported_box(&tkhd, &[]);

    let mut mdhd = Mdhd::default();
    mdhd.timescale = 1_000;
    mdhd.duration_v0 = 1_000;
    mdhd.language = [5, 14, 7];
    let mdhd = encode_supported_box(&mdhd, &[]);
    let hdlr = handler_box("vide", "VideoHandler");

    let mut schm = Schm::default();
    schm.set_version(0);
    schm.scheme_type = fourcc("cenc");
    schm.scheme_version = 0x0001_0000;
    let sinf = encode_supported_box(
        &Sinf,
        &[
            encode_supported_box(
                &Frma {
                    data_format: fourcc("avc1"),
                },
                &[],
            ),
            encode_supported_box(&schm, &[]),
        ]
        .concat(),
    );

    let mut stsd = Stsd::default();
    stsd.entry_count = 1;
    let encv = encode_supported_box(
        &video_sample_entry_with_type("encv", 320, 180),
        &[encode_supported_box(&avc_config(), &[]), sinf].concat(),
    );
    let stsd = encode_supported_box(&stsd, &encv);

    let mut stco = Stco::default();
    stco.entry_count = 1;
    stco.chunk_offset = chunk_offsets.to_vec();
    let stco = encode_supported_box(&stco, &[]);

    let mut stts = Stts::default();
    stts.entry_count = 1;
    stts.entries = vec![SttsEntry {
        sample_count: 1,
        sample_delta: 1_000,
    }];
    let stts = encode_supported_box(&stts, &[]);

    let mut stsc = Stsc::default();
    stsc.entry_count = 1;
    stsc.entries = vec![StscEntry {
        first_chunk: 1,
        samples_per_chunk: 1,
        sample_description_index: 1,
    }];
    let stsc = encode_supported_box(&stsc, &[]);

    let mut stsz = Stsz::default();
    stsz.sample_count = 1;
    stsz.entry_size = vec![5];
    let stsz = encode_supported_box(&stsz, &[]);

    let stbl = encode_supported_box(&Stbl, &[stsd, stco, stts, stsc, stsz].concat());
    let minf = encode_supported_box(&Minf, &stbl);
    let mdia = encode_supported_box(&Mdia, &[mdhd, hdlr, minf].concat());
    encode_supported_box(&Trak, &[tkhd, mdia].concat())
}

fn build_single_track_movie_file(
    compatible_brands: Vec<FourCc>,
    track_builder: fn(&[u64; 1]) -> Vec<u8>,
    mdat_payload: Vec<u8>,
) -> Vec<u8> {
    let ftyp = encode_supported_box(
        &Ftyp {
            major_brand: fourcc("isom"),
            minor_version: 0x0200,
            compatible_brands,
        },
        &[],
    );

    let placeholder_moov = build_single_track_moov(track_builder(&[0]));
    let mdat_data_offset = ftyp.len() as u64 + placeholder_moov.len() as u64 + 8;
    let moov = build_single_track_moov(track_builder(&[mdat_data_offset]));
    let mdat = encode_raw_box(fourcc("mdat"), &mdat_payload);
    [ftyp, moov, mdat].concat()
}

fn build_single_track_moov(track: Vec<u8>) -> Vec<u8> {
    let mut mvhd = Mvhd::default();
    mvhd.timescale = 1_000;
    mvhd.duration_v0 = 1_000;
    mvhd.rate = 1 << 16;
    mvhd.volume = 1 << 8;
    mvhd.next_track_id = 2;
    let mvhd = encode_supported_box(&mvhd, &[]);
    encode_supported_box(&Moov, &[mvhd, track].concat())
}

fn build_hevc_movie_file() -> Vec<u8> {
    build_single_track_movie_file(
        vec![fourcc("isom"), fourcc("iso8"), fourcc("hvc1")],
        build_hevc_trak,
        vec![0x12, 0x34, 0x56, 0x78],
    )
}

fn build_hevc_trak(chunk_offsets: &[u64; 1]) -> Vec<u8> {
    let sample_entry = encode_supported_box(
        &video_sample_entry_with_type("hvc1", 640, 360),
        &encode_supported_box(&hevc_config(), &[]),
    );
    build_single_sample_video_trak(1, 1_000, 1_000, (640, 360), sample_entry, chunk_offsets, 4)
}

fn build_vp09_movie_file() -> Vec<u8> {
    build_single_track_movie_file(
        vec![fourcc("isom"), fourcc("iso8"), fourcc("vp09")],
        build_vp09_trak,
        vec![0xaa, 0xbb, 0xcc, 0xdd],
    )
}

fn build_vp09_trak(chunk_offsets: &[u64; 1]) -> Vec<u8> {
    let sample_entry = encode_supported_box(
        &video_sample_entry_with_type("vp09", 640, 360),
        &encode_supported_box(&vp9_config(), &[]),
    );
    build_single_sample_video_trak(1, 1_000, 1_000, (640, 360), sample_entry, chunk_offsets, 4)
}

fn build_opus_movie_file() -> Vec<u8> {
    build_single_track_movie_file(
        vec![fourcc("isom"), fourcc("iso8"), fourcc("Opus")],
        build_opus_trak,
        vec![0x11, 0x22, 0x33, 0x44],
    )
}

fn build_opus_trak(chunk_offsets: &[u64; 1]) -> Vec<u8> {
    let sample_entry = encode_supported_box(
        &audio_sample_entry_with_type("Opus", 2, 48_000),
        &encode_supported_box(&opus_config(), &[]),
    );
    build_single_sample_audio_trak(1, 48_000, 1_024, sample_entry, chunk_offsets, 4)
}

fn build_ac3_movie_file() -> Vec<u8> {
    build_single_track_movie_file(
        vec![fourcc("isom"), fourcc("iso8"), fourcc("ac-3")],
        build_ac3_trak,
        vec![0x21, 0x22, 0x23, 0x24],
    )
}

fn build_ac3_trak(chunk_offsets: &[u64; 1]) -> Vec<u8> {
    let sample_entry = encode_supported_box(
        &audio_sample_entry_with_type("ac-3", 6, 48_000),
        &encode_supported_box(&ac3_config(), &[]),
    );
    build_single_sample_audio_trak(1, 48_000, 1_536, sample_entry, chunk_offsets, 4)
}

fn build_ec3_movie_file() -> Vec<u8> {
    build_single_track_movie_file(
        vec![fourcc("isom"), fourcc("iso8"), fourcc("ec-3")],
        build_ec3_trak,
        vec![0x25, 0x26, 0x27, 0x28],
    )
}

fn build_ec3_trak(chunk_offsets: &[u64; 1]) -> Vec<u8> {
    let sample_entry = encode_supported_box(
        &audio_sample_entry_with_type("ec-3", 6, 48_000),
        &encode_supported_box(
            &Dec3 {
                data_rate: 448,
                num_ind_sub: 0,
                ec3_substreams: vec![Ec3Substream {
                    fscod: 2,
                    bsid: 0x10,
                    acmod: 4,
                    ..Ec3Substream::default()
                }],
                reserved: Vec::new(),
            },
            &[],
        ),
    );
    build_single_sample_audio_trak(1, 48_000, 1_536, sample_entry, chunk_offsets, 4)
}

fn build_ac4_movie_file() -> Vec<u8> {
    build_single_track_movie_file(
        vec![fourcc("isom"), fourcc("iso8"), fourcc("ac-4")],
        build_ac4_trak,
        vec![0x29, 0x2a, 0x2b, 0x2c],
    )
}

fn build_ac4_trak(chunk_offsets: &[u64; 1]) -> Vec<u8> {
    let sample_entry = encode_supported_box(
        &audio_sample_entry_with_type("ac-4", 2, 48_000),
        &encode_supported_box(
            &Dac4 {
                data: vec![
                    0x22, 0x00, 0x80, 0x01, 0xf4, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                ],
            },
            &[],
        ),
    );
    build_single_sample_audio_trak(1, 48_000, 1_024, sample_entry, chunk_offsets, 4)
}

fn build_pcm_movie_file() -> Vec<u8> {
    build_single_track_movie_file(
        vec![fourcc("isom"), fourcc("iso8"), fourcc("ipcm")],
        build_pcm_trak,
        vec![0x31, 0x32, 0x33, 0x34],
    )
}

fn build_pcm_trak(chunk_offsets: &[u64; 1]) -> Vec<u8> {
    let sample_entry = encode_supported_box(
        &audio_sample_entry_with_type("ipcm", 2, 48_000),
        &encode_supported_box(&pcm_config(), &[]),
    );
    build_single_sample_audio_trak(1, 48_000, 1_024, sample_entry, chunk_offsets, 4)
}

fn build_stpp_movie_file() -> Vec<u8> {
    build_single_track_movie_file(
        vec![fourcc("isom"), fourcc("iso8"), fourcc("stpp")],
        build_stpp_trak,
        vec![0x41, 0x42, 0x43, 0x44],
    )
}

fn build_stpp_trak(chunk_offsets: &[u64; 1]) -> Vec<u8> {
    let sample_entry = encode_supported_box(&xml_subtitle_sample_entry(), &[]);
    build_single_sample_subtitle_trak(
        1,
        1_000,
        1_000,
        subtitle_media_header_box(),
        sample_entry,
        chunk_offsets,
        4,
    )
}

fn build_sbtt_movie_file() -> Vec<u8> {
    build_single_track_movie_file(
        vec![fourcc("isom"), fourcc("iso8"), fourcc("sbtt")],
        build_sbtt_trak,
        vec![0x51, 0x52, 0x53, 0x54],
    )
}

fn build_sbtt_trak(chunk_offsets: &[u64; 1]) -> Vec<u8> {
    let sample_entry = encode_supported_box(&text_subtitle_sample_entry(), &[]);
    build_single_sample_subtitle_trak(
        1,
        1_000,
        1_000,
        subtitle_media_header_box(),
        sample_entry,
        chunk_offsets,
        4,
    )
}

fn build_wvtt_movie_file() -> Vec<u8> {
    build_single_track_movie_file(
        vec![fourcc("isom"), fourcc("iso8"), fourcc("wvtt")],
        build_wvtt_trak,
        vec![0x61, 0x62, 0x63, 0x64],
    )
}

fn build_wvtt_trak(chunk_offsets: &[u64; 1]) -> Vec<u8> {
    let sample_entry = encode_supported_box(
        &wvtt_sample_entry(),
        &[
            encode_supported_box(
                &WebVTTConfigurationBox {
                    config: "WEBVTT".to_string(),
                },
                &[],
            ),
            encode_supported_box(
                &WebVTTSourceLabelBox {
                    source_label: "eng".to_string(),
                },
                &[],
            ),
        ]
        .concat(),
    );
    build_single_sample_subtitle_trak(
        1,
        1_000,
        1_000,
        null_media_header_box(),
        sample_entry,
        chunk_offsets,
        4,
    )
}

fn build_encrypted_hevc_movie_file() -> Vec<u8> {
    build_single_track_movie_file(
        vec![fourcc("iso6"), fourcc("dash"), fourcc("cenc")],
        build_encrypted_hevc_trak,
        vec![0x71, 0x72, 0x73, 0x74],
    )
}

fn build_encrypted_hevc_trak(chunk_offsets: &[u64; 1]) -> Vec<u8> {
    let mut schm = Schm::default();
    schm.set_version(0);
    schm.scheme_type = fourcc("cenc");
    schm.scheme_version = 0x0001_0000;
    let sinf = encode_supported_box(
        &Sinf,
        &[
            encode_supported_box(
                &Frma {
                    data_format: fourcc("hvc1"),
                },
                &[],
            ),
            encode_supported_box(&schm, &[]),
        ]
        .concat(),
    );

    let sample_entry = encode_supported_box(
        &video_sample_entry_with_type("encv", 640, 360),
        &[encode_supported_box(&hevc_config(), &[]), sinf].concat(),
    );
    build_single_sample_video_trak(1, 1_000, 1_000, (640, 360), sample_entry, chunk_offsets, 4)
}

fn build_vvc_movie_file() -> Vec<u8> {
    build_single_track_movie_file(
        vec![fourcc("isom"), fourcc("iso8"), fourcc("vvc1")],
        build_vvc_trak,
        vec![0x75, 0x76, 0x77, 0x78],
    )
}

fn build_vvc_trak(chunk_offsets: &[u64; 1]) -> Vec<u8> {
    let sample_entry = encode_supported_box(
        &video_sample_entry_with_type("vvc1", 640, 360),
        &encode_supported_box(
            &{
                let mut vvcc = VVCDecoderConfiguration::default();
                vvcc.set_version(0);
                vvcc.decoder_configuration_record = vec![0x01, 0x23, 0x45, 0x67, 0x89];
                vvcc
            },
            &[],
        ),
    );
    build_single_sample_video_trak(1, 1_000, 1_000, (640, 360), sample_entry, chunk_offsets, 4)
}

fn build_avs3_movie_file() -> Vec<u8> {
    build_single_track_movie_file(
        vec![fourcc("isom"), fourcc("iso8"), fourcc("avs3")],
        build_avs3_trak,
        vec![0x85, 0x86, 0x87, 0x88],
    )
}

fn build_avs3_trak(chunk_offsets: &[u64; 1]) -> Vec<u8> {
    let sample_entry = encode_supported_box(
        &video_sample_entry_with_type("avs3", 640, 360),
        &encode_supported_box(&avs3_config(), &[]),
    );
    build_single_sample_video_trak(1, 1_000, 1_000, (640, 360), sample_entry, chunk_offsets, 4)
}

fn build_flac_movie_file() -> Vec<u8> {
    build_single_track_movie_file(
        vec![fourcc("isom"), fourcc("iso8"), fourcc("fLaC")],
        build_flac_trak,
        vec![0x89, 0x8a, 0x8b, 0x8c],
    )
}

fn build_flac_trak(chunk_offsets: &[u64; 1]) -> Vec<u8> {
    let sample_entry = encode_supported_box(
        &audio_sample_entry_with_type("fLaC", 2, 48_000),
        &encode_supported_box(&flac_config(), &[]),
    );
    build_single_sample_audio_trak(1, 48_000, 1_024, sample_entry, chunk_offsets, 4)
}

fn build_mha1_movie_file() -> Vec<u8> {
    build_single_track_movie_file(
        vec![fourcc("isom"), fourcc("iso8"), fourcc("mha1")],
        build_mha1_trak,
        vec![0x8d, 0x8e, 0x8f, 0x90],
    )
}

fn build_mha1_trak(chunk_offsets: &[u64; 1]) -> Vec<u8> {
    let sample_entry = encode_supported_box(
        &audio_sample_entry_with_type("mha1", 2, 48_000),
        &encode_supported_box(&mha_config(), &[]),
    );
    build_single_sample_audio_trak(1, 48_000, 1_024, sample_entry, chunk_offsets, 4)
}

fn build_media_characteristics_movie_file() -> Vec<u8> {
    build_single_track_movie_file(
        vec![fourcc("isom"), fourcc("iso8"), fourcc("avc1")],
        build_media_characteristics_trak,
        vec![0x81, 0x82, 0x83, 0x84],
    )
}

fn build_media_characteristics_trak(chunk_offsets: &[u64; 1]) -> Vec<u8> {
    let sample_entry = encode_supported_box(
        &video_sample_entry_with_type("avc1", 640, 360),
        &[
            encode_supported_box(&avc_config(), &[]),
            encode_supported_box(
                &Btrt {
                    buffer_size_db: 32_768,
                    max_bitrate: 4_000_000,
                    avg_bitrate: 2_500_000,
                },
                &[],
            ),
            encode_supported_box(
                &Clap {
                    clean_aperture_width_n: 1_920,
                    clean_aperture_width_d: 1,
                    clean_aperture_height_n: 1_080,
                    clean_aperture_height_d: 1,
                    horiz_off_n: 0,
                    horiz_off_d: 1,
                    vert_off_n: 0,
                    vert_off_d: 1,
                },
                &[],
            ),
            encode_supported_box(
                &{
                    let mut coll = CoLL::default();
                    coll.set_version(0);
                    coll.max_cll = 1_000;
                    coll.max_fall = 400;
                    coll
                },
                &[],
            ),
            encode_supported_box(
                &Colr {
                    colour_type: fourcc("nclx"),
                    colour_primaries: 9,
                    transfer_characteristics: 16,
                    matrix_coefficients: 9,
                    full_range_flag: true,
                    reserved: 0,
                    profile: Vec::new(),
                    unknown: Vec::new(),
                },
                &[],
            ),
            encode_supported_box(
                &Pasp {
                    h_spacing: 4,
                    v_spacing: 3,
                },
                &[],
            ),
            encode_supported_box(
                &Fiel {
                    field_count: 2,
                    field_ordering: 6,
                },
                &[],
            ),
            encode_supported_box(
                &{
                    let mut smdm = SmDm::default();
                    smdm.set_version(0);
                    smdm.primary_r_chromaticity_x = 34_000;
                    smdm.primary_r_chromaticity_y = 16_000;
                    smdm.primary_g_chromaticity_x = 13_250;
                    smdm.primary_g_chromaticity_y = 34_500;
                    smdm.primary_b_chromaticity_x = 7_500;
                    smdm.primary_b_chromaticity_y = 3_000;
                    smdm.white_point_chromaticity_x = 15_635;
                    smdm.white_point_chromaticity_y = 16_450;
                    smdm.luminance_max = 1_000_000;
                    smdm.luminance_min = 50;
                    smdm
                },
                &[],
            ),
        ]
        .concat(),
    );
    build_single_sample_video_trak(1, 1_000, 1_000, (640, 360), sample_entry, chunk_offsets, 4)
}

fn build_single_sample_video_trak(
    track_id: u32,
    timescale: u32,
    duration: u32,
    dimensions: (u16, u16),
    sample_entry: Vec<u8>,
    chunk_offsets: &[u64; 1],
    sample_size: u32,
) -> Vec<u8> {
    let mut tkhd = Tkhd::default();
    tkhd.track_id = track_id;
    tkhd.duration_v0 = duration;
    tkhd.width = u32::from(dimensions.0) << 16;
    tkhd.height = u32::from(dimensions.1) << 16;
    let tkhd = encode_supported_box(&tkhd, &[]);

    let mut mdhd = Mdhd::default();
    mdhd.timescale = timescale;
    mdhd.duration_v0 = duration;
    mdhd.language = [5, 14, 7];
    let mdhd = encode_supported_box(&mdhd, &[]);
    let hdlr = handler_box("vide", "VideoHandler");

    let mut stsd = Stsd::default();
    stsd.entry_count = 1;
    let stsd = encode_supported_box(&stsd, &sample_entry);

    let mut stco = Stco::default();
    stco.entry_count = 1;
    stco.chunk_offset = chunk_offsets.to_vec();
    let stco = encode_supported_box(&stco, &[]);

    let mut stts = Stts::default();
    stts.entry_count = 1;
    stts.entries = vec![SttsEntry {
        sample_count: 1,
        sample_delta: duration,
    }];
    let stts = encode_supported_box(&stts, &[]);

    let mut stsc = Stsc::default();
    stsc.entry_count = 1;
    stsc.entries = vec![StscEntry {
        first_chunk: 1,
        samples_per_chunk: 1,
        sample_description_index: 1,
    }];
    let stsc = encode_supported_box(&stsc, &[]);

    let mut stsz = Stsz::default();
    stsz.sample_count = 1;
    stsz.entry_size = vec![u64::from(sample_size)];
    let stsz = encode_supported_box(&stsz, &[]);

    let stbl = encode_supported_box(&Stbl, &[stsd, stco, stts, stsc, stsz].concat());
    let minf = encode_supported_box(&Minf, &stbl);
    let mdia = encode_supported_box(&Mdia, &[mdhd, hdlr, minf].concat());
    encode_supported_box(&Trak, &[tkhd, mdia].concat())
}

fn build_single_sample_audio_trak(
    track_id: u32,
    timescale: u32,
    duration: u32,
    sample_entry: Vec<u8>,
    chunk_offsets: &[u64; 1],
    sample_size: u32,
) -> Vec<u8> {
    let mut tkhd = Tkhd::default();
    tkhd.track_id = track_id;
    tkhd.duration_v0 = duration;
    let tkhd = encode_supported_box(&tkhd, &[]);

    let mut mdhd = Mdhd::default();
    mdhd.timescale = timescale;
    mdhd.duration_v0 = duration;
    mdhd.language = [5, 14, 7];
    let mdhd = encode_supported_box(&mdhd, &[]);
    let hdlr = handler_box("soun", "SoundHandler");

    let mut stsd = Stsd::default();
    stsd.entry_count = 1;
    let stsd = encode_supported_box(&stsd, &sample_entry);

    let mut stco = Stco::default();
    stco.entry_count = 1;
    stco.chunk_offset = chunk_offsets.to_vec();
    let stco = encode_supported_box(&stco, &[]);

    let mut stts = Stts::default();
    stts.entry_count = 1;
    stts.entries = vec![SttsEntry {
        sample_count: 1,
        sample_delta: duration,
    }];
    let stts = encode_supported_box(&stts, &[]);

    let mut stsc = Stsc::default();
    stsc.entry_count = 1;
    stsc.entries = vec![StscEntry {
        first_chunk: 1,
        samples_per_chunk: 1,
        sample_description_index: 1,
    }];
    let stsc = encode_supported_box(&stsc, &[]);

    let mut stsz = Stsz::default();
    stsz.sample_count = 1;
    stsz.entry_size = vec![u64::from(sample_size)];
    let stsz = encode_supported_box(&stsz, &[]);

    let stbl = encode_supported_box(&Stbl, &[stsd, stco, stts, stsc, stsz].concat());
    let minf = encode_supported_box(&Minf, &stbl);
    let mdia = encode_supported_box(&Mdia, &[mdhd, hdlr, minf].concat());
    encode_supported_box(&Trak, &[tkhd, mdia].concat())
}

fn build_single_sample_subtitle_trak(
    track_id: u32,
    timescale: u32,
    duration: u32,
    media_header: Vec<u8>,
    sample_entry: Vec<u8>,
    chunk_offsets: &[u64; 1],
    sample_size: u32,
) -> Vec<u8> {
    let mut tkhd = Tkhd::default();
    tkhd.track_id = track_id;
    tkhd.duration_v0 = duration;
    let tkhd = encode_supported_box(&tkhd, &[]);

    let mut mdhd = Mdhd::default();
    mdhd.timescale = timescale;
    mdhd.duration_v0 = duration;
    mdhd.language = [5, 14, 7];
    let mdhd = encode_supported_box(&mdhd, &[]);
    let hdlr = handler_box("subt", "SubtitleHandler");

    let mut stsd = Stsd::default();
    stsd.entry_count = 1;
    let stsd = encode_supported_box(&stsd, &sample_entry);

    let mut stco = Stco::default();
    stco.entry_count = 1;
    stco.chunk_offset = chunk_offsets.to_vec();
    let stco = encode_supported_box(&stco, &[]);

    let mut stts = Stts::default();
    stts.entry_count = 1;
    stts.entries = vec![SttsEntry {
        sample_count: 1,
        sample_delta: duration,
    }];
    let stts = encode_supported_box(&stts, &[]);

    let mut stsc = Stsc::default();
    stsc.entry_count = 1;
    stsc.entries = vec![StscEntry {
        first_chunk: 1,
        samples_per_chunk: 1,
        sample_description_index: 1,
    }];
    let stsc = encode_supported_box(&stsc, &[]);

    let mut stsz = Stsz::default();
    stsz.sample_count = 1;
    stsz.entry_size = vec![u64::from(sample_size)];
    let stsz = encode_supported_box(&stsz, &[]);

    let stbl = encode_supported_box(&Stbl, &[stsd, stco, stts, stsc, stsz].concat());
    let minf = encode_supported_box(&Minf, &[media_header, stbl].concat());
    let mdia = encode_supported_box(&Mdia, &[mdhd, hdlr, minf].concat());
    encode_supported_box(&Trak, &[tkhd, mdia].concat())
}

fn subtitle_media_header_box() -> Vec<u8> {
    encode_supported_box(&Sthd::default(), &[])
}

fn null_media_header_box() -> Vec<u8> {
    encode_supported_box(&Nmhd::default(), &[])
}

fn avc_config() -> AVCDecoderConfiguration {
    AVCDecoderConfiguration {
        configuration_version: 1,
        profile: 0x64,
        profile_compatibility: 0,
        level: 0x1f,
        length_size_minus_one: 3,
        ..AVCDecoderConfiguration::default()
    }
}

fn hevc_config() -> HEVCDecoderConfiguration {
    let mut general_profile_compatibility = [false; 32];
    general_profile_compatibility[1] = true;

    HEVCDecoderConfiguration {
        configuration_version: 1,
        general_profile_space: 1,
        general_tier_flag: true,
        general_profile_idc: 2,
        general_profile_compatibility,
        general_constraint_indicator: [1, 2, 3, 4, 5, 6],
        general_level_idc: 120,
        min_spatial_segmentation_idc: 0,
        parallelism_type: 0,
        chroma_format_idc: 1,
        bit_depth_luma_minus8: 2,
        bit_depth_chroma_minus8: 2,
        avg_frame_rate: 30_000,
        constant_frame_rate: 0,
        num_temporal_layers: 1,
        temporal_id_nested: 1,
        length_size_minus_one: 3,
        num_of_nalu_arrays: 0,
        nalu_arrays: Vec::new(),
    }
}

fn av1_config() -> AV1CodecConfiguration {
    AV1CodecConfiguration {
        seq_profile: 0,
        seq_level_idx_0: 13,
        seq_tier_0: 1,
        high_bitdepth: 1,
        twelve_bit: 0,
        monochrome: 0,
        chroma_subsampling_x: 1,
        chroma_subsampling_y: 0,
        chroma_sample_position: 2,
        initial_presentation_delay_present: 1,
        initial_presentation_delay_minus_one: 3,
        config_obus: vec![0x12, 0x34, 0x56],
    }
}

fn vp9_config() -> VpCodecConfiguration {
    let mut config = VpCodecConfiguration::default();
    config.profile = 2;
    config.level = 31;
    config.bit_depth = 10;
    config.chroma_subsampling = 1;
    config.video_full_range_flag = 1;
    config.colour_primaries = 9;
    config.transfer_characteristics = 16;
    config.matrix_coefficients = 9;
    config.codec_initialization_data_size = 3;
    config.codec_initialization_data = vec![0x01, 0x02, 0x03];
    config
}

fn opus_config() -> DOps {
    DOps {
        version: 0,
        output_channel_count: 2,
        pre_skip: 312,
        input_sample_rate: 48_000,
        output_gain: 0,
        channel_mapping_family: 1,
        stream_count: 2,
        coupled_count: 1,
        channel_mapping: vec![0, 1],
    }
}

fn ac3_config() -> Dac3 {
    Dac3 {
        fscod: 1,
        bsid: 8,
        bsmod: 3,
        acmod: 7,
        lfe_on: 1,
        bit_rate_code: 10,
    }
}

fn pcm_config() -> PcmC {
    let mut config = PcmC::default();
    config.format_flags = 1;
    config.pcm_sample_size = 24;
    config
}

fn avs3_config() -> Av3c {
    Av3c {
        configuration_version: 1,
        sequence_header_length: 4,
        sequence_header: vec![0x01, 0x02, 0x03, 0x04],
        library_dependency_idc: 2,
    }
}

fn flac_config() -> DfLa {
    let mut config = DfLa::default();
    config.metadata_blocks = vec![FlacMetadataBlock {
        last_metadata_block_flag: true,
        block_type: 0,
        length: 34,
        block_data: vec![
            0x11, 0x22, 0x00, 0x10, 0x00, 0x10, 0x00, 0x0c, 0xac, 0x44, 0xf0, 0x00, 0x00, 0x00,
            0x64, 0x20, 0x00, 0x00, 0x0b, 0xb8, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0,
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
        ],
    }];
    config
}

fn mha_config() -> MhaC {
    MhaC {
        config_version: 1,
        mpeg_h_3da_profile_level_indication: 12,
        reference_channel_layout: 6,
        mpeg_h_3da_config_length: 4,
        mpeg_h_3da_config: vec![0x01, 0x02, 0x03, 0x04],
    }
}

fn handler_box(handler_type: &str, name: &str) -> Vec<u8> {
    let mut hdlr = Hdlr::default();
    hdlr.handler_type = fourcc(handler_type);
    hdlr.name = name.to_string();
    encode_supported_box(&hdlr, &[])
}

fn video_sample_entry_with_type(box_type: &str, width: u16, height: u16) -> VisualSampleEntry {
    let mut entry = VisualSampleEntry {
        sample_entry: SampleEntry {
            box_type: fourcc(box_type),
            data_reference_index: 1,
        },
        width,
        height,
        frame_count: 1,
        ..VisualSampleEntry::default()
    };
    entry.set_box_type(fourcc(box_type));
    entry
}

fn video_sample_entry() -> VisualSampleEntry {
    video_sample_entry_with_type("avc1", 320, 180)
}

fn audio_sample_entry() -> AudioSampleEntry {
    audio_sample_entry_with_type("mp4a", 2, 48_000)
}

fn audio_sample_entry_with_type(
    box_type: &str,
    channel_count: u16,
    sample_rate: u32,
) -> AudioSampleEntry {
    let mut entry = AudioSampleEntry {
        sample_entry: SampleEntry {
            box_type: fourcc(box_type),
            data_reference_index: 1,
        },
        channel_count,
        sample_size: 16,
        sample_rate: sample_rate << 16,
        ..AudioSampleEntry::default()
    };
    entry.set_box_type(fourcc(box_type));
    entry
}

fn xml_subtitle_sample_entry() -> XMLSubtitleSampleEntry {
    XMLSubtitleSampleEntry {
        sample_entry: SampleEntry {
            box_type: fourcc("stpp"),
            data_reference_index: 1,
        },
        namespace: "urn:ebu:tt:metadata".to_string(),
        schema_location: "urn:ebu:tt:schema".to_string(),
        auxiliary_mime_types: "application/ttml+xml".to_string(),
    }
}

fn text_subtitle_sample_entry() -> TextSubtitleSampleEntry {
    TextSubtitleSampleEntry {
        sample_entry: SampleEntry {
            box_type: fourcc("sbtt"),
            data_reference_index: 1,
        },
        content_encoding: "utf-8".to_string(),
        mime_format: "text/plain".to_string(),
    }
}

fn wvtt_sample_entry() -> WVTTSampleEntry {
    WVTTSampleEntry {
        sample_entry: SampleEntry {
            box_type: fourcc("wvtt"),
            data_reference_index: 1,
        },
    }
}

fn aac_profile_esds(object_type_indication: u8, decoder_specific_info: &[u8]) -> Esds {
    let mut esds = Esds::default();
    esds.descriptors = vec![
        Descriptor {
            tag: DECODER_CONFIG_DESCRIPTOR_TAG,
            size: 13,
            decoder_config_descriptor: Some(DecoderConfigDescriptor {
                object_type_indication,
                stream_type: 5,
                reserved: true,
                ..DecoderConfigDescriptor::default()
            }),
            ..Descriptor::default()
        },
        Descriptor {
            tag: DECODER_SPECIFIC_INFO_TAG,
            size: decoder_specific_info.len() as u32,
            data: decoder_specific_info.to_vec(),
            ..Descriptor::default()
        },
    ];
    esds
}

fn movie_mdat_payload() -> Vec<u8> {
    let video_chunk_one = [avc_sample(5), avc_sample(1)].concat();
    let video_chunk_two = avc_sample(1);
    let audio_chunk = [vec![0x11, 0x22, 0x33], vec![0x44, 0x55, 0x66, 0x77]].concat();
    [video_chunk_one, video_chunk_two, audio_chunk].concat()
}

fn avc_sample(nal_type: u8) -> Vec<u8> {
    vec![0x00, 0x00, 0x00, 0x01, nal_type]
}

fn sample_info(
    size: u32,
    time_delta: u32,
    composition_time_offset: i64,
) -> mp4forge::probe::SampleInfo {
    mp4forge::probe::SampleInfo {
        size,
        time_delta,
        composition_time_offset,
    }
}

fn segment_info(track_id: u32, size: u32, duration: u32) -> mp4forge::probe::SegmentInfo {
    mp4forge::probe::SegmentInfo {
        track_id,
        size,
        duration,
        ..mp4forge::probe::SegmentInfo::default()
    }
}

fn fourcc(value: &str) -> FourCc {
    FourCc::try_from(value).unwrap()
}

fn encode_supported_box<B>(box_value: &B, children: &[u8]) -> Vec<u8>
where
    B: CodecBox,
{
    let mut payload = Vec::new();
    marshal(&mut payload, box_value, None).unwrap();
    payload.extend_from_slice(children);
    encode_raw_box(box_value.box_type(), &payload)
}

fn encode_raw_box(box_type: FourCc, payload: &[u8]) -> Vec<u8> {
    let info = BoxInfo::new(box_type, 8 + payload.len() as u64);
    let mut bytes = info.encode();
    bytes.extend_from_slice(payload);
    bytes
}
