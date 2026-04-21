use std::any::type_name;
use std::fmt::Debug;
use std::io::Cursor;

use mp4forge::FourCc;
use mp4forge::boxes::iso14496_12::{
    AVCDecoderConfiguration, AVCParameterSet, AlternativeStartupEntry, AlternativeStartupEntryL,
    AlternativeStartupEntryOpt, AudioSampleEntry, Btrt, Co64, Colr, Cslg, Ctts, CttsEntry, Dinf,
    Dref, Edts, Elst, ElstEntry, Emsg, Fiel, Free, Frma, Ftyp, HEVCDecoderConfiguration, HEVCNalu,
    HEVCNaluArray, Hdlr, Mdat, Mdhd, Mdia, Mehd, Meta, Mfhd, Mfra, Mfro, Minf, Moof, Moov, Mvex,
    Mvhd, Pasp, Saio, Saiz, SampleEntry, Sbgp, SbgpEntry, Schi, Schm, Sdtp, SdtpSampleElem, Sgpd,
    Sidx, SidxReference, Sinf, Skip, Smhd, Stbl, Stco, Stsc, StscEntry, Stsd, Stss, Stsz, Stts,
    SttsEntry, Styp, TFHD_BASE_DATA_OFFSET_PRESENT, TFHD_DEFAULT_SAMPLE_DURATION_PRESENT,
    TRUN_DATA_OFFSET_PRESENT, TRUN_FIRST_SAMPLE_FLAGS_PRESENT,
    TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT, TRUN_SAMPLE_DURATION_PRESENT,
    TRUN_SAMPLE_SIZE_PRESENT, TemporalLevelEntry, TextSubtitleSampleEntry, Tfdt, Tfhd, Tfra,
    TfraEntry, Tkhd, Traf, Trak, Trep, Trex, Trun, TrunEntry, Udta, VisualRandomAccessEntry,
    VisualSampleEntry, Vmhd, Wave, XMLSubtitleSampleEntry,
};
use mp4forge::boxes::{AnyTypeBox, default_registry};
use mp4forge::codec::{CodecBox, ImmutableBox, MutableBox, marshal, unmarshal, unmarshal_any};
use mp4forge::stringify::stringify;

fn assert_box_roundtrip<T>(src: T, payload: &[u8], expected: &str)
where
    T: CodecBox + Default + PartialEq + Debug + 'static,
{
    let mut encoded = Vec::new();
    let written = marshal(&mut encoded, &src, None).unwrap();
    assert_eq!(
        written,
        payload.len() as u64,
        "marshal length for {}",
        type_name::<T>()
    );
    assert_eq!(encoded, payload, "marshal bytes for {}", type_name::<T>());

    let mut decoded = T::default();
    let mut reader = Cursor::new(payload.to_vec());
    let read = unmarshal(&mut reader, payload.len() as u64, &mut decoded, None).unwrap();
    assert_eq!(
        read,
        payload.len() as u64,
        "unmarshal length for {}",
        type_name::<T>()
    );
    assert_eq!(decoded, src, "unmarshal value for {}", type_name::<T>());

    let registry = default_registry();
    let mut any_reader = Cursor::new(payload.to_vec());
    let (any_box, any_read) = unmarshal_any(
        &mut any_reader,
        payload.len() as u64,
        src.box_type(),
        &registry,
        None,
    )
    .unwrap();
    assert_eq!(
        any_read,
        payload.len() as u64,
        "registry unmarshal length for {}",
        type_name::<T>()
    );
    assert_eq!(any_box.as_any().downcast_ref::<T>().unwrap(), &src);

    assert_eq!(stringify(&src, None).unwrap(), expected);
}

fn assert_any_box_roundtrip<T>(src: T, payload: &[u8], expected: &str)
where
    T: CodecBox + AnyTypeBox + Default + PartialEq + Debug + 'static,
{
    let mut encoded = Vec::new();
    let written = marshal(&mut encoded, &src, None).unwrap();
    assert_eq!(
        written,
        payload.len() as u64,
        "marshal length for {}",
        type_name::<T>()
    );
    assert_eq!(encoded, payload, "marshal bytes for {}", type_name::<T>());

    let mut decoded = T::default();
    decoded.set_box_type(src.box_type());
    let mut reader = Cursor::new(payload.to_vec());
    let read = unmarshal(&mut reader, payload.len() as u64, &mut decoded, None).unwrap();
    assert_eq!(
        read,
        payload.len() as u64,
        "unmarshal length for {}",
        type_name::<T>()
    );
    assert_eq!(decoded, src, "unmarshal value for {}", type_name::<T>());

    let registry = default_registry();
    let mut any_reader = Cursor::new(payload.to_vec());
    let (any_box, any_read) = unmarshal_any(
        &mut any_reader,
        payload.len() as u64,
        src.box_type(),
        &registry,
        None,
    )
    .unwrap();
    assert_eq!(
        any_read,
        payload.len() as u64,
        "registry unmarshal length for {}",
        type_name::<T>()
    );
    assert_eq!(any_box.as_any().downcast_ref::<T>().unwrap(), &src);

    assert_eq!(stringify(&src, None).unwrap(), expected);
}

