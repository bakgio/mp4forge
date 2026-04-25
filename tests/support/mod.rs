#![allow(dead_code)]
#![allow(clippy::field_reassign_with_default)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use mp4forge::boxes::AnyTypeBox;
use mp4forge::boxes::iso14496_12::{
    AVCDecoderConfiguration, Btrt, Emeb, Emib, EventMessageSampleEntry, Frma, Ftyp, Hdlr, Mdhd,
    Mdia, Mfhd, Minf, Moof, Moov, Mvex, Mvhd, Pasp, Saio, Saiz, SampleEntry, Sbgp, SbgpEntry, Schi,
    Schm, SeigEntry, SeigEntryL, Sgpd, Silb, SilbEntry, Sinf, Stbl, Stco, Stsc, Stsd, Stsz, Stts,
    TFHD_DEFAULT_SAMPLE_DURATION_PRESENT, TFHD_DEFAULT_SAMPLE_SIZE_PRESENT, Tfdt, Tfhd, Traf, Trak,
    Trex, Trun, VisualSampleEntry,
};
use mp4forge::boxes::iso23001_7::{
    SENC_USE_SUBSAMPLE_ENCRYPTION, Senc, SencSample, SencSubsample, Tenc,
};
use mp4forge::codec::MutableBox;
use mp4forge::codec::{CodecBox, marshal};
use mp4forge::{BoxInfo, FourCc};

pub fn encode_supported_box<B>(box_value: &B, children: &[u8]) -> Vec<u8>
where
    B: CodecBox,
{
    let mut payload = Vec::new();
    marshal(&mut payload, box_value, None).unwrap();
    payload.extend_from_slice(children);
    encode_raw_box(box_value.box_type(), &payload)
}

pub fn encode_raw_box(box_type: FourCc, payload: &[u8]) -> Vec<u8> {
    let info = BoxInfo::new(box_type, 8 + payload.len() as u64);
    let mut bytes = info.encode();
    bytes.extend_from_slice(payload);
    bytes
}

pub fn fourcc(value: &str) -> FourCc {
    FourCc::try_from(value).unwrap()
}

pub fn write_temp_file(prefix: &str, data: &[u8]) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "mp4forge-{prefix}-{}-{unique}.mp4",
        std::process::id()
    ));
    fs::write(&path, data).unwrap();
    path
}

pub fn temp_output_dir(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("mp4forge-{prefix}-{}-{unique}", std::process::id()))
}

pub fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

pub fn read_text(path: &Path) -> String {
    normalize_text(&fs::read_to_string(path).unwrap())
}

pub fn read_golden(relative_path: &str) -> String {
    read_text(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("golden")
            .join(relative_path),
    )
}

pub fn normalize_text(value: &str) -> String {
    value.replace("\r\n", "\n")
}

pub fn build_encrypted_fragmented_video_file() -> Vec<u8> {
    let ftyp = encode_supported_box(
        &Ftyp {
            major_brand: fourcc("iso6"),
            minor_version: 1,
            compatible_brands: vec![fourcc("iso6"), fourcc("dash"), fourcc("cenc")],
        },
        &[],
    );
    let moov = build_encrypted_fragmented_video_moov();
    let moof = build_encrypted_fragmented_video_moof();
    let mdat = encode_raw_box(fourcc("mdat"), &[0xde, 0xad, 0xbe, 0xef]);
    [ftyp, moov, moof, mdat].concat()
}

pub fn build_visual_sample_entry_box_with_trailing_bytes() -> Vec<u8> {
    let pasp = encode_supported_box(
        &Pasp {
            h_spacing: 1,
            v_spacing: 1,
        },
        &[],
    );
    let mut extensions = pasp;
    extensions.extend_from_slice(&visual_sample_entry_trailing_bytes());
    encode_supported_box(&video_sample_entry_with_type("avc1", 640, 360), &extensions)
}

pub fn visual_sample_entry_trailing_bytes() -> Vec<u8> {
    vec![0xde, 0xad, 0xbe]
}

pub fn build_event_message_movie_file() -> Vec<u8> {
    let ftyp = encode_supported_box(
        &Ftyp {
            major_brand: fourcc("isom"),
            minor_version: 1,
            compatible_brands: vec![fourcc("isom"), fourcc("iso8")],
        },
        &[],
    );
    let moov = build_event_message_moov();
    let emib = encode_supported_box(&event_message_instance_box(), &[]);
    let emeb = encode_supported_box(&Emeb, &[]);
    let mdat = encode_raw_box(fourcc("mdat"), &[0x01, 0x02, 0x03, 0x04]);
    [ftyp, moov, emib, emeb, mdat].concat()
}

