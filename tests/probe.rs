#![allow(clippy::field_reassign_with_default)]

use std::io::Cursor;

use mp4forge::boxes::AnyTypeBox;
use mp4forge::boxes::iso14496_12::{
    AVCDecoderConfiguration, AudioSampleEntry, Ctts, CttsEntry, Edts, Elst, ElstEntry, Ftyp, Mdhd,
    Mdia, Minf, Moof, Moov, Mvhd, SampleEntry, Stbl, Stco, Stsc, StscEntry, Stsd, Stsz, Stts,
    SttsEntry, TFHD_DEFAULT_SAMPLE_DURATION_PRESENT, TFHD_DEFAULT_SAMPLE_SIZE_PRESENT,
    TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT, TRUN_SAMPLE_DURATION_PRESENT,
    TRUN_SAMPLE_SIZE_PRESENT, Tfdt, Tfhd, Tkhd, Traf, Trak, Trun, TrunEntry, VisualSampleEntry,
};
use mp4forge::boxes::iso14496_14::{
    DECODER_CONFIG_DESCRIPTOR_TAG, DECODER_SPECIFIC_INFO_TAG, DecoderConfigDescriptor, Descriptor,
    Esds,
};
use mp4forge::codec::{CodecBox, MutableBox, marshal};
use mp4forge::probe::{
    AacProfileInfo, EditListEntry, TrackCodec, average_sample_bitrate, average_segment_bitrate,
    detect_aac_profile, find_idr_frames, max_sample_bitrate, max_segment_bitrate, probe,
    probe_bytes, probe_fra, probe_fra_bytes,
};
use mp4forge::{BoxInfo, FourCc};

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
fn probe_fra_bytes_matches_cursor_based_probe_fra() {
    let file = build_fragment_file();
    let expected = probe_fra(&mut Cursor::new(file.clone())).unwrap();
    let actual = probe_fra_bytes(&file).unwrap();
    assert_eq!(actual, expected);
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
fn detect_aac_profile_matches_reference_cases() {
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
fn bitrate_helpers_match_reference_math() {
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
    let mdia = encode_supported_box(&Mdia, &[mdhd, minf].concat());
    encode_supported_box(&Trak, &[tkhd, edts, mdia].concat())
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
    let mdia = encode_supported_box(&Mdia, &[mdhd, minf].concat());
    encode_supported_box(&Trak, &[tkhd, mdia].concat())
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

fn video_sample_entry() -> VisualSampleEntry {
    let mut entry = VisualSampleEntry {
        sample_entry: SampleEntry {
            box_type: fourcc("avc1"),
            data_reference_index: 1,
        },
        width: 320,
        height: 180,
        frame_count: 1,
        ..VisualSampleEntry::default()
    };
    entry.set_box_type(fourcc("avc1"));
    entry
}

fn audio_sample_entry() -> AudioSampleEntry {
    let mut entry = AudioSampleEntry {
        sample_entry: SampleEntry {
            box_type: fourcc("mp4a"),
            data_reference_index: 1,
        },
        channel_count: 2,
        sample_size: 16,
        sample_rate: 48_000_u32 << 16,
        ..AudioSampleEntry::default()
    };
    entry.set_box_type(fourcc("mp4a"));
    entry
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