#[test]
fn core_iso14496_12_catalog_roundtrips() {
    let mut dref = Dref::default();
    dref.set_version(0);
    dref.entry_count = 0x12345678;

    let mut cslg = Cslg::default();
    cslg.set_version(0);
    cslg.composition_to_dts_shift_v0 = 0x12345678;
    cslg.least_decode_to_display_delta_v0 = -0x12345678;
    cslg.greatest_decode_to_display_delta_v0 = 0x12345678;
    cslg.composition_start_time_v0 = -0x12345678;
    cslg.composition_end_time_v0 = 0x12345678;

    let mut ctts_v0 = Ctts::default();
    ctts_v0.set_version(0);
    ctts_v0.entry_count = 2;
    ctts_v0.entries = vec![
        CttsEntry {
            sample_count: 0x01234567,
            sample_offset_v0: 0x12345678,
            ..CttsEntry::default()
        },
        CttsEntry {
            sample_count: 0x89abcdef,
            sample_offset_v0: 0x789abcde,
            ..CttsEntry::default()
        },
    ];

    let mut ctts_v1 = Ctts::default();
    ctts_v1.set_version(1);
    ctts_v1.entry_count = 2;
    ctts_v1.entries = vec![
        CttsEntry {
            sample_count: 0x01234567,
            sample_offset_v1: 0x12345678,
            ..CttsEntry::default()
        },
        CttsEntry {
            sample_count: 0x89abcdef,
            sample_offset_v1: -0x789abcde,
            ..CttsEntry::default()
        },
    ];

    let mut elst_v0 = Elst::default();
    elst_v0.set_version(0);
    elst_v0.entry_count = 2;
    elst_v0.entries = vec![
        ElstEntry {
            segment_duration_v0: 0x0100000a,
            media_time_v0: 0x0100000b,
            media_rate_integer: 0x010c,
            ..ElstEntry::default()
        },
        ElstEntry {
            segment_duration_v0: 0x0200000a,
            media_time_v0: 0x0200000b,
            media_rate_integer: 0x020c,
            ..ElstEntry::default()
        },
    ];

    let mut elst_v1 = Elst::default();
    elst_v1.set_version(1);
    elst_v1.entry_count = 2;
    elst_v1.entries = vec![
        ElstEntry {
            segment_duration_v1: 0x010000000000000a,
            media_time_v1: 0x010000000000000b,
            media_rate_integer: 0x010c,
            ..ElstEntry::default()
        },
        ElstEntry {
            segment_duration_v1: 0x020000000000000a,
            media_time_v1: 0x020000000000000b,
            media_rate_integer: 0x020c,
            ..ElstEntry::default()
        },
    ];

    let mut mdhd_v0 = Mdhd::default();
    mdhd_v0.set_version(0);
    mdhd_v0.creation_time_v0 = 0x12345678;
    mdhd_v0.modification_time_v0 = 0x23456789;
    mdhd_v0.timescale = 0x01020304;
    mdhd_v0.duration_v0 = 0x02030405;
    mdhd_v0.pad = true;
    mdhd_v0.language = [b'j' - 0x60, b'p' - 0x60, b'n' - 0x60];

    let mut mdhd_v1 = Mdhd::default();
    mdhd_v1.set_version(1);
    mdhd_v1.creation_time_v1 = 0x123456789abcdef0;
    mdhd_v1.modification_time_v1 = 0x23456789abcdef01;
    mdhd_v1.timescale = 0x01020304;
    mdhd_v1.duration_v1 = 0x0203040506070809;
    mdhd_v1.pad = true;
    mdhd_v1.language = [b'j' - 0x60, b'p' - 0x60, b'n' - 0x60];

    let mut mehd_v0 = Mehd::default();
    mehd_v0.set_version(0);
    mehd_v0.fragment_duration_v0 = 0x01234567;

    let mut mehd_v1 = Mehd::default();
    mehd_v1.set_version(1);
    mehd_v1.fragment_duration_v1 = 0x0123456789abcdef;

    let mut mfhd = Mfhd::default();
    mfhd.set_version(0);
    mfhd.sequence_number = 0x12345678;

    let mut mfro = Mfro::default();
    mfro.set_version(0);
    mfro.size = 0x12345678;

    let mut mvhd_v0 = Mvhd::default();
    mvhd_v0.set_version(0);
    mvhd_v0.creation_time_v0 = 0x01234567;
    mvhd_v0.modification_time_v0 = 0x23456789;
    mvhd_v0.timescale = 0x456789ab;
    mvhd_v0.duration_v0 = 0x6789abcd;
    mvhd_v0.rate = -0x01234567;
    mvhd_v0.volume = 0x0123;
    mvhd_v0.next_track_id = 0xabcdef01;

    let mut mvhd_v1 = Mvhd::default();
    mvhd_v1.set_version(1);
    mvhd_v1.creation_time_v1 = 0x0123456789abcdef;
    mvhd_v1.modification_time_v1 = 0x23456789abcdef01;
    mvhd_v1.timescale = 0x89abcdef;
    mvhd_v1.duration_v1 = 0x456789abcdef0123;
    mvhd_v1.rate = -0x01234567;
    mvhd_v1.volume = 0x0123;
    mvhd_v1.next_track_id = 0xabcdef01;

    let mut smhd = Smhd::default();
    smhd.set_version(0);
    smhd.balance = 0x0123;

    let mut stco = Stco::default();
    stco.set_version(0);
    stco.entry_count = 2;
    stco.chunk_offset = vec![0x01234567, 0x89abcdef];

    let mut stsc = Stsc::default();
    stsc.set_version(0);
    stsc.entry_count = 2;
    stsc.entries = vec![
        StscEntry {
            first_chunk: 0x01234567,
            samples_per_chunk: 0x23456789,
            sample_description_index: 0x456789ab,
        },
        StscEntry {
            first_chunk: 0x6789abcd,
            samples_per_chunk: 0x89abcdef,
            sample_description_index: 0xabcdef01,
        },
    ];

    let mut stsd = Stsd::default();
    stsd.set_version(0);
    stsd.entry_count = 0x01234567;

    let mut stss = Stss::default();
    stss.set_version(0);
    stss.entry_count = 2;
    stss.sample_number = vec![0x01234567, 0x89abcdef];

    let mut stss_single = Stss::default();
    stss_single.set_version(0);
    stss_single.entry_count = 1;
    stss_single.sample_number = vec![0x01234567];

    let mut stsz_common = Stsz::default();
    stsz_common.set_version(0);
    stsz_common.sample_size = 0x01234567;
    stsz_common.sample_count = 2;

    let mut stsz_array = Stsz::default();
    stsz_array.set_version(0);
    stsz_array.sample_count = 2;
    stsz_array.entry_size = vec![0x01234567, 0x23456789];

    let mut stts = Stts::default();
    stts.set_version(0);
    stts.entry_count = 2;
    stts.entries = vec![
        SttsEntry {
            sample_count: 0x01234567,
            sample_delta: 0x23456789,
        },
        SttsEntry {
            sample_count: 0x456789ab,
            sample_delta: 0x6789abcd,
        },
    ];

    let mut tfdt_v0 = Tfdt::default();
    tfdt_v0.set_version(0);
    tfdt_v0.base_media_decode_time_v0 = 0x01234567;

    let mut tfdt_v1 = Tfdt::default();
    tfdt_v1.set_version(1);
    tfdt_v1.base_media_decode_time_v1 = 0x0123456789abcdef;

    let mut tfhd_empty = Tfhd::default();
    tfhd_empty.set_version(0);
    tfhd_empty.track_id = 0x08404649;

    let mut tfhd_optional = Tfhd::default();
    tfhd_optional.set_version(0);
    tfhd_optional.set_flags(TFHD_BASE_DATA_OFFSET_PRESENT | TFHD_DEFAULT_SAMPLE_DURATION_PRESENT);
    tfhd_optional.track_id = 0x08404649;
    tfhd_optional.base_data_offset = 0x0123456789abcdef;
    tfhd_optional.default_sample_duration = 0x23456789;

    let mut tkhd_v0 = Tkhd::default();
    tkhd_v0.set_version(0);
    tkhd_v0.creation_time_v0 = 0x01234567;
    tkhd_v0.modification_time_v0 = 0x12345678;
    tkhd_v0.track_id = 0x23456789;
    tkhd_v0.duration_v0 = 0x456789ab;
    tkhd_v0.layer = 23456;
    tkhd_v0.alternate_group = -23456;
    tkhd_v0.volume = 0x0100;
    tkhd_v0.matrix = [0x00010000, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000];
    tkhd_v0.width = 125829120;
    tkhd_v0.height = 70778880;

    let mut tkhd_v1 = Tkhd::default();
    tkhd_v1.set_version(1);
    tkhd_v1.creation_time_v1 = 0x0123456789abcdef;
    tkhd_v1.modification_time_v1 = 0x123456789abcdef0;
    tkhd_v1.track_id = 0x23456789;
    tkhd_v1.duration_v1 = 0x456789abcdef0123;
    tkhd_v1.layer = 23456;
    tkhd_v1.alternate_group = -23456;
    tkhd_v1.volume = 0x0100;
    tkhd_v1.matrix = tkhd_v0.matrix;
    tkhd_v1.width = 125829120;
    tkhd_v1.height = 70778880;

    let mut trep = Trep::default();
    trep.set_version(0);
    trep.track_id = 0x01234567;

    let mut trex = Trex::default();
    trex.set_version(0);
    trex.track_id = 0x01234567;
    trex.default_sample_description_index = 0x23456789;
    trex.default_sample_duration = 0x456789ab;
    trex.default_sample_size = 0x6789abcd;
    trex.default_sample_flags = 0x89abcdef;

    let mut trun_duration = Trun::default();
    trun_duration.set_version(0);
    trun_duration.set_flags(TRUN_DATA_OFFSET_PRESENT | TRUN_SAMPLE_DURATION_PRESENT);
    trun_duration.sample_count = 3;
    trun_duration.data_offset = 50;
    trun_duration.entries = vec![
        TrunEntry {
            sample_duration: 100,
            ..TrunEntry::default()
        },
        TrunEntry {
            sample_duration: 101,
            ..TrunEntry::default()
        },
        TrunEntry {
            sample_duration: 102,
            ..TrunEntry::default()
        },
    ];

    let mut trun_sizes = Trun::default();
    trun_sizes.set_version(0);
    trun_sizes.set_flags(TRUN_FIRST_SAMPLE_FLAGS_PRESENT | TRUN_SAMPLE_SIZE_PRESENT);
    trun_sizes.sample_count = 3;
    trun_sizes.first_sample_flags = 0x02468ace;
    trun_sizes.entries = vec![
        TrunEntry {
            sample_size: 100,
            ..TrunEntry::default()
        },
        TrunEntry {
            sample_size: 101,
            ..TrunEntry::default()
        },
        TrunEntry {
            sample_size: 102,
            ..TrunEntry::default()
        },
    ];

    let mut trun_cto = Trun::default();
    trun_cto.set_version(1);
    trun_cto.set_flags(TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT);
    trun_cto.sample_count = 3;
    trun_cto.entries = vec![
        TrunEntry {
            sample_composition_time_offset_v1: 200,
            ..TrunEntry::default()
        },
        TrunEntry {
            sample_composition_time_offset_v1: 201,
            ..TrunEntry::default()
        },
        TrunEntry {
            sample_composition_time_offset_v1: -202,
            ..TrunEntry::default()
        },
    ];

    let mut vmhd = Vmhd::default();
    vmhd.set_version(0);
    vmhd.graphicsmode = 0x0123;
    vmhd.opcolor = [0x2345, 0x4567, 0x6789];

    assert_box_roundtrip(
        Ftyp {
            major_brand: FourCc::from_bytes(*b"abem"),
            minor_version: 0x12345678,
            compatible_brands: vec![FourCc::from_bytes(*b"abcd"), FourCc::from_bytes(*b"efgh")],
        },
        &[
            b'a', b'b', b'e', b'm', 0x12, 0x34, 0x56, 0x78, b'a', b'b', b'c', b'd', b'e', b'f',
            b'g', b'h',
        ],
        "MajorBrand=\"abem\" MinorVersion=305419896 CompatibleBrands=[{CompatibleBrand=\"abcd\"}, {CompatibleBrand=\"efgh\"}]",
    );
    assert_box_roundtrip(
        Styp {
            major_brand: FourCc::from_bytes(*b"abem"),
            minor_version: 0x12345678,
            compatible_brands: vec![FourCc::from_bytes(*b"abcd"), FourCc::from_bytes(*b"efgh")],
        },
        &[
            b'a', b'b', b'e', b'm', 0x12, 0x34, 0x56, 0x78, b'a', b'b', b'c', b'd', b'e', b'f',
            b'g', b'h',
        ],
        "MajorBrand=\"abem\" MinorVersion=305419896 CompatibleBrands=[{CompatibleBrand=\"abcd\"}, {CompatibleBrand=\"efgh\"}]",
    );
    assert_box_roundtrip(
        Free {
            data: vec![0x12, 0x34, 0x56],
        },
        &[0x12, 0x34, 0x56],
        "Data=[0x12, 0x34, 0x56]",
    );
    assert_box_roundtrip(
        Skip {
            data: vec![0x12, 0x34, 0x56],
        },
        &[0x12, 0x34, 0x56],
        "Data=[0x12, 0x34, 0x56]",
    );
    assert_box_roundtrip(
        Mdat {
            data: vec![0x11, 0x22, 0x33],
        },
        &[0x11, 0x22, 0x33],
        "Data=[0x11, 0x22, 0x33]",
    );
    assert_box_roundtrip(Dinf, &[], "");
    assert_box_roundtrip(Edts, &[], "");
    assert_box_roundtrip(Mdia, &[], "");
    assert_box_roundtrip(Minf, &[], "");
    assert_box_roundtrip(Moof, &[], "");
    assert_box_roundtrip(Moov, &[], "");
    assert_box_roundtrip(Mvex, &[], "");
    assert_box_roundtrip(Mfra, &[], "");
    assert_box_roundtrip(Stbl, &[], "");
    assert_box_roundtrip(Traf, &[], "");
    assert_box_roundtrip(Trak, &[], "");
    assert_box_roundtrip(Udta, &[], "");
    assert_box_roundtrip(
        dref,
        &[0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x56, 0x78],
        "Version=0 Flags=0x000000 EntryCount=305419896",
    );
    assert_box_roundtrip(
        cslg,
        &[
            0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x56, 0x78, 0xed, 0xcb, 0xa9, 0x88, 0x12, 0x34,
            0x56, 0x78, 0xed, 0xcb, 0xa9, 0x88, 0x12, 0x34, 0x56, 0x78,
        ],
        "Version=0 Flags=0x000000 CompositionToDTSShiftV0=305419896 LeastDecodeToDisplayDeltaV0=-305419896 GreatestDecodeToDisplayDeltaV0=305419896 CompositionStartTimeV0=-305419896 CompositionEndTimeV0=305419896",
    );
    assert_box_roundtrip(
        ctts_v0,
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x01, 0x23, 0x45, 0x67, 0x12, 0x34,
            0x56, 0x78, 0x89, 0xab, 0xcd, 0xef, 0x78, 0x9a, 0xbc, 0xde,
        ],
        "Version=0 Flags=0x000000 EntryCount=2 Entries=[{SampleCount=19088743 SampleOffsetV0=305419896}, {SampleCount=2309737967 SampleOffsetV0=2023406814}]",
    );
    assert_box_roundtrip(
        ctts_v1,
        &[
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x01, 0x23, 0x45, 0x67, 0x12, 0x34,
            0x56, 0x78, 0x89, 0xab, 0xcd, 0xef, 0x87, 0x65, 0x43, 0x22,
        ],
        "Version=1 Flags=0x000000 EntryCount=2 Entries=[{SampleCount=19088743 SampleOffsetV1=305419896}, {SampleCount=2309737967 SampleOffsetV1=-2023406814}]",
    );
    assert_box_roundtrip(
        elst_v0,
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x01, 0x00, 0x00, 0x0a, 0x01, 0x00,
            0x00, 0x0b, 0x01, 0x0c, 0x00, 0x00, 0x02, 0x00, 0x00, 0x0a, 0x02, 0x00, 0x00, 0x0b,
            0x02, 0x0c, 0x00, 0x00,
        ],
        "Version=0 Flags=0x000000 EntryCount=2 Entries=[{SegmentDurationV0=16777226 MediaTimeV0=16777227 MediaRateInteger=268}, {SegmentDurationV0=33554442 MediaTimeV0=33554443 MediaRateInteger=524}]",
    );
    assert_box_roundtrip(
        elst_v1,
        &[
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x0a, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0b, 0x01, 0x0c, 0x00, 0x00,
            0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0a, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x0b, 0x02, 0x0c, 0x00, 0x00,
        ],
        "Version=1 Flags=0x000000 EntryCount=2 Entries=[{SegmentDurationV1=72057594037927946 MediaTimeV1=72057594037927947 MediaRateInteger=268}, {SegmentDurationV1=144115188075855882 MediaTimeV1=144115188075855883 MediaRateInteger=524}]",
    );
    assert_box_roundtrip(
        mehd_v0,
        &[0x00, 0x00, 0x00, 0x00, 0x01, 0x23, 0x45, 0x67],
        "Version=0 Flags=0x000000 FragmentDurationV0=19088743",
    );
    assert_box_roundtrip(
        mehd_v1,
        &[
            0x01, 0x00, 0x00, 0x00, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
        ],
        "Version=1 Flags=0x000000 FragmentDurationV1=81985529216486895",
    );
    assert_box_roundtrip(
        mfhd,
        &[0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x56, 0x78],
        "Version=0 Flags=0x000000 SequenceNumber=305419896",
    );
    assert_box_roundtrip(
        mfro,
        &[0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x56, 0x78],
        "Version=0 Flags=0x000000 Size=305419896",
    );
    assert_box_roundtrip(
        mdhd_v0,
        &[
            0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x56, 0x78, 0x23, 0x45, 0x67, 0x89, 0x01, 0x02,
            0x03, 0x04, 0x02, 0x03, 0x04, 0x05, 0xaa, 0x0e, 0x00, 0x00,
        ],
        "Version=0 Flags=0x000000 CreationTimeV0=305419896 ModificationTimeV0=591751049 Timescale=16909060 DurationV0=33752069 Language=\"jpn\" PreDefined=0",
    );
    assert_box_roundtrip(
        mdhd_v1,
        &[
            0x01, 0x00, 0x00, 0x00, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x23, 0x45,
            0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x01, 0x02, 0x03, 0x04, 0x02, 0x03, 0x04, 0x05,
            0x06, 0x07, 0x08, 0x09, 0xaa, 0x0e, 0x00, 0x00,
        ],
        "Version=1 Flags=0x000000 CreationTimeV1=1311768467463790320 ModificationTimeV1=2541551405711093505 Timescale=16909060 DurationV1=144964032628459529 Language=\"jpn\" PreDefined=0",
    );
    let mut mvhd_v0_bytes = vec![
        0x00, 0x00, 0x00, 0x00, 0x01, 0x23, 0x45, 0x67, 0x23, 0x45, 0x67, 0x89, 0x45, 0x67, 0x89,
        0xab, 0x67, 0x89, 0xab, 0xcd, 0xfe, 0xdc, 0xba, 0x99, 0x01, 0x23, 0x00, 0x00,
    ];
    mvhd_v0_bytes.extend_from_slice(&[0; 68]);
    mvhd_v0_bytes.extend_from_slice(&[0xab, 0xcd, 0xef, 0x01]);
    assert_box_roundtrip(
        mvhd_v0,
        &mvhd_v0_bytes,
        "Version=0 Flags=0x000000 CreationTimeV0=19088743 ModificationTimeV0=591751049 Timescale=1164413355 DurationV0=1737075661 Rate=-291.27110 Volume=291 Matrix=[0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0] PreDefined=[0, 0, 0, 0, 0, 0] NextTrackID=2882400001",
    );
    let mut mvhd_v1_bytes = vec![
        0x01, 0x00, 0x00, 0x00, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x23, 0x45, 0x67,
        0x89, 0xab, 0xcd, 0xef, 0x01, 0x89, 0xab, 0xcd, 0xef, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
        0x01, 0x23, 0xfe, 0xdc, 0xba, 0x99, 0x01, 0x23, 0x00, 0x00,
    ];
    mvhd_v1_bytes.extend_from_slice(&[0; 68]);
    mvhd_v1_bytes.extend_from_slice(&[0xab, 0xcd, 0xef, 0x01]);
    assert_box_roundtrip(
        mvhd_v1,
        &mvhd_v1_bytes,
        "Version=1 Flags=0x000000 CreationTimeV1=81985529216486895 ModificationTimeV1=2541551405711093505 Timescale=2309737967 DurationV1=5001117282205630755 Rate=-291.27110 Volume=291 Matrix=[0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0] PreDefined=[0, 0, 0, 0, 0, 0] NextTrackID=2882400001",
    );
    assert_box_roundtrip(
        smhd,
        &[0x00, 0x00, 0x00, 0x00, 0x01, 0x23, 0x00, 0x00],
        "Version=0 Flags=0x000000 Balance=1.137",
    );
    assert_box_roundtrip(
        stco,
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef,
        ],
        "Version=0 Flags=0x000000 EntryCount=2 ChunkOffset=[19088743, 2309737967]",
    );
    let mut co64 = Co64::default();
    co64.entry_count = 2;
    co64.chunk_offset = vec![0x0123456789abcdef, 0x89abcdef01234567];
    assert_box_roundtrip(
        co64,
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67,
        ],
        "Version=0 Flags=0x000000 EntryCount=2 ChunkOffset=[81985529216486895, 9920249030613615975]",
    );
    assert_box_roundtrip(
        stsc,
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x01, 0x23, 0x45, 0x67, 0x23, 0x45,
            0x67, 0x89, 0x45, 0x67, 0x89, 0xab, 0x67, 0x89, 0xab, 0xcd, 0x89, 0xab, 0xcd, 0xef,
            0xab, 0xcd, 0xef, 0x01,
        ],
        "Version=0 Flags=0x000000 EntryCount=2 Entries=[{FirstChunk=19088743 SamplesPerChunk=591751049 SampleDescriptionIndex=1164413355}, {FirstChunk=1737075661 SamplesPerChunk=2309737967 SampleDescriptionIndex=2882400001}]",
    );
    assert_box_roundtrip(
        stsd,
        &[0x00, 0x00, 0x00, 0x00, 0x01, 0x23, 0x45, 0x67],
        "Version=0 Flags=0x000000 EntryCount=19088743",
    );
    assert_box_roundtrip(
        stss,
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef,
        ],
        "Version=0 Flags=0x000000 EntryCount=2 SampleNumber=[19088743, 2309737967]",
    );
    assert_box_roundtrip(
        stss_single,
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01, 0x23, 0x45, 0x67,
        ],
        "Version=0 Flags=0x000000 EntryCount=1 SampleNumber=[19088743]",
    );
    assert_box_roundtrip(
        stsz_common,
        &[
            0x00, 0x00, 0x00, 0x00, 0x01, 0x23, 0x45, 0x67, 0x00, 0x00, 0x00, 0x02,
        ],
        "Version=0 Flags=0x000000 SampleSize=19088743 SampleCount=2 EntrySize=[]",
    );
    assert_box_roundtrip(
        stsz_array,
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x01, 0x23,
            0x45, 0x67, 0x23, 0x45, 0x67, 0x89,
        ],
        "Version=0 Flags=0x000000 SampleSize=0 SampleCount=2 EntrySize=[19088743, 591751049]",
    );
    assert_box_roundtrip(
        stts,
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x01, 0x23, 0x45, 0x67, 0x23, 0x45,
            0x67, 0x89, 0x45, 0x67, 0x89, 0xab, 0x67, 0x89, 0xab, 0xcd,
        ],
        "Version=0 Flags=0x000000 EntryCount=2 Entries=[{SampleCount=19088743 SampleDelta=591751049}, {SampleCount=1164413355 SampleDelta=1737075661}]",
    );
    assert_box_roundtrip(
        tfdt_v0,
        &[0x00, 0x00, 0x00, 0x00, 0x01, 0x23, 0x45, 0x67],
        "Version=0 Flags=0x000000 BaseMediaDecodeTimeV0=19088743",
    );
    assert_box_roundtrip(
        tfdt_v1,
        &[
            0x01, 0x00, 0x00, 0x00, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
        ],
        "Version=1 Flags=0x000000 BaseMediaDecodeTimeV1=81985529216486895",
    );
    assert_box_roundtrip(
        tfhd_empty,
        &[0x00, 0x00, 0x00, 0x00, 0x08, 0x40, 0x46, 0x49],
        "Version=0 Flags=0x000000 TrackID=138430025",
    );
    assert_box_roundtrip(
        tfhd_optional,
        &[
            0x00, 0x00, 0x00, 0x09, 0x08, 0x40, 0x46, 0x49, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef, 0x23, 0x45, 0x67, 0x89,
        ],
        "Version=0 Flags=0x000009 TrackID=138430025 BaseDataOffset=81985529216486895 DefaultSampleDuration=591751049",
    );
    let mut tkhd_v0_bytes = vec![
        0x00, 0x00, 0x00, 0x00, 0x01, 0x23, 0x45, 0x67, 0x12, 0x34, 0x56, 0x78, 0x23, 0x45, 0x67,
        0x89, 0x00, 0x00, 0x00, 0x00, 0x45, 0x67, 0x89, 0xab,
    ];
    tkhd_v0_bytes.extend_from_slice(&[0; 8]);
    tkhd_v0_bytes.extend_from_slice(&tkhd_v0.layer.to_be_bytes());
    tkhd_v0_bytes.extend_from_slice(&tkhd_v0.alternate_group.to_be_bytes());
    tkhd_v0_bytes.extend_from_slice(&tkhd_v0.volume.to_be_bytes());
    tkhd_v0_bytes.extend_from_slice(&0_u16.to_be_bytes());
    for value in tkhd_v0.matrix {
        tkhd_v0_bytes.extend_from_slice(&value.to_be_bytes());
    }
    tkhd_v0_bytes.extend_from_slice(&tkhd_v0.width.to_be_bytes());
    tkhd_v0_bytes.extend_from_slice(&tkhd_v0.height.to_be_bytes());
    assert_box_roundtrip(
        tkhd_v0,
        &tkhd_v0_bytes,
        "Version=0 Flags=0x000000 CreationTimeV0=19088743 ModificationTimeV0=305419896 TrackID=591751049 DurationV0=1164413355 Layer=23456 AlternateGroup=-23456 Volume=256 Matrix=[0x10000, 0x0, 0x0, 0x0, 0x10000, 0x0, 0x0, 0x0, 0x40000000] Width=1920 Height=1080",
    );
    let mut tkhd_v1_bytes = vec![
        0x01, 0x00, 0x00, 0x00, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x12, 0x34, 0x56,
        0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x23, 0x45, 0x67, 0x89, 0x00, 0x00, 0x00, 0x00, 0x45, 0x67,
        0x89, 0xab, 0xcd, 0xef, 0x01, 0x23,
    ];
    tkhd_v1_bytes.extend_from_slice(&[0; 8]);
    tkhd_v1_bytes.extend_from_slice(&tkhd_v1.layer.to_be_bytes());
    tkhd_v1_bytes.extend_from_slice(&tkhd_v1.alternate_group.to_be_bytes());
    tkhd_v1_bytes.extend_from_slice(&tkhd_v1.volume.to_be_bytes());
    tkhd_v1_bytes.extend_from_slice(&0_u16.to_be_bytes());
    for value in tkhd_v1.matrix {
        tkhd_v1_bytes.extend_from_slice(&value.to_be_bytes());
    }
    tkhd_v1_bytes.extend_from_slice(&tkhd_v1.width.to_be_bytes());
    tkhd_v1_bytes.extend_from_slice(&tkhd_v1.height.to_be_bytes());
    assert_box_roundtrip(
        tkhd_v1,
        &tkhd_v1_bytes,
        "Version=1 Flags=0x000000 CreationTimeV1=81985529216486895 ModificationTimeV1=1311768467463790320 TrackID=591751049 DurationV1=5001117282205630755 Layer=23456 AlternateGroup=-23456 Volume=256 Matrix=[0x10000, 0x0, 0x0, 0x0, 0x10000, 0x0, 0x0, 0x0, 0x40000000] Width=1920 Height=1080",
    );
    assert_box_roundtrip(
        trep,
        &[0x00, 0x00, 0x00, 0x00, 0x01, 0x23, 0x45, 0x67],
        "Version=0 Flags=0x000000 TrackID=19088743",
    );
    assert_box_roundtrip(
        trex,
        &[
            0x00, 0x00, 0x00, 0x00, 0x01, 0x23, 0x45, 0x67, 0x23, 0x45, 0x67, 0x89, 0x45, 0x67,
            0x89, 0xab, 0x67, 0x89, 0xab, 0xcd, 0x89, 0xab, 0xcd, 0xef,
        ],
        "Version=0 Flags=0x000000 TrackID=19088743 DefaultSampleDescriptionIndex=591751049 DefaultSampleDuration=1164413355 DefaultSampleSize=1737075661 DefaultSampleFlags=0x89abcdef",
    );
    assert_box_roundtrip(
        trun_duration,
        &[
            0x00, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x32, 0x00, 0x00,
            0x00, 0x64, 0x00, 0x00, 0x00, 0x65, 0x00, 0x00, 0x00, 0x66,
        ],
        "Version=0 Flags=0x000101 SampleCount=3 DataOffset=50 Entries=[{SampleDuration=100}, {SampleDuration=101}, {SampleDuration=102}]",
    );
    assert_box_roundtrip(
        trun_sizes,
        &[
            0x00, 0x00, 0x02, 0x04, 0x00, 0x00, 0x00, 0x03, 0x02, 0x46, 0x8a, 0xce, 0x00, 0x00,
            0x00, 0x64, 0x00, 0x00, 0x00, 0x65, 0x00, 0x00, 0x00, 0x66,
        ],
        "Version=0 Flags=0x000204 SampleCount=3 FirstSampleFlags=0x2468ace Entries=[{SampleSize=100}, {SampleSize=101}, {SampleSize=102}]",
    );
    assert_box_roundtrip(
        trun_cto,
        &[
            0x01, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0xc8, 0x00, 0x00,
            0x00, 0xc9, 0xff, 0xff, 0xff, 0x36,
        ],
        "Version=1 Flags=0x000800 SampleCount=3 Entries=[{SampleCompositionTimeOffsetV1=200}, {SampleCompositionTimeOffsetV1=201}, {SampleCompositionTimeOffsetV1=-202}]",
    );
    assert_box_roundtrip(
        vmhd,
        &[
            0x00, 0x00, 0x00, 0x00, 0x01, 0x23, 0x23, 0x45, 0x45, 0x67, 0x67, 0x89,
        ],
        "Version=0 Flags=0x000000 Graphicsmode=291 Opcolor=[9029, 17767, 26505]",
    );
}