fn build_encrypted_fragmented_video_moov() -> Vec<u8> {
    let mut mvhd = Mvhd::default();
    mvhd.timescale = 1_000;
    mvhd.duration_v0 = 1_000;
    mvhd.rate = 1 << 16;
    mvhd.volume = 1 << 8;
    mvhd.next_track_id = 2;
    let mvhd = encode_supported_box(&mvhd, &[]);

    let mut trex = Trex::default();
    trex.track_id = 1;
    trex.default_sample_description_index = 1;
    let trex = encode_supported_box(&trex, &[]);
    let mvex = encode_supported_box(&Mvex, &trex);

    encode_supported_box(
        &Moov,
        &[mvhd, build_encrypted_fragmented_video_trak(), mvex].concat(),
    )
}

fn build_encrypted_fragmented_video_trak() -> Vec<u8> {
    let mut tkhd = mp4forge::boxes::iso14496_12::Tkhd::default();
    tkhd.track_id = 1;
    tkhd.width = u32::from(320_u16) << 16;
    tkhd.height = u32::from(180_u16) << 16;
    let tkhd = encode_supported_box(&tkhd, &[]);

    let mut mdhd = Mdhd::default();
    mdhd.timescale = 1_000;
    mdhd.language = [5, 14, 7];
    let mdhd = encode_supported_box(&mdhd, &[]);

    let mut stsd = Stsd::default();
    stsd.entry_count = 1;
    let stsd = encode_supported_box(
        &stsd,
        &encode_supported_box(
            &video_sample_entry_with_type("encv", 320, 180),
            &[
                encode_supported_box(&avc_config(), &[]),
                build_encrypted_fragmented_video_sinf(),
            ]
            .concat(),
        ),
    );

    let mut stco = Stco::default();
    stco.entry_count = 0;
    let stco = encode_supported_box(&stco, &[]);

    let mut stts = Stts::default();
    stts.entry_count = 0;
    let stts = encode_supported_box(&stts, &[]);

    let mut stsc = Stsc::default();
    stsc.entry_count = 0;
    let stsc = encode_supported_box(&stsc, &[]);

    let mut stsz = Stsz::default();
    stsz.sample_count = 0;
    let stsz = encode_supported_box(&stsz, &[]);

    let stbl = encode_supported_box(&Stbl, &[stsd, stco, stts, stsc, stsz].concat());
    let minf = encode_supported_box(&Minf, &stbl);
    let mdia = encode_supported_box(
        &Mdia,
        &[mdhd, handler_box("vide", "VideoHandler"), minf].concat(),
    );
    encode_supported_box(&Trak, &[tkhd, mdia].concat())
}

fn build_event_message_moov() -> Vec<u8> {
    let mut mvhd = Mvhd::default();
    mvhd.timescale = 1_000;
    mvhd.duration_v0 = 1_000;
    mvhd.rate = 1 << 16;
    mvhd.volume = 1 << 8;
    mvhd.next_track_id = 2;
    let mvhd = encode_supported_box(&mvhd, &[]);

    encode_supported_box(&Moov, &[mvhd, build_event_message_trak()].concat())
}