#[test]
fn additional_iso14496_12_catalog_roundtrips() {
    let mut hdlr = Hdlr::default();
    hdlr.set_version(0);
    hdlr.pre_defined = 0x12345678;
    hdlr.handler_type = FourCc::from_bytes(*b"abem");
    hdlr.name = String::from("Abema");

    let mut meta = Meta::default();
    meta.set_version(0);

    let mut saio_v0 = Saio::default();
    saio_v0.set_version(0);
    saio_v0.entry_count = 3;
    saio_v0.offset_v0 = vec![0x01234567, 0x23456789, 0x456789ab];

    let mut saio_v0_aux = Saio::default();
    saio_v0_aux.set_version(0);
    saio_v0_aux.set_flags(0x000001);
    saio_v0_aux.aux_info_type = FourCc::from_bytes(*b"test");
    saio_v0_aux.aux_info_type_parameter = 0x89abcdef;
    saio_v0_aux.entry_count = 3;
    saio_v0_aux.offset_v0 = vec![0x01234567, 0x23456789, 0x456789ab];

    let mut saio_v1 = Saio::default();
    saio_v1.set_version(1);
    saio_v1.entry_count = 2;
    saio_v1.offset_v1 = vec![0x0123456789abcdef, 0x0123456789abcdef];

    let mut saiz_default = Saiz::default();
    saiz_default.set_version(0);
    saiz_default.default_sample_info_size = 1;
    saiz_default.sample_count = 0x01234567;

    let mut saiz_array = Saiz::default();
    saiz_array.set_version(0);
    saiz_array.sample_count = 3;
    saiz_array.sample_info_size = vec![1, 2, 3];

    let mut saiz_aux = Saiz::default();
    saiz_aux.set_version(0);
    saiz_aux.set_flags(0x000001);
    saiz_aux.aux_info_type = FourCc::from_bytes(*b"test");
    saiz_aux.aux_info_type_parameter = 0x89abcdef;
    saiz_aux.default_sample_info_size = 1;
    saiz_aux.sample_count = 0x01234567;

    let mut sbgp_v0 = Sbgp::default();
    sbgp_v0.set_version(0);
    sbgp_v0.grouping_type = 0x01234567;
    sbgp_v0.entry_count = 2;
    sbgp_v0.entries = vec![
        SbgpEntry {
            sample_count: 0x23456789,
            group_description_index: 0x3456789a,
        },
        SbgpEntry {
            sample_count: 0x456789ab,
            group_description_index: 0x56789abc,
        },
    ];

    let mut sbgp_v1 = Sbgp::default();
    sbgp_v1.set_version(1);
    sbgp_v1.grouping_type = 0x01234567;
    sbgp_v1.grouping_type_parameter = 0x89abcdef;
    sbgp_v1.entry_count = 2;
    sbgp_v1.entries = sbgp_v0.entries.clone();

    let mut sdtp = Sdtp::default();
    sdtp.set_version(0);
    sdtp.samples = vec![
        SdtpSampleElem::default(),
        SdtpSampleElem {
            sample_depends_on: 1,
            sample_is_depended_on: 2,
            sample_has_redundancy: 3,
            ..SdtpSampleElem::default()
        },
        SdtpSampleElem {
            is_leading: 3,
            sample_depends_on: 2,
            sample_is_depended_on: 1,
            ..SdtpSampleElem::default()
        },
    ];

    let opts = vec![
        AlternativeStartupEntryOpt {
            num_output_samples: 0x0123,
            num_total_samples: 0x4567,
        },
        AlternativeStartupEntryOpt {
            num_output_samples: 0x89ab,
            num_total_samples: 0xcdef,
        },
    ];

    let mut sgpd_roll_v1 = Sgpd::default();
    sgpd_roll_v1.set_version(1);
    sgpd_roll_v1.grouping_type = FourCc::from_bytes(*b"roll");
    sgpd_roll_v1.default_length = 2;
    sgpd_roll_v1.entry_count = 2;
    sgpd_roll_v1.roll_distances = vec![0x1111, 0x2222];

    let mut sgpd_prol_v1 = Sgpd::default();
    sgpd_prol_v1.set_version(1);
    sgpd_prol_v1.grouping_type = FourCc::from_bytes(*b"prol");
    sgpd_prol_v1.default_length = 2;
    sgpd_prol_v1.entry_count = 2;
    sgpd_prol_v1.roll_distances = vec![0x1111, 0x2222];

    let mut sgpd_alst_v1 = Sgpd::default();
    sgpd_alst_v1.set_version(1);
    sgpd_alst_v1.grouping_type = FourCc::from_bytes(*b"alst");
    sgpd_alst_v1.default_length = 12;
    sgpd_alst_v1.entry_count = 2;
    sgpd_alst_v1.alternative_startup_entries = vec![
        AlternativeStartupEntry {
            roll_count: 2,
            first_output_sample: 0x0123,
            sample_offset: vec![0x01234567, 0x89abcdef],
            opts: Vec::new(),
        },
        AlternativeStartupEntry {
            roll_count: 2,
            first_output_sample: 0x4567,
            sample_offset: vec![0x456789ab, 0xcdef0123],
            opts: Vec::new(),
        },
    ];

    let mut sgpd_alst_default_v1 = Sgpd::default();
    sgpd_alst_default_v1.set_version(1);
    sgpd_alst_default_v1.grouping_type = FourCc::from_bytes(*b"alst");
    sgpd_alst_default_v1.default_length = 20;
    sgpd_alst_default_v1.entry_count = 2;
    sgpd_alst_default_v1.alternative_startup_entries = vec![
        AlternativeStartupEntry {
            roll_count: 2,
            first_output_sample: 0x0123,
            sample_offset: vec![0x01234567, 0x89abcdef],
            opts: opts.clone(),
        },
        AlternativeStartupEntry {
            roll_count: 2,
            first_output_sample: 0x4567,
            sample_offset: vec![0x456789ab, 0xcdef0123],
            opts: opts.clone(),
        },
    ];

    let mut sgpd_alst_len_v1 = Sgpd::default();
    sgpd_alst_len_v1.set_version(1);
    sgpd_alst_len_v1.grouping_type = FourCc::from_bytes(*b"alst");
    sgpd_alst_len_v1.default_length = 0;
    sgpd_alst_len_v1.entry_count = 2;
    sgpd_alst_len_v1.alternative_startup_entries_l = vec![
        AlternativeStartupEntryL {
            description_length: 16,
            alternative_startup_entry: AlternativeStartupEntry {
                roll_count: 2,
                first_output_sample: 0x0123,
                sample_offset: vec![0x01234567, 0x89abcdef],
                opts: vec![opts[0].clone()],
            },
        },
        AlternativeStartupEntryL {
            description_length: 20,
            alternative_startup_entry: AlternativeStartupEntry {
                roll_count: 2,
                first_output_sample: 0x4567,
                sample_offset: vec![0x456789ab, 0xcdef0123],
                opts: opts.clone(),
            },
        },
    ];

    let mut sgpd_rap_v1 = Sgpd::default();
    sgpd_rap_v1.set_version(1);
    sgpd_rap_v1.grouping_type = FourCc::from_bytes(*b"rap ");
    sgpd_rap_v1.default_length = 1;
    sgpd_rap_v1.entry_count = 2;
    sgpd_rap_v1.visual_random_access_entries = vec![
        VisualRandomAccessEntry {
            num_leading_samples_known: true,
            num_leading_samples: 0x27,
        },
        VisualRandomAccessEntry {
            num_leading_samples_known: false,
            num_leading_samples: 0x1a,
        },
    ];

    let mut sgpd_tele_v1 = Sgpd::default();
    sgpd_tele_v1.set_version(1);
    sgpd_tele_v1.grouping_type = FourCc::from_bytes(*b"tele");
    sgpd_tele_v1.default_length = 1;
    sgpd_tele_v1.entry_count = 2;
    sgpd_tele_v1.temporal_level_entries = vec![
        TemporalLevelEntry {
            level_independently_decodable: true,
        },
        TemporalLevelEntry {
            level_independently_decodable: false,
        },
    ];

    let mut sgpd_roll_v2 = Sgpd::default();
    sgpd_roll_v2.set_version(2);
    sgpd_roll_v2.grouping_type = FourCc::from_bytes(*b"roll");
    sgpd_roll_v2.default_sample_description_index = 5;
    sgpd_roll_v2.entry_count = 2;
    sgpd_roll_v2.roll_distances = vec![0x1111, 0x2222];

    let mut sidx_v0 = Sidx::default();
    sidx_v0.set_version(0);
    sidx_v0.reference_id = 0x01234567;
    sidx_v0.timescale = 0x23456789;
    sidx_v0.earliest_presentation_time_v0 = 0x456789ab;
    sidx_v0.first_offset_v0 = 0x6789abcd;
    sidx_v0.reference_count = 2;
    sidx_v0.references = vec![
        SidxReference {
            reference_type: false,
            referenced_size: 0x01234567,
            subsegment_duration: 0x23456789,
            starts_with_sap: true,
            sap_type: 6,
            sap_delta_time: 0x09abcdef,
        },
        SidxReference {
            reference_type: true,
            referenced_size: 0x01234567,
            subsegment_duration: 0x23456789,
            starts_with_sap: false,
            sap_type: 5,
            sap_delta_time: 0x09abcdef,
        },
    ];

    let mut sidx_v1 = Sidx::default();
    sidx_v1.set_version(1);
    sidx_v1.reference_id = 0x01234567;
    sidx_v1.timescale = 0x23456789;
    sidx_v1.earliest_presentation_time_v1 = 0x0123456789abcdef;
    sidx_v1.first_offset_v1 = 0x23456789abcdef01;
    sidx_v1.reference_count = 2;
    sidx_v1.references = sidx_v0.references.clone();

    let mut tfra_v0 = Tfra::default();
    tfra_v0.set_version(0);
    tfra_v0.track_id = 0x11111111;
    tfra_v0.length_size_of_traf_num = 0x1;
    tfra_v0.length_size_of_trun_num = 0x2;
    tfra_v0.length_size_of_sample_num = 0x3;
    tfra_v0.number_of_entry = 2;
    tfra_v0.entries = vec![
        TfraEntry {
            time_v0: 0x22222222,
            moof_offset_v0: 0x33333333,
            traf_number: 0x4444,
            trun_number: 0x555555,
            sample_number: 0x66666666,
            ..TfraEntry::default()
        },
        TfraEntry {
            time_v0: 0x77777777,
            moof_offset_v0: 0x88888888,
            traf_number: 0x9999,
            trun_number: 0xaaaaaa,
            sample_number: 0xbbbbbbbb,
            ..TfraEntry::default()
        },
    ];

    let mut tfra_v1 = Tfra::default();
    tfra_v1.set_version(1);
    tfra_v1.track_id = 0x11111111;
    tfra_v1.length_size_of_traf_num = 0x1;
    tfra_v1.length_size_of_trun_num = 0x2;
    tfra_v1.length_size_of_sample_num = 0x3;
    tfra_v1.number_of_entry = 2;
    tfra_v1.entries = vec![
        TfraEntry {
            time_v1: 0x2222222222222222,
            moof_offset_v1: 0x3333333333333333,
            traf_number: 0x4444,
            trun_number: 0x555555,
            sample_number: 0x66666666,
            ..TfraEntry::default()
        },
        TfraEntry {
            time_v1: 0x7777777777777777,
            moof_offset_v1: 0x8888888888888888,
            traf_number: 0x9999,
            trun_number: 0xaaaaaa,
            sample_number: 0xbbbbbbbb,
            ..TfraEntry::default()
        },
    ];

    assert_box_roundtrip(
        hdlr,
        &[
            0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x56, 0x78, b'a', b'b', b'e', b'm', 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, b'A', b'b', b'e', b'm',
            b'a', 0x00,
        ],
        "Version=0 Flags=0x000000 PreDefined=305419896 HandlerType=\"abem\" Name=\"Abema\"",
    );
    assert_box_roundtrip(meta, &[0x00, 0x00, 0x00, 0x00], "Version=0 Flags=0x000000");
    assert_box_roundtrip(Schi, &[], "");
    assert_box_roundtrip(Sinf, &[], "");
    assert_box_roundtrip(Wave, &[], "");
    assert_box_roundtrip(
        saio_v0,
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x01, 0x23, 0x45, 0x67, 0x23, 0x45,
            0x67, 0x89, 0x45, 0x67, 0x89, 0xab,
        ],
        "Version=0 Flags=0x000000 EntryCount=3 OffsetV0=[19088743, 591751049, 1164413355]",
    );
    assert_box_roundtrip(
        saio_v0_aux,
        &[
            0x00, 0x00, 0x00, 0x01, b't', b'e', b's', b't', 0x89, 0xab, 0xcd, 0xef, 0x00, 0x00,
            0x00, 0x03, 0x01, 0x23, 0x45, 0x67, 0x23, 0x45, 0x67, 0x89, 0x45, 0x67, 0x89, 0xab,
        ],
        "Version=0 Flags=0x000001 AuxInfoType=\"test\" AuxInfoTypeParameter=0x89abcdef EntryCount=3 OffsetV0=[19088743, 591751049, 1164413355]",
    );
    assert_box_roundtrip(
        saio_v1,
        &[
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
        ],
        "Version=1 Flags=0x000000 EntryCount=2 OffsetV1=[81985529216486895, 81985529216486895]",
    );
    assert_box_roundtrip(
        saiz_default,
        &[0x00, 0x00, 0x00, 0x00, 0x01, 0x01, 0x23, 0x45, 0x67],
        "Version=0 Flags=0x000000 DefaultSampleInfoSize=1 SampleCount=19088743",
    );
    assert_box_roundtrip(
        saiz_array,
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x01, 0x02, 0x03,
        ],
        "Version=0 Flags=0x000000 DefaultSampleInfoSize=0 SampleCount=3 SampleInfoSize=[1, 2, 3]",
    );
    assert_box_roundtrip(
        saiz_aux,
        &[
            0x00, 0x00, 0x00, 0x01, b't', b'e', b's', b't', 0x89, 0xab, 0xcd, 0xef, 0x01, 0x01,
            0x23, 0x45, 0x67,
        ],
        "Version=0 Flags=0x000001 AuxInfoType=\"test\" AuxInfoTypeParameter=0x89abcdef DefaultSampleInfoSize=1 SampleCount=19088743",
    );
    assert_box_roundtrip(
        sbgp_v0,
        &[
            0x00, 0x00, 0x00, 0x00, 0x01, 0x23, 0x45, 0x67, 0x00, 0x00, 0x00, 0x02, 0x23, 0x45,
            0x67, 0x89, 0x34, 0x56, 0x78, 0x9a, 0x45, 0x67, 0x89, 0xab, 0x56, 0x78, 0x9a, 0xbc,
        ],
        "Version=0 Flags=0x000000 GroupingType=19088743 EntryCount=2 Entries=[{SampleCount=591751049 GroupDescriptionIndex=878082202}, {SampleCount=1164413355 GroupDescriptionIndex=1450744508}]",
    );
    assert_box_roundtrip(
        sbgp_v1,
        &[
            0x01, 0x00, 0x00, 0x00, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x00, 0x00,
            0x00, 0x02, 0x23, 0x45, 0x67, 0x89, 0x34, 0x56, 0x78, 0x9a, 0x45, 0x67, 0x89, 0xab,
            0x56, 0x78, 0x9a, 0xbc,
        ],
        "Version=1 Flags=0x000000 GroupingType=19088743 GroupingTypeParameter=2309737967 EntryCount=2 Entries=[{SampleCount=591751049 GroupDescriptionIndex=878082202}, {SampleCount=1164413355 GroupDescriptionIndex=1450744508}]",
    );
    assert_box_roundtrip(
        sdtp,
        &[0x00, 0x00, 0x00, 0x00, 0x00, 0x1b, 0xe4],
        "Version=0 Flags=0x000000 Samples=[{IsLeading=0x0 SampleDependsOn=0x0 SampleIsDependedOn=0x0 SampleHasRedundancy=0x0}, {IsLeading=0x0 SampleDependsOn=0x1 SampleIsDependedOn=0x2 SampleHasRedundancy=0x3}, {IsLeading=0x3 SampleDependsOn=0x2 SampleIsDependedOn=0x1 SampleHasRedundancy=0x0}]",
    );
    assert_box_roundtrip(
        sgpd_roll_v1,
        &[
            0x01, 0x00, 0x00, 0x00, b'r', b'o', b'l', b'l', 0x00, 0x00, 0x00, 0x02, 0x00, 0x00,
            0x00, 0x02, 0x11, 0x11, 0x22, 0x22,
        ],
        "Version=1 Flags=0x000000 GroupingType=\"roll\" DefaultLength=2 EntryCount=2 RollDistances=[4369, 8738]",
    );
    assert_box_roundtrip(
        sgpd_prol_v1,
        &[
            0x01, 0x00, 0x00, 0x00, b'p', b'r', b'o', b'l', 0x00, 0x00, 0x00, 0x02, 0x00, 0x00,
            0x00, 0x02, 0x11, 0x11, 0x22, 0x22,
        ],
        "Version=1 Flags=0x000000 GroupingType=\"prol\" DefaultLength=2 EntryCount=2 RollDistances=[4369, 8738]",
    );
    assert_box_roundtrip(
        sgpd_alst_v1,
        &[
            0x01, 0x00, 0x00, 0x00, b'a', b'l', b's', b't', 0x00, 0x00, 0x00, 0x0c, 0x00, 0x00,
            0x00, 0x02, 0x00, 0x02, 0x01, 0x23, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
            0x00, 0x02, 0x45, 0x67, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23,
        ],
        "Version=1 Flags=0x000000 GroupingType=\"alst\" DefaultLength=12 EntryCount=2 AlternativeStartupEntries=[{RollCount=2 FirstOutputSample=291 SampleOffset=[19088743, 2309737967] Opts=[]}, {RollCount=2 FirstOutputSample=17767 SampleOffset=[1164413355, 3454992675] Opts=[]}]",
    );
    assert_box_roundtrip(
        sgpd_alst_default_v1,
        &[
            0x01, 0x00, 0x00, 0x00, b'a', b'l', b's', b't', 0x00, 0x00, 0x00, 0x14, 0x00, 0x00,
            0x00, 0x02, 0x00, 0x02, 0x01, 0x23, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x00, 0x02, 0x45, 0x67, 0x45, 0x67,
            0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
        ],
        "Version=1 Flags=0x000000 GroupingType=\"alst\" DefaultLength=20 EntryCount=2 AlternativeStartupEntries=[{RollCount=2 FirstOutputSample=291 SampleOffset=[19088743, 2309737967] Opts=[{NumOutputSamples=291 NumTotalSamples=17767}, {NumOutputSamples=35243 NumTotalSamples=52719}]}, {RollCount=2 FirstOutputSample=17767 SampleOffset=[1164413355, 3454992675] Opts=[{NumOutputSamples=291 NumTotalSamples=17767}, {NumOutputSamples=35243 NumTotalSamples=52719}]}]",
    );
    assert_box_roundtrip(
        sgpd_alst_len_v1,
        &[
            0x01, 0x00, 0x00, 0x00, b'a', b'l', b's', b't', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x02, 0x00, 0x00, 0x00, 0x10, 0x00, 0x02, 0x01, 0x23, 0x01, 0x23, 0x45, 0x67,
            0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x00, 0x00, 0x00, 0x14, 0x00, 0x02,
            0x45, 0x67, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x01, 0x23, 0x45, 0x67,
            0x89, 0xab, 0xcd, 0xef,
        ],
        "Version=1 Flags=0x000000 GroupingType=\"alst\" DefaultLength=0 EntryCount=2 AlternativeStartupEntriesL=[{DescriptionLength=16 RollCount=2 FirstOutputSample=291 SampleOffset=[19088743, 2309737967] Opts=[{NumOutputSamples=291 NumTotalSamples=17767}]}, {DescriptionLength=20 RollCount=2 FirstOutputSample=17767 SampleOffset=[1164413355, 3454992675] Opts=[{NumOutputSamples=291 NumTotalSamples=17767}, {NumOutputSamples=35243 NumTotalSamples=52719}]}]",
    );
    assert_box_roundtrip(
        sgpd_rap_v1,
        &[
            0x01, 0x00, 0x00, 0x00, b'r', b'a', b'p', b' ', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
            0x00, 0x02, 0xa7, 0x1a,
        ],
        "Version=1 Flags=0x000000 GroupingType=\"rap \" DefaultLength=1 EntryCount=2 VisualRandomAccessEntries=[{NumLeadingSamplesKnown=true NumLeadingSamples=0x27}, {NumLeadingSamplesKnown=false NumLeadingSamples=0x1a}]",
    );
    assert_box_roundtrip(
        sgpd_tele_v1,
        &[
            0x01, 0x00, 0x00, 0x00, b't', b'e', b'l', b'e', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
            0x00, 0x02, 0x80, 0x00,
        ],
        "Version=1 Flags=0x000000 GroupingType=\"tele\" DefaultLength=1 EntryCount=2 TemporalLevelEntries=[{LevelIndependentlyDecodable=true}, {LevelIndependentlyDecodable=false}]",
    );
    assert_box_roundtrip(
        sgpd_roll_v2,
        &[
            0x02, 0x00, 0x00, 0x00, b'r', b'o', b'l', b'l', 0x00, 0x00, 0x00, 0x05, 0x00, 0x00,
            0x00, 0x02, 0x11, 0x11, 0x22, 0x22,
        ],
        "Version=2 Flags=0x000000 GroupingType=\"roll\" DefaultSampleDescriptionIndex=5 EntryCount=2 RollDistances=[4369, 8738]",
    );
    assert_box_roundtrip(
        sidx_v0,
        &[
            0x00, 0x00, 0x00, 0x00, 0x01, 0x23, 0x45, 0x67, 0x23, 0x45, 0x67, 0x89, 0x45, 0x67,
            0x89, 0xab, 0x67, 0x89, 0xab, 0xcd, 0x00, 0x00, 0x00, 0x02, 0x01, 0x23, 0x45, 0x67,
            0x23, 0x45, 0x67, 0x89, 0xe9, 0xab, 0xcd, 0xef, 0x81, 0x23, 0x45, 0x67, 0x23, 0x45,
            0x67, 0x89, 0x59, 0xab, 0xcd, 0xef,
        ],
        "Version=0 Flags=0x000000 ReferenceID=19088743 Timescale=591751049 EarliestPresentationTimeV0=1164413355 FirstOffsetV0=1737075661 ReferenceCount=2 References=[{ReferenceType=false ReferencedSize=19088743 SubsegmentDuration=591751049 StartsWithSAP=true SAPType=6 SAPDeltaTime=162254319}, {ReferenceType=true ReferencedSize=19088743 SubsegmentDuration=591751049 StartsWithSAP=false SAPType=5 SAPDeltaTime=162254319}]",
    );
    assert_box_roundtrip(
        sidx_v1,
        &[
            0x01, 0x00, 0x00, 0x00, 0x01, 0x23, 0x45, 0x67, 0x23, 0x45, 0x67, 0x89, 0x01, 0x23,
            0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01,
            0x00, 0x00, 0x00, 0x02, 0x01, 0x23, 0x45, 0x67, 0x23, 0x45, 0x67, 0x89, 0xe9, 0xab,
            0xcd, 0xef, 0x81, 0x23, 0x45, 0x67, 0x23, 0x45, 0x67, 0x89, 0x59, 0xab, 0xcd, 0xef,
        ],
        "Version=1 Flags=0x000000 ReferenceID=19088743 Timescale=591751049 EarliestPresentationTimeV1=81985529216486895 FirstOffsetV1=2541551405711093505 ReferenceCount=2 References=[{ReferenceType=false ReferencedSize=19088743 SubsegmentDuration=591751049 StartsWithSAP=true SAPType=6 SAPDeltaTime=162254319}, {ReferenceType=true ReferencedSize=19088743 SubsegmentDuration=591751049 StartsWithSAP=false SAPType=5 SAPDeltaTime=162254319}]",
    );
    assert_box_roundtrip(
        tfra_v0,
        &[
            0x00, 0x00, 0x00, 0x00, 0x11, 0x11, 0x11, 0x11, 0x00, 0x00, 0x00, 0x1b, 0x00, 0x00,
            0x00, 0x02, 0x22, 0x22, 0x22, 0x22, 0x33, 0x33, 0x33, 0x33, 0x44, 0x44, 0x55, 0x55,
            0x55, 0x66, 0x66, 0x66, 0x66, 0x77, 0x77, 0x77, 0x77, 0x88, 0x88, 0x88, 0x88, 0x99,
            0x99, 0xaa, 0xaa, 0xaa, 0xbb, 0xbb, 0xbb, 0xbb,
        ],
        "Version=0 Flags=0x000000 TrackID=286331153 LengthSizeOfTrafNum=0x1 LengthSizeOfTrunNum=0x2 LengthSizeOfSampleNum=0x3 NumberOfEntry=2 Entries=[{TimeV0=572662306 MoofOffsetV0=858993459 TrafNumber=17476 TrunNumber=5592405 SampleNumber=1717986918}, {TimeV0=2004318071 MoofOffsetV0=2290649224 TrafNumber=39321 TrunNumber=11184810 SampleNumber=3149642683}]",
    );
    assert_box_roundtrip(
        tfra_v1,
        &[
            0x01, 0x00, 0x00, 0x00, 0x11, 0x11, 0x11, 0x11, 0x00, 0x00, 0x00, 0x1b, 0x00, 0x00,
            0x00, 0x02, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x33, 0x33, 0x33, 0x33,
            0x33, 0x33, 0x33, 0x33, 0x44, 0x44, 0x55, 0x55, 0x55, 0x66, 0x66, 0x66, 0x66, 0x77,
            0x77, 0x77, 0x77, 0x77, 0x77, 0x77, 0x77, 0x88, 0x88, 0x88, 0x88, 0x88, 0x88, 0x88,
            0x88, 0x99, 0x99, 0xaa, 0xaa, 0xaa, 0xbb, 0xbb, 0xbb, 0xbb,
        ],
        "Version=1 Flags=0x000000 TrackID=286331153 LengthSizeOfTrafNum=0x1 LengthSizeOfTrunNum=0x2 LengthSizeOfSampleNum=0x3 NumberOfEntry=2 Entries=[{TimeV1=2459565876494606882 MoofOffsetV1=3689348814741910323 TrafNumber=17476 TrunNumber=5592405 SampleNumber=1717986918}, {TimeV1=8608480567731124087 MoofOffsetV1=9838263505978427528 TrafNumber=39321 TrunNumber=11184810 SampleNumber=3149642683}]",
    );
}

#[test]
fn sample_entry_and_leaf_iso14496_12_catalog_roundtrips() {
    let mut emsg_v0 = Emsg::default();
    emsg_v0.set_version(0);
    emsg_v0.scheme_id_uri = String::from("urn:test");
    emsg_v0.value = String::from("foo");
    emsg_v0.timescale = 1000;
    emsg_v0.presentation_time_delta = 123;
    emsg_v0.event_duration = 3000;
    emsg_v0.id = 0xabcd;
    emsg_v0.message_data = b"abema".to_vec();

    let mut emsg_v1 = Emsg::default();
    emsg_v1.set_version(1);
    emsg_v1.scheme_id_uri = String::from("urn:test");
    emsg_v1.value = String::from("foo");
    emsg_v1.timescale = 1000;
    emsg_v1.presentation_time = 123;
    emsg_v1.event_duration = 3000;
    emsg_v1.id = 0xabcd;
    emsg_v1.message_data = b"abema".to_vec();

    let mut schm_uri = Schm::default();
    schm_uri.set_version(0);
    schm_uri.set_flags(0x000001);
    schm_uri.scheme_type = FourCc::from_bytes(*b"test");
    schm_uri.scheme_version = 0x12345678;
    schm_uri.scheme_uri = String::from("foo://bar/baz");

    let mut schm = Schm::default();
    schm.set_version(0);
    schm.scheme_type = FourCc::from_bytes(*b"test");
    schm.scheme_version = 0x12345678;

    let visual = VisualSampleEntry {
        sample_entry: SampleEntry {
            box_type: FourCc::from_bytes(*b"avc1"),
            data_reference_index: 0x1234,
        },
        pre_defined: 0x0101,
        pre_defined2: [0x01000001, 0x01000002, 0x01000003],
        width: 0x0102,
        height: 0x0103,
        horizresolution: 0x01000004,
        vertresolution: 0x01000005,
        reserved2: 0x01000006,
        frame_count: 0x0104,
        compressorname: [
            8, b'a', b'b', b'e', b'm', b'a', 0x00, b't', b'v', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ],
        depth: 0x0105,
        pre_defined3: 1001,
    };

    let audio = AudioSampleEntry {
        sample_entry: SampleEntry {
            box_type: FourCc::from_bytes(*b"enca"),
            data_reference_index: 0x1234,
        },
        entry_version: 0x0123,
        channel_count: 0x2345,
        sample_size: 0x4567,
        pre_defined: 0x6789,
        sample_rate: 0x01234567,
        quicktime_data: Vec::new(),
    };

    let audio_qt_v1 = AudioSampleEntry {
        sample_entry: SampleEntry {
            box_type: FourCc::from_bytes(*b"enca"),
            data_reference_index: 0x1234,
        },
        entry_version: 1,
        channel_count: 0x2345,
        sample_size: 0x4567,
        pre_defined: 0x6789,
        sample_rate: 0x01234567,
        quicktime_data: vec![
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ],
    };

    let audio_qt_v2 = AudioSampleEntry {
        sample_entry: SampleEntry {
            box_type: FourCc::from_bytes(*b"enca"),
            data_reference_index: 0x1234,
        },
        entry_version: 2,
        channel_count: 0x2345,
        sample_size: 0x4567,
        pre_defined: 0x6789,
        sample_rate: 0x01234567,
        quicktime_data: vec![
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb,
            0xcc, 0xdd, 0xee, 0xff, 0x00, 0x11, 0x22, 0x33,
        ],
    };

    let avcc_main = AVCDecoderConfiguration {
        configuration_version: 0x12,
        profile: 0x4d,
        profile_compatibility: 0x40,
        level: 0x1f,
        length_size_minus_one: 0x02,
        num_of_sequence_parameter_sets: 2,
        sequence_parameter_sets: vec![
            AVCParameterSet {
                length: 2,
                nal_unit: vec![0x12, 0x34],
            },
            AVCParameterSet {
                length: 3,
                nal_unit: vec![0x12, 0x34, 0x56],
            },
        ],
        num_of_picture_parameter_sets: 2,
        picture_parameter_sets: vec![
            AVCParameterSet {
                length: 2,
                nal_unit: vec![0xab, 0xcd],
            },
            AVCParameterSet {
                length: 3,
                nal_unit: vec![0xab, 0xcd, 0xef],
            },
        ],
        ..AVCDecoderConfiguration::default()
    };

    let avcc_high_old = AVCDecoderConfiguration {
        configuration_version: 0x12,
        profile: 0x64,
        profile_compatibility: 0x00,
        level: 0x28,
        length_size_minus_one: 0x02,
        num_of_sequence_parameter_sets: 2,
        sequence_parameter_sets: avcc_main.sequence_parameter_sets.clone(),
        num_of_picture_parameter_sets: 2,
        picture_parameter_sets: avcc_main.picture_parameter_sets.clone(),
        ..AVCDecoderConfiguration::default()
    };

    let avcc_high_new = AVCDecoderConfiguration {
        configuration_version: 0x12,
        profile: 0x64,
        profile_compatibility: 0x00,
        level: 0x28,
        length_size_minus_one: 0x02,
        num_of_sequence_parameter_sets: 2,
        sequence_parameter_sets: avcc_main.sequence_parameter_sets.clone(),
        num_of_picture_parameter_sets: 2,
        picture_parameter_sets: avcc_main.picture_parameter_sets.clone(),
        high_profile_fields_enabled: true,
        chroma_format: 0x01,
        bit_depth_luma_minus8: 0x02,
        bit_depth_chroma_minus8: 0x03,
        num_of_sequence_parameter_set_ext: 2,
        sequence_parameter_sets_ext: vec![
            AVCParameterSet {
                length: 2,
                nal_unit: vec![0x12, 0x34],
            },
            AVCParameterSet {
                length: 3,
                nal_unit: vec![0x12, 0x34, 0x56],
            },
        ],
    };

    let hvcc = HEVCDecoderConfiguration {
        configuration_version: 0x01,
        general_profile_idc: 0x01,
        general_profile_compatibility: [
            false, true, true, false, false, false, false, false, false, false, false, false,
            false, false, false, false, false, false, false, false, false, false, false, false,
            false, false, false, false, false, false, false, false,
        ],
        general_constraint_indicator: [0x90, 0x00, 0x00, 0x00, 0x00, 0x00],
        general_level_idc: 0x78,
        min_spatial_segmentation_idc: 0x0000,
        chroma_format_idc: 0x01,
        temporal_id_nested: 0x03,
        length_size_minus_one: 0x03,
        num_of_nalu_arrays: 4,
        nalu_arrays: vec![
            HEVCNaluArray {
                nalu_type: 0x20,
                num_nalus: 1,
                nalus: vec![HEVCNalu {
                    length: 24,
                    nal_unit: vec![
                        0x40, 0x01, 0x0c, 0x01, 0xff, 0xff, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00,
                        0x90, 0x00, 0x00, 0x03, 0x00, 0x00, 0x03, 0x00, 0x78, 0x99, 0x98, 0x09,
                    ],
                }],
                ..HEVCNaluArray::default()
            },
            HEVCNaluArray {
                nalu_type: 0x21,
                num_nalus: 1,
                nalus: vec![HEVCNalu {
                    length: 42,
                    nal_unit: vec![
                        0x06, 0x01, 0x01, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x90, 0x00, 0x00,
                        0x03, 0x00, 0x00, 0x03, 0x00, 0x78, 0xa0, 0x03, 0xc0, 0x80, 0x10, 0xe5,
                        0x96, 0x66, 0x69, 0x24, 0xca, 0xe0, 0x10, 0x00, 0x00, 0x03, 0x00, 0x10,
                        0x00, 0x00, 0x03, 0x01, 0xe0, 0x80,
                    ],
                }],
                ..HEVCNaluArray::default()
            },
            HEVCNaluArray {
                nalu_type: 0x22,
                num_nalus: 1,
                nalus: vec![HEVCNalu {
                    length: 7,
                    nal_unit: vec![0x44, 0x01, 0xc1, 0x72, 0xb4, 0x62, 0x40],
                }],
                ..HEVCNaluArray::default()
            },
            HEVCNaluArray {
                nalu_type: 0x27,
                num_nalus: 1,
                nalus: vec![HEVCNalu {
                    length: 11,
                    nal_unit: vec![
                        0x4e, 0x01, 0x05, 0xff, 0xff, 0xff, 0xa6, 0x2c, 0xa2, 0xde, 0x09,
                    ],
                }],
                ..HEVCNaluArray::default()
            },
        ],
        ..HEVCDecoderConfiguration::default()
    };

    let stpp = XMLSubtitleSampleEntry {
        sample_entry: SampleEntry {
            box_type: FourCc::from_bytes(*b"stpp"),
            data_reference_index: 0x1234,
        },
        namespace: String::from("http://foo.org/bar http://baz.org/qux"),
        schema_location: String::from("http://quux.org/corge"),
        auxiliary_mime_types: String::from("xxx/yyy"),
    };

    let sbtt = TextSubtitleSampleEntry {
        sample_entry: SampleEntry {
            box_type: FourCc::from_bytes(*b"sbtt"),
            data_reference_index: 0x1234,
        },
        content_encoding: String::from("foo"),
        mime_format: String::from("bar/baz"),
    };

    assert_box_roundtrip(
        Btrt {
            buffer_size_db: 0x12345678,
            max_bitrate: 0x3456789a,
            avg_bitrate: 0x56789abc,
        },
        &[
            0x12, 0x34, 0x56, 0x78, 0x34, 0x56, 0x78, 0x9a, 0x56, 0x78, 0x9a, 0xbc,
        ],
        "BufferSizeDB=305419896 MaxBitrate=878082202 AvgBitrate=1450744508",
    );
    assert_box_roundtrip(
        Colr {
            colour_type: FourCc::from_bytes(*b"nclx"),
            colour_primaries: 0x0123,
            transfer_characteristics: 0x2345,
            matrix_coefficients: 0x4567,
            full_range_flag: true,
            reserved: 0x67,
            ..Colr::default()
        },
        &[
            b'n', b'c', b'l', b'x', 0x01, 0x23, 0x23, 0x45, 0x45, 0x67, 0xe7,
        ],
        "ColourType=\"nclx\" ColourPrimaries=291 TransferCharacteristics=9029 MatrixCoefficients=17767 FullRangeFlag=true Reserved=0x67",
    );
    assert_box_roundtrip(
        Colr {
            colour_type: FourCc::from_bytes(*b"rICC"),
            profile: vec![0x12, 0x34, 0x56, 0x78, 0xab],
            ..Colr::default()
        },
        &[b'r', b'I', b'C', b'C', 0x12, 0x34, 0x56, 0x78, 0xab],
        "ColourType=\"rICC\" Profile=[0x12, 0x34, 0x56, 0x78, 0xab]",
    );
    assert_box_roundtrip(
        Colr {
            colour_type: FourCc::from_bytes(*b"nclc"),
            unknown: vec![0x01, 0x23, 0x45],
            ..Colr::default()
        },
        &[b'n', b'c', b'l', b'c', 0x01, 0x23, 0x45],
        "ColourType=\"nclc\" Unknown=[0x1, 0x23, 0x45]",
    );
    assert_box_roundtrip(
        emsg_v0,
        &[
            0x00, 0x00, 0x00, 0x00, 0x75, 0x72, 0x6e, 0x3a, 0x74, 0x65, 0x73, 0x74, 0x00, 0x66,
            0x6f, 0x6f, 0x00, 0x00, 0x00, 0x03, 0xe8, 0x00, 0x00, 0x00, 0x7b, 0x00, 0x00, 0x0b,
            0xb8, 0x00, 0x00, 0xab, 0xcd, 0x61, 0x62, 0x65, 0x6d, 0x61,
        ],
        "Version=0 Flags=0x000000 SchemeIdUri=\"urn:test\" Value=\"foo\" Timescale=1000 PresentationTimeDelta=123 EventDuration=3000 Id=43981 MessageData=\"abema\"",
    );
    assert_box_roundtrip(
        emsg_v1,
        &[
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0xe8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x7b, 0x00, 0x00, 0x0b, 0xb8, 0x00, 0x00, 0xab, 0xcd, 0x75, 0x72, 0x6e, 0x3a,
            0x74, 0x65, 0x73, 0x74, 0x00, 0x66, 0x6f, 0x6f, 0x00, 0x61, 0x62, 0x65, 0x6d, 0x61,
        ],
        "Version=1 Flags=0x000000 SchemeIdUri=\"urn:test\" Value=\"foo\" Timescale=1000 PresentationTime=123 EventDuration=3000 Id=43981 MessageData=\"abema\"",
    );
    assert_box_roundtrip(
        Fiel {
            field_count: 0xe9,
            field_ordering: 0x70,
        },
        &[0xe9, 0x70],
        "FieldCount=0xe9 FieldOrdering=0x70",
    );
    assert_box_roundtrip(
        Frma {
            data_format: FourCc::from_bytes(*b"test"),
        },
        b"test",
        "DataFormat=\"test\"",
    );
    assert_box_roundtrip(
        Pasp {
            h_spacing: 0x01234567,
            v_spacing: 0x23456789,
        },
        &[0x01, 0x23, 0x45, 0x67, 0x23, 0x45, 0x67, 0x89],
        "HSpacing=19088743 VSpacing=591751049",
    );
    assert_box_roundtrip(
        schm,
        &[
            0x00, 0x00, 0x00, 0x00, b't', b'e', b's', b't', 0x12, 0x34, 0x56, 0x78,
        ],
        "Version=0 Flags=0x000000 SchemeType=\"test\" SchemeVersion=0x12345678",
    );
    assert_box_roundtrip(
        schm_uri,
        &[
            0x00, 0x00, 0x00, 0x01, b't', b'e', b's', b't', 0x12, 0x34, 0x56, 0x78, b'f', b'o',
            b'o', b':', b'/', b'/', b'b', b'a', b'r', b'/', b'b', b'a', b'z',
        ],
        "Version=0 Flags=0x000001 SchemeType=\"test\" SchemeVersion=0x12345678 SchemeUri=\"foo://bar/baz\"",
    );
    assert_box_roundtrip(
        avcc_main,
        &[
            0x12, 0x4d, 0x40, 0x1f, 0xfe, 0xe2, 0x00, 0x02, 0x12, 0x34, 0x00, 0x03, 0x12, 0x34,
            0x56, 0x02, 0x00, 0x02, 0xab, 0xcd, 0x00, 0x03, 0xab, 0xcd, 0xef,
        ],
        "ConfigurationVersion=0x12 Profile=0x4d ProfileCompatibility=0x40 Level=0x1f LengthSizeMinusOne=0x2 NumOfSequenceParameterSets=0x2 SequenceParameterSets=[{Length=2 NALUnit=[0x12, 0x34]}, {Length=3 NALUnit=[0x12, 0x34, 0x56]}] NumOfPictureParameterSets=0x2 PictureParameterSets=[{Length=2 NALUnit=[0xab, 0xcd]}, {Length=3 NALUnit=[0xab, 0xcd, 0xef]}]",
    );
    assert_box_roundtrip(
        avcc_high_old,
        &[
            0x12, 0x64, 0x00, 0x28, 0xfe, 0xe2, 0x00, 0x02, 0x12, 0x34, 0x00, 0x03, 0x12, 0x34,
            0x56, 0x02, 0x00, 0x02, 0xab, 0xcd, 0x00, 0x03, 0xab, 0xcd, 0xef,
        ],
        "ConfigurationVersion=0x12 Profile=0x64 ProfileCompatibility=0x0 Level=0x28 LengthSizeMinusOne=0x2 NumOfSequenceParameterSets=0x2 SequenceParameterSets=[{Length=2 NALUnit=[0x12, 0x34]}, {Length=3 NALUnit=[0x12, 0x34, 0x56]}] NumOfPictureParameterSets=0x2 PictureParameterSets=[{Length=2 NALUnit=[0xab, 0xcd]}, {Length=3 NALUnit=[0xab, 0xcd, 0xef]}]",
    );
    assert_box_roundtrip(
        avcc_high_new,
        &[
            0x12, 0x64, 0x00, 0x28, 0xfe, 0xe2, 0x00, 0x02, 0x12, 0x34, 0x00, 0x03, 0x12, 0x34,
            0x56, 0x02, 0x00, 0x02, 0xab, 0xcd, 0x00, 0x03, 0xab, 0xcd, 0xef, 0xfd, 0xfa, 0xfb,
            0x02, 0x00, 0x02, 0x12, 0x34, 0x00, 0x03, 0x12, 0x34, 0x56,
        ],
        "ConfigurationVersion=0x12 Profile=0x64 ProfileCompatibility=0x0 Level=0x28 LengthSizeMinusOne=0x2 NumOfSequenceParameterSets=0x2 SequenceParameterSets=[{Length=2 NALUnit=[0x12, 0x34]}, {Length=3 NALUnit=[0x12, 0x34, 0x56]}] NumOfPictureParameterSets=0x2 PictureParameterSets=[{Length=2 NALUnit=[0xab, 0xcd]}, {Length=3 NALUnit=[0xab, 0xcd, 0xef]}] ChromaFormat=0x1 BitDepthLumaMinus8=0x2 BitDepthChromaMinus8=0x3 NumOfSequenceParameterSetExt=0x2 SequenceParameterSetsExt=[{Length=2 NALUnit=[0x12, 0x34]}, {Length=3 NALUnit=[0x12, 0x34, 0x56]}]",
    );
    assert_box_roundtrip(
        hvcc,
        &[
            0x01, 0x01, 0x60, 0x00, 0x00, 0x00, 0x90, 0x00, 0x00, 0x00, 0x00, 0x00, 0x78, 0xe0,
            0x00, 0xfc, 0xfd, 0xf8, 0xf8, 0x00, 0x00, 0x0f, 0x04, 0x20, 0x00, 0x01, 0x00, 0x18,
            0x40, 0x01, 0x0c, 0x01, 0xff, 0xff, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x90, 0x00,
            0x00, 0x03, 0x00, 0x00, 0x03, 0x00, 0x78, 0x99, 0x98, 0x09, 0x21, 0x00, 0x01, 0x00,
            0x2a, 0x06, 0x01, 0x01, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x90, 0x00, 0x00, 0x03,
            0x00, 0x00, 0x03, 0x00, 0x78, 0xa0, 0x03, 0xc0, 0x80, 0x10, 0xe5, 0x96, 0x66, 0x69,
            0x24, 0xca, 0xe0, 0x10, 0x00, 0x00, 0x03, 0x00, 0x10, 0x00, 0x00, 0x03, 0x01, 0xe0,
            0x80, 0x22, 0x00, 0x01, 0x00, 0x07, 0x44, 0x01, 0xc1, 0x72, 0xb4, 0x62, 0x40, 0x27,
            0x00, 0x01, 0x00, 0x0b, 0x4e, 0x01, 0x05, 0xff, 0xff, 0xff, 0xa6, 0x2c, 0xa2, 0xde,
            0x09,
        ],
        concat!(
            "ConfigurationVersion=0x1 GeneralProfileSpace=0x0 GeneralTierFlag=false ",
            "GeneralProfileIdc=0x1 GeneralProfileCompatibility=[false, true, true, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false] ",
            "GeneralConstraintIndicator=[0x90, 0x0, 0x0, 0x0, 0x0, 0x0] GeneralLevelIdc=0x78 ",
            "MinSpatialSegmentationIdc=0 ParallelismType=0x0 ChromaFormatIdc=0x1 ",
            "BitDepthLumaMinus8=0x0 BitDepthChromaMinus8=0x0 AvgFrameRate=0 ConstantFrameRate=0x0 ",
            "NumTemporalLayers=0x0 TemporalIdNested=0x3 LengthSizeMinusOne=0x3 NumOfNaluArrays=0x4 ",
            "NaluArrays=[{Completeness=false Reserved=false NaluType=0x20 NumNalus=1 Nalus=[{Length=24 NALUnit=[0x40, 0x1, 0xc, 0x1, 0xff, 0xff, 0x1, 0x60, 0x0, 0x0, 0x3, 0x0, 0x90, 0x0, 0x0, 0x3, 0x0, 0x0, 0x3, 0x0, 0x78, 0x99, 0x98, 0x9]}]}, ",
            "{Completeness=false Reserved=false NaluType=0x21 NumNalus=1 Nalus=[{Length=42 NALUnit=[0x6, 0x1, 0x1, 0x1, 0x60, 0x0, 0x0, 0x3, 0x0, 0x90, 0x0, 0x0, 0x3, 0x0, 0x0, 0x3, 0x0, 0x78, 0xa0, 0x3, 0xc0, 0x80, 0x10, 0xe5, 0x96, 0x66, 0x69, 0x24, 0xca, 0xe0, 0x10, 0x0, 0x0, 0x3, 0x0, 0x10, 0x0, 0x0, 0x3, 0x1, 0xe0, 0x80]}]}, ",
            "{Completeness=false Reserved=false NaluType=0x22 NumNalus=1 Nalus=[{Length=7 NALUnit=[0x44, 0x1, 0xc1, 0x72, 0xb4, 0x62, 0x40]}]}, ",
            "{Completeness=false Reserved=false NaluType=0x27 NumNalus=1 Nalus=[{Length=11 NALUnit=[0x4e, 0x1, 0x5, 0xff, 0xff, 0xff, 0xa6, 0x2c, 0xa2, 0xde, 0x9]}]}]",
        ),
    );
    assert_box_roundtrip(
        stpp,
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x12, 0x34, b'h', b't', b't', b'p', b':', b'/',
            b'/', b'f', b'o', b'o', b'.', b'o', b'r', b'g', b'/', b'b', b'a', b'r', b' ', b'h',
            b't', b't', b'p', b':', b'/', b'/', b'b', b'a', b'z', b'.', b'o', b'r', b'g', b'/',
            b'q', b'u', b'x', 0x00, b'h', b't', b't', b'p', b':', b'/', b'/', b'q', b'u', b'u',
            b'x', b'.', b'o', b'r', b'g', b'/', b'c', b'o', b'r', b'g', b'e', 0x00, b'x', b'x',
            b'x', b'/', b'y', b'y', b'y', 0x00,
        ],
        "DataReferenceIndex=4660 Namespace=\"http://foo.org/bar http://baz.org/qux\" SchemaLocation=\"http://quux.org/corge\" AuxiliaryMIMETypes=\"xxx/yyy\"",
    );
    assert_box_roundtrip(
        sbtt,
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x12, 0x34, b'f', b'o', b'o', 0x00, b'b', b'a',
            b'r', b'/', b'b', b'a', b'z', 0x00,
        ],
        "DataReferenceIndex=4660 ContentEncoding=\"foo\" MIMEFormat=\"bar/baz\"",
    );
    assert_any_box_roundtrip(
        visual,
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00,
            0x00, 0x01, 0x01, 0x00, 0x00, 0x02, 0x01, 0x00, 0x00, 0x03, 0x01, 0x02, 0x01, 0x03,
            0x01, 0x00, 0x00, 0x04, 0x01, 0x00, 0x00, 0x05, 0x01, 0x00, 0x00, 0x06, 0x01, 0x04,
            0x08, b'a', b'b', b'e', b'm', b'a', 0x00, b't', b'v', 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x01, 0x05, 0x03, 0xe9,
        ],
        "DataReferenceIndex=4660 PreDefined=257 PreDefined2=[16777217, 16777218, 16777219] Width=258 Height=259 Horizresolution=16777220 Vertresolution=16777221 FrameCount=260 Compressorname=\"abema.tv\" Depth=261 PreDefined3=1001",
    );
    assert_any_box_roundtrip(
        audio,
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x01, 0x23, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x23, 0x45, 0x45, 0x67, 0x67, 0x89, 0x00, 0x00, 0x01, 0x23, 0x45, 0x67,
        ],
        "DataReferenceIndex=4660 EntryVersion=291 ChannelCount=9029 SampleSize=17767 PreDefined=26505 SampleRate=291.27110",
    );
    assert_any_box_roundtrip(
        audio_qt_v1,
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x23, 0x45, 0x45, 0x67, 0x67, 0x89, 0x00, 0x00, 0x01, 0x23, 0x45, 0x67,
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ],
        "DataReferenceIndex=4660 EntryVersion=1 ChannelCount=9029 SampleSize=17767 PreDefined=26505 SampleRate=291.27110 QuickTimeData=[0x0, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]",
    );
    assert_any_box_roundtrip(
        audio_qt_v2,
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x23, 0x45, 0x45, 0x67, 0x67, 0x89, 0x00, 0x00, 0x01, 0x23, 0x45, 0x67,
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb,
            0xcc, 0xdd, 0xee, 0xff, 0x00, 0x11, 0x22, 0x33,
        ],
        "DataReferenceIndex=4660 EntryVersion=2 ChannelCount=9029 SampleSize=17767 PreDefined=26505 SampleRate=291.27110 QuickTimeData=[0x0, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x0, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x0, 0x11, 0x22, 0x33]",
    );
}