fn build_event_message_trak() -> Vec<u8> {
    let mut tkhd = mp4forge::boxes::iso14496_12::Tkhd::default();
    tkhd.track_id = 1;
    tkhd.duration_v0 = 1_000;
    let tkhd = encode_supported_box(&tkhd, &[]);

    let mut mdhd = Mdhd::default();
    mdhd.timescale = 1_000;
    mdhd.duration_v0 = 1_000;
    mdhd.language = [5, 14, 7];
    let mdhd = encode_supported_box(&mdhd, &[]);

    let mut stsd = Stsd::default();
    stsd.entry_count = 1;
    let stsd = encode_supported_box(&stsd, &event_message_sample_entry_box());

    let mut stco = Stco::default();
    stco.entry_count = 1;
    stco.chunk_offset = vec![0x40];
    let stco = encode_supported_box(&stco, &[]);

    let mut stts = Stts::default();
    stts.entry_count = 1;
    stts.entries = vec![mp4forge::boxes::iso14496_12::SttsEntry {
        sample_count: 1,
        sample_delta: 1_000,
    }];
    let stts = encode_supported_box(&stts, &[]);

    let mut stsc = Stsc::default();
    stsc.entry_count = 1;
    stsc.entries = vec![mp4forge::boxes::iso14496_12::StscEntry {
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
    let mdia = encode_supported_box(
        &Mdia,
        &[mdhd, handler_box("subt", "SubtitleHandler"), minf].concat(),
    );
    encode_supported_box(&Trak, &[tkhd, mdia].concat())
}

fn event_message_sample_entry_box() -> Vec<u8> {
    let entry = EventMessageSampleEntry {
        sample_entry: SampleEntry {
            box_type: fourcc("evte"),
            data_reference_index: 1,
        },
    };
    let children = [
        encode_supported_box(
            &Btrt {
                buffer_size_db: 32_768,
                max_bitrate: 4_000_000,
                avg_bitrate: 2_500_000,
            },
            &[],
        ),
        encode_supported_box(&event_message_scheme_box(), &[]),
    ]
    .concat();
    encode_supported_box(&entry, &children)
}

pub fn event_message_scheme_box() -> Silb {
    let mut silb = Silb::default();
    silb.set_version(0);
    silb.scheme_count = 2;
    silb.schemes = vec![
        SilbEntry {
            scheme_id_uri: "urn:mpeg:dash:event:2012".to_string(),
            value: "event-1".to_string(),
            at_least_one_flag: false,
        },
        SilbEntry {
            scheme_id_uri: "urn:scte:scte35:2013:bin".to_string(),
            value: "splice".to_string(),
            at_least_one_flag: true,
        },
    ];
    silb.other_schemes_flag = true;
    silb
}

pub fn event_message_instance_box() -> Emib {
    let mut emib = Emib::default();
    emib.set_version(0);
    emib.presentation_time_delta = -1_000;
    emib.event_duration = 2_000;
    emib.id = 1_234;
    emib.scheme_id_uri = "urn:scte:scte35:2013:bin".to_string();
    emib.value = "2".to_string();
    emib.message_data = vec![0x01, 0x02, 0x03];
    emib
}

fn build_encrypted_fragmented_video_sinf() -> Vec<u8> {
    let mut schm = Schm::default();
    schm.set_version(0);
    schm.scheme_type = fourcc("cenc");
    schm.scheme_version = 0x0001_0000;

    let mut tenc = Tenc::default();
    tenc.set_version(1);
    tenc.default_crypt_byte_block = 1;
    tenc.default_skip_byte_block = 9;
    tenc.default_is_protected = 1;
    tenc.default_per_sample_iv_size = 8;
    tenc.default_kid = encrypted_fragment_default_kid();

    let schi = encode_supported_box(&Schi, &encode_supported_box(&tenc, &[]));
    encode_supported_box(
        &Sinf,
        &[
            encode_supported_box(
                &Frma {
                    data_format: fourcc("avc1"),
                },
                &[],
            ),
            encode_supported_box(&schm, &[]),
            schi,
        ]
        .concat(),
    )
}

fn build_encrypted_fragmented_video_moof() -> Vec<u8> {
    let mut mfhd = Mfhd::default();
    mfhd.sequence_number = 1;
    let mfhd = encode_supported_box(&mfhd, &[]);

    let mut tfhd = Tfhd::default();
    tfhd.set_flags(TFHD_DEFAULT_SAMPLE_DURATION_PRESENT | TFHD_DEFAULT_SAMPLE_SIZE_PRESENT);
    tfhd.track_id = 1;
    tfhd.default_sample_duration = 1_000;
    tfhd.default_sample_size = 4;
    let tfhd = encode_supported_box(&tfhd, &[]);

    let mut tfdt = Tfdt::default();
    tfdt.set_version(1);
    tfdt.base_media_decode_time_v1 = 0;
    let tfdt = encode_supported_box(&tfdt, &[]);

    let mut trun = Trun::default();
    trun.sample_count = 1;
    let trun = encode_supported_box(&trun, &[]);

    let mut saiz = Saiz::default();
    saiz.sample_count = 1;
    saiz.sample_info_size = vec![16];
    let saiz = encode_supported_box(&saiz, &[]);

    let mut saio = Saio::default();
    saio.entry_count = 1;
    saio.offset_v0 = vec![0];
    let saio = encode_supported_box(&saio, &[]);

    let mut senc = Senc::default();
    senc.set_version(0);
    senc.set_flags(SENC_USE_SUBSAMPLE_ENCRYPTION);
    senc.sample_count = 1;
    senc.samples = vec![SencSample {
        initialization_vector: vec![1, 2, 3, 4, 5, 6, 7, 8],
        subsamples: vec![SencSubsample {
            bytes_of_clear_data: 32,
            bytes_of_protected_data: 480,
        }],
    }];
    let senc = encode_supported_box(&senc, &[]);

    let mut sgpd = Sgpd::default();
    sgpd.set_version(1);
    sgpd.grouping_type = fourcc("seig");
    sgpd.default_length = 0;
    sgpd.entry_count = 1;
    sgpd.seig_entries_l = vec![SeigEntryL {
        description_length: 20,
        seig_entry: SeigEntry {
            crypt_byte_block: 1,
            skip_byte_block: 9,
            is_protected: 1,
            per_sample_iv_size: 8,
            kid: encrypted_fragment_default_kid(),
            ..SeigEntry::default()
        },
    }];
    let sgpd = encode_supported_box(&sgpd, &[]);

    let mut sbgp = Sbgp::default();
    sbgp.grouping_type = u32::from_be_bytes(*b"seig");
    sbgp.entry_count = 1;
    sbgp.entries = vec![SbgpEntry {
        sample_count: 1,
        group_description_index: 65_537,
    }];
    let sbgp = encode_supported_box(&sbgp, &[]);

    let traf = encode_supported_box(
        &Traf,
        &[tfhd, tfdt, trun, saiz, saio, senc, sgpd, sbgp].concat(),
    );
    encode_supported_box(&Moof, &[mfhd, traf].concat())
}

fn encrypted_fragment_default_kid() -> [u8; 16] {
    [
        0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x10, 0x32, 0x54, 0x76, 0x98, 0xba, 0xdc,
        0xfe,
    ]
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