#[test]
fn irregular_decode_helpers_match_reference_behavior() {
    let handler_cases = [
        ([0x00, 0x00, 0x00, 0x00], b"abema".as_slice(), "abema"),
        ([0x00, 0x00, 0x00, 0x00], b"".as_slice(), ""),
        (
            [0x00, 0x00, 0x00, 0x00],
            b" a 1st byte equals to this length".as_slice(),
            " a 1st byte equals to this length",
        ),
        (*b"mhlr", &[5, b'a', b'b', b'e', b'm', b'a'][..], "abema"),
        (*b"mhlr", &[0x00, 0x00][..], ""),
        (
            *b"mhlr",
            b" a 1st byte equals to this length".as_slice(),
            "a 1st byte equals to this length",
        ),
    ];

    for (pre_defined, name_bytes, expected_name) in handler_cases {
        let mut payload = Vec::new();
        payload.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        payload.extend_from_slice(&pre_defined);
        payload.extend_from_slice(b"vide");
        payload.extend_from_slice(&[0x00; 12]);
        payload.extend_from_slice(name_bytes);

        let mut decoded = Hdlr::default();
        let mut reader = Cursor::new(payload.clone());
        let read = unmarshal(&mut reader, payload.len() as u64, &mut decoded, None).unwrap();
        assert_eq!(read, payload.len() as u64);
        assert_eq!(decoded.pre_defined.to_be_bytes(), pre_defined);
        assert_eq!(decoded.handler_type, FourCc::from_bytes(*b"vide"));
        assert_eq!(decoded.name, expected_name);
    }

    let meta_payload = [
        0x00, 0x00, 0x01, 0x00, b'h', b'd', b'l', b'r', 0x00, 0x00, 0x00, 0x00,
    ];
    let mut meta = Meta::default();
    let mut meta_reader = Cursor::new(meta_payload);
    let meta_read =
        unmarshal(&mut meta_reader, meta_payload.len() as u64, &mut meta, None).unwrap();
    assert_eq!(meta_read, 0);
    assert_eq!(meta_reader.position(), 0);
    assert!(meta.is_quicktime_headerless());
    assert_eq!(meta.version(), 0);
    assert_eq!(meta.flags(), 0);
}

#[test]
fn counted_payload_validation_rejects_truncated_sbgp_entries() {
    let payload = [
        0x00, 0x00, 0x00, 0x00, 0x01, 0x23, 0x45, 0x67, 0x00, 0x00, 0x00, 0x02, 0x23, 0x45, 0x67,
        0x89, 0x34, 0x56, 0x78, 0x9a,
    ];
    let mut decoded = Sbgp::default();
    let mut reader = Cursor::new(payload);
    let error = unmarshal(&mut reader, payload.len() as u64, &mut decoded, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid field value for Entries: entry payload length does not match the entry count"
    );
}

#[test]
fn built_in_registry_reports_supported_versions_for_landed_types() {
    let registry = default_registry();

    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"mvhd")),
        Some(&[0, 1][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"tfhd")),
        Some(&[0][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"meta")),
        Some(&[0][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"saio")),
        Some(&[0, 1][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"sgpd")),
        Some(&[1, 2][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"tfra")),
        Some(&[0, 1][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"emsg")),
        Some(&[0, 1][..])
    );
    assert!(registry.is_registered(FourCc::from_bytes(*b"ftyp")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"avcC")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"btrt")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"colr")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"hdlr")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"hvcC")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"avc1")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"mp4a")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"pasp")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"schm")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"sbtt")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"sidx")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"stpp")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"trun")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"wave")));
}

#[test]
fn fixed_point_and_version_helpers_match_expected_values() {
    let mut mvhd = Mvhd::default();
    mvhd.set_version(1);
    mvhd.creation_time_v1 = u64::MAX;
    mvhd.modification_time_v1 = u64::MAX - 1;
    mvhd.duration_v1 = u64::MAX - 2;
    mvhd.rate = 0x04d2b000;
    assert_eq!(mvhd.creation_time(), u64::MAX);
    assert_eq!(mvhd.modification_time(), u64::MAX - 1);
    assert_eq!(mvhd.duration(), u64::MAX - 2);
    assert_eq!(mvhd.rate_value(), 1234.6875);
    assert_eq!(mvhd.rate_int(), 1234);
    let mut whole_rate_mvhd = Mvhd::default();
    whole_rate_mvhd.rate = 1 << 16;
    let rendered = stringify(&whole_rate_mvhd, None).unwrap();
    assert!(rendered.contains(" Rate=1 "));
    assert!(!rendered.contains("Rate=1.00000"));

    let mut smhd = Smhd::default();
    smhd.balance = 0x3420;
    assert_eq!(smhd.balance_value(), 52.125);
    assert_eq!(smhd.balance_int(), 52);

    let mut tkhd = Tkhd::default();
    tkhd.width = 0x205800;
    tkhd.height = 0x05ec2c00;
    assert_eq!(tkhd.width_value(), 32.34375);
    assert_eq!(tkhd.width_int(), 32);
    assert_eq!(tkhd.height_value(), 1516.171875);
    assert_eq!(tkhd.height_int(), 1516);

    let mut mehd = Mehd::default();
    mehd.set_version(1);
    mehd.fragment_duration_v1 = u64::MAX;
    assert_eq!(mehd.fragment_duration(), u64::MAX);

    let mut mdhd = Mdhd::default();
    mdhd.set_version(1);
    mdhd.creation_time_v1 = u64::MAX;
    mdhd.modification_time_v1 = u64::MAX - 1;
    mdhd.duration_v1 = u64::MAX - 2;
    assert_eq!(mdhd.creation_time(), u64::MAX);
    assert_eq!(mdhd.modification_time(), u64::MAX - 1);
    assert_eq!(mdhd.duration(), u64::MAX - 2);

    let mut saio = Saio::default();
    saio.set_version(0);
    saio.offset_v0 = vec![u64::from(u32::MAX), u64::from(u32::MAX - 1)];
    assert_eq!(saio.offset(0), u64::from(u32::MAX));
    assert_eq!(saio.offset(1), u64::from(u32::MAX - 1));
    saio.set_version(1);
    saio.offset_v1 = vec![u64::MAX, u64::MAX - 1];
    assert_eq!(saio.offset(0), u64::MAX);
    assert_eq!(saio.offset(1), u64::MAX - 1);

    let mut sidx = Sidx::default();
    sidx.set_version(0);
    sidx.earliest_presentation_time_v0 = u32::MAX;
    sidx.first_offset_v0 = u32::MAX - 1;
    assert_eq!(sidx.earliest_presentation_time(), u64::from(u32::MAX));
    assert_eq!(sidx.first_offset(), u64::from(u32::MAX - 1));
    sidx.set_version(1);
    sidx.earliest_presentation_time_v1 = u64::MAX;
    sidx.first_offset_v1 = u64::MAX - 1;
    assert_eq!(sidx.earliest_presentation_time(), u64::MAX);
    assert_eq!(sidx.first_offset(), u64::MAX - 1);

    let mut tfra = Tfra::default();
    tfra.set_version(0);
    tfra.entries = vec![
        TfraEntry {
            time_v0: u32::MAX,
            moof_offset_v0: u32::MAX - 1,
            ..TfraEntry::default()
        },
        TfraEntry {
            time_v0: u32::MAX - 2,
            moof_offset_v0: u32::MAX - 3,
            ..TfraEntry::default()
        },
    ];
    assert_eq!(tfra.time(0), u64::from(u32::MAX));
    assert_eq!(tfra.moof_offset(0), u64::from(u32::MAX - 1));
    assert_eq!(tfra.time(1), u64::from(u32::MAX - 2));
    assert_eq!(tfra.moof_offset(1), u64::from(u32::MAX - 3));
    tfra.set_version(1);
    tfra.entries = vec![
        TfraEntry {
            time_v1: u64::MAX,
            moof_offset_v1: u64::MAX - 1,
            ..TfraEntry::default()
        },
        TfraEntry {
            time_v1: u64::MAX - 2,
            moof_offset_v1: u64::MAX - 3,
            ..TfraEntry::default()
        },
    ];
    assert_eq!(tfra.time(0), u64::MAX);
    assert_eq!(tfra.moof_offset(0), u64::MAX - 1);
    assert_eq!(tfra.time(1), u64::MAX - 2);
    assert_eq!(tfra.moof_offset(1), u64::MAX - 3);

    let audio = AudioSampleEntry {
        sample_rate: 0x205800,
        ..AudioSampleEntry::default()
    };
    assert_eq!(audio.sample_rate_value(), 32.34375);
    assert_eq!(audio.sample_rate_int(), 32);

    let stpp = XMLSubtitleSampleEntry {
        namespace: String::from("http://foo.org/bar http://baz.org/qux"),
        schema_location: String::from("http://quux.org/corge grault"),
        auxiliary_mime_types: String::from("application/ttml+xml text/xml"),
        ..XMLSubtitleSampleEntry::default()
    };
    assert_eq!(
        stpp.namespace_list(),
        vec!["http://foo.org/bar", "http://baz.org/qux"]
    );
    assert_eq!(
        stpp.schema_location_list(),
        vec!["http://quux.org/corge", "grault"]
    );
    assert_eq!(
        stpp.auxiliary_mime_types_list(),
        vec!["application/ttml+xml", "text/xml"]
    );
}

#[test]
fn avcc_rejects_inconsistent_high_profile_state() {
    let invalid = AVCDecoderConfiguration {
        configuration_version: 0x12,
        profile: 0x4d,
        profile_compatibility: 0x40,
        level: 0x1f,
        length_size_minus_one: 0x02,
        num_of_sequence_parameter_sets: 1,
        sequence_parameter_sets: vec![AVCParameterSet {
            length: 2,
            nal_unit: vec![0x12, 0x34],
        }],
        num_of_picture_parameter_sets: 1,
        picture_parameter_sets: vec![AVCParameterSet {
            length: 2,
            nal_unit: vec![0xab, 0xcd],
        }],
        high_profile_fields_enabled: true,
        chroma_format: 0x01,
        bit_depth_luma_minus8: 0x02,
        bit_depth_chroma_minus8: 0x03,
        num_of_sequence_parameter_set_ext: 0,
        sequence_parameter_sets_ext: Vec::new(),
    };

    let error = marshal(&mut Vec::new(), &invalid, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid field value for HighProfileFieldsEnabled: each values of Profile and HighProfileFieldsEnabled are inconsistent"
    );
}

#[test]
fn hvcc_rejects_truncated_nalu_array_payloads() {
    let payload = [
        0x01, 0x01, 0x60, 0x00, 0x00, 0x00, 0x90, 0x00, 0x00, 0x00, 0x00, 0x00, 0x78, 0xe0, 0x00,
        0xfc, 0xfd, 0xf8, 0xf8, 0x00, 0x00, 0x0f, 0x04, 0x20, 0x00, 0x01, 0x00, 0x18, 0x40, 0x01,
        0x0c, 0x01, 0xff, 0xff, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x90, 0x00, 0x00, 0x03, 0x00,
        0x00, 0x03, 0x00, 0x78, 0x99, 0x98, 0x09, 0x21, 0x00, 0x01, 0x00, 0x2a, 0x06, 0x01, 0x01,
        0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x90, 0x00, 0x00, 0x03, 0x00, 0x00, 0x03, 0x00, 0x78,
        0xa0, 0x03, 0xc0, 0x80, 0x10, 0xe5, 0x96, 0x66, 0x69, 0x24, 0xca, 0xe0, 0x10, 0x00, 0x00,
        0x03, 0x00, 0x10, 0x00, 0x00, 0x03, 0x01, 0xe0, 0x80, 0x22, 0x00, 0x01, 0x00, 0x07, 0x44,
        0x01, 0xc1, 0x72, 0xb4, 0x62, 0x40, 0x27, 0x00, 0x01, 0x00, 0x0b, 0x4e, 0x01, 0x05, 0xff,
        0xff, 0xff, 0xa6, 0x2c, 0xa2, 0xde,
    ];

    let mut decoded = HEVCDecoderConfiguration::default();
    let mut reader = Cursor::new(payload);
    let error = unmarshal(&mut reader, payload.len() as u64, &mut decoded, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid field value for NaluArrays: NAL-array payload length does not match the entry count"
    );
}

#[test]
fn ftyp_compatible_brand_helpers_preserve_uniqueness() {
    let mut ftyp = Ftyp::default();

    let mp41 = FourCc::from_bytes(*b"mp41");
    let avc1 = FourCc::from_bytes(*b"avc1");
    let iso5 = FourCc::from_bytes(*b"iso5");

    ftyp.add_compatible_brand(mp41);
    ftyp.add_compatible_brand(avc1);
    ftyp.add_compatible_brand(iso5);
    ftyp.add_compatible_brand(iso5);

    assert_eq!(ftyp.compatible_brands.len(), 3);
    assert!(ftyp.has_compatible_brand(mp41));
    assert!(ftyp.has_compatible_brand(avc1));
    assert!(ftyp.has_compatible_brand(iso5));

    ftyp.remove_compatible_brand(mp41);
    assert!(!ftyp.has_compatible_brand(mp41));
    assert_eq!(ftyp.compatible_brands.len(), 2);
}
