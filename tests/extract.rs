use std::io::Cursor;

use mp4forge::boxes::AnyTypeBox;
use mp4forge::boxes::iso14496_12::{
    Cdsc, Elng, Emeb, Emib, EventMessageSampleEntry, Ftyp, Leva, LevaLevel, Mdia, Meta, Minf, Moov,
    Mvex, Saio, Saiz, Sbgp, Sgpd, Silb, Ssix, SsixRange, SsixSubsegment, Stbl, Subs, SubsEntry,
    SubsSample, Tkhd, Trak, Tref, Trep, Udta,
};
use mp4forge::boxes::iso23001_7::{Senc, Tenc};
use mp4forge::boxes::metadata::{
    DATA_TYPE_STRING_UTF8, Data, Ilst, Key, Keys, NumberedMetadataItem,
};
use mp4forge::codec::{CodecBox, marshal};
use mp4forge::extract::{
    ExtractError, extract_box, extract_box_as, extract_box_as_bytes, extract_box_bytes,
    extract_box_payload_bytes, extract_box_with_payload, extract_boxes, extract_boxes_as_bytes,
    extract_boxes_bytes, extract_boxes_payload_bytes,
};
use mp4forge::stringify::stringify;
use mp4forge::walk::BoxPath;
use mp4forge::{BoxInfo, FourCc};

mod support;

use support::{
    build_encrypted_fragmented_video_file, build_event_message_movie_file, fixture_path,
};

#[test]
fn extract_boxes_match_exact_wildcard_and_relative_paths() {
    let trak = encode_supported_box(&Trak, &[]);
    let meta = encode_supported_box(&Meta::default(), &[]);
    let udta = encode_supported_box(&Udta, &meta);
    let moov = encode_supported_box(&Moov, &[trak, udta].concat());

    let wildcard = extract_box(
        &mut Cursor::new(moov.clone()),
        None,
        BoxPath::from([fourcc("moov"), FourCc::ANY]),
    )
    .unwrap();
    assert_eq!(box_types(&wildcard), vec![fourcc("trak"), fourcc("udta")]);

    let exact = extract_boxes(
        &mut Cursor::new(moov.clone()),
        None,
        &[
            BoxPath::from([fourcc("moov")]),
            BoxPath::from([fourcc("moov"), fourcc("udta")]),
        ],
    )
    .unwrap();
    assert_eq!(box_types(&exact), vec![fourcc("moov"), fourcc("udta")]);

    let parent = extract_box(
        &mut Cursor::new(moov.clone()),
        None,
        BoxPath::from([fourcc("moov")]),
    )
    .unwrap()
    .pop()
    .unwrap();
    let relative = extract_box(
        &mut Cursor::new(moov),
        Some(&parent),
        BoxPath::from([fourcc("udta")]),
    )
    .unwrap();
    assert_eq!(box_types(&relative), vec![fourcc("udta")]);
}

#[test]
fn extract_box_with_payload_uses_walked_lookup_context() {
    let qt = fourcc("qt  ");
    let ftyp = Ftyp {
        major_brand: qt,
        minor_version: 0x0200,
        compatible_brands: vec![qt],
    };
    let mut keys = Keys::default();
    keys.entry_count = 1;
    keys.entries = vec![Key {
        key_size: 9,
        key_namespace: fourcc("mdta"),
        key_value: vec![b'x'],
    }];

    let mut numbered = NumberedMetadataItem::default();
    numbered.set_box_type(FourCc::from_u32(1));
    numbered.item_name = fourcc("data");
    numbered.data = Data {
        data_type: DATA_TYPE_STRING_UTF8,
        data_lang: 0,
        data: b"1.0.0".to_vec(),
    };

    let keys_box = encode_supported_box(&keys, &[]);
    let numbered_box = encode_supported_box(&numbered, &[]);
    let ilst_box = encode_supported_box(&Ilst, &numbered_box);
    let meta_box = encode_supported_box(&Meta::default(), &[keys_box, ilst_box].concat());
    let moov_box = encode_supported_box(&Moov, &meta_box);
    let file = [encode_supported_box(&ftyp, &[]), moov_box].concat();

    let extracted = extract_box_with_payload(
        &mut Cursor::new(file),
        None,
        BoxPath::from([
            fourcc("moov"),
            fourcc("meta"),
            fourcc("ilst"),
            FourCc::from_u32(1),
        ]),
    )
    .unwrap();

    assert_eq!(extracted.len(), 1);
    let extracted = &extracted[0];
    assert_eq!(extracted.info.box_type(), FourCc::from_u32(1));
    assert!(extracted.info.lookup_context().under_ilst());
    assert_eq!(
        extracted.info.lookup_context().metadata_keys_entry_count(),
        1
    );

    let numbered = extracted
        .payload
        .as_ref()
        .as_any()
        .downcast_ref::<NumberedMetadataItem>()
        .unwrap();
    assert_eq!(numbered.item_name, fourcc("data"));
    assert_eq!(numbered.data.data_type, DATA_TYPE_STRING_UTF8);
    assert_eq!(numbered.data.data, b"1.0.0");
}

#[test]
fn extract_box_as_returns_typed_payloads() {
    let mut tkhd_a = Tkhd::default();
    tkhd_a.track_id = 1;
    let mut tkhd_b = Tkhd::default();
    tkhd_b.track_id = 2;
    let trak_a = encode_supported_box(&Trak, &encode_supported_box(&tkhd_a, &[]));
    let trak_b = encode_supported_box(&Trak, &encode_supported_box(&tkhd_b, &[]));
    let moov = encode_supported_box(&Moov, &[trak_a, trak_b].concat());

    let extracted = extract_box_as::<_, Tkhd>(
        &mut Cursor::new(moov),
        None,
        BoxPath::from([fourcc("moov"), fourcc("trak"), fourcc("tkhd")]),
    )
    .unwrap();

    assert_eq!(extracted.len(), 2);
    assert_eq!(
        extracted
            .iter()
            .map(|tkhd| tkhd.track_id)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn extract_box_bytes_preserve_exact_leaf_box_bytes_for_relative_paths() {
    let leaf = encode_raw_box(fourcc("zzzz"), &[0xde, 0xad, 0xbe, 0xef]);
    let udta = encode_supported_box(&Udta, &leaf);
    let moov = encode_supported_box(&Moov, &udta);

    let parent = extract_box(
        &mut Cursor::new(moov.clone()),
        None,
        BoxPath::from([fourcc("moov")]),
    )
    .unwrap()
    .pop()
    .unwrap();

    let extracted = extract_box_bytes(
        &mut Cursor::new(moov),
        Some(&parent),
        BoxPath::from([fourcc("udta"), fourcc("zzzz")]),
    )
    .unwrap();

    assert_eq!(extracted, vec![leaf]);
}

#[test]
fn extract_box_payload_bytes_preserve_exact_container_payload_bytes() {
    let leaf = encode_raw_box(fourcc("zzzz"), &[0xde, 0xad, 0xbe, 0xef]);
    let udta = encode_supported_box(&Udta, &leaf);
    let moov = encode_supported_box(&Moov, &udta);

    let extracted = extract_box_payload_bytes(
        &mut Cursor::new(moov),
        None,
        BoxPath::from([fourcc("moov"), fourcc("udta")]),
    )
    .unwrap();

    assert_eq!(extracted, vec![leaf]);
}

#[test]
fn extract_box_as_decodes_known_tref_children_and_preserves_unknown_ones_as_raw_bytes() {
    let cdsc = encode_supported_box(
        &Cdsc {
            track_ids: vec![9, 11],
        },
        &[],
    );
    let unknown = encode_raw_box(fourcc("zzzz"), &[0xaa, 0xbb, 0xcc, 0xdd]);
    let tref = encode_supported_box(&Tref, &[cdsc.clone(), unknown.clone()].concat());
    let trak = encode_supported_box(&Trak, &tref);
    let moov = encode_supported_box(&Moov, &trak);

    let extracted_cdsc = extract_box_as::<_, Cdsc>(
        &mut Cursor::new(moov.clone()),
        None,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("tref"),
            fourcc("cdsc"),
        ]),
    )
    .unwrap();
    assert_eq!(extracted_cdsc.len(), 1);
    assert_eq!(extracted_cdsc[0].track_ids, vec![9, 11]);

    let extracted_unknown = extract_box_bytes(
        &mut Cursor::new(moov),
        None,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("tref"),
            fourcc("zzzz"),
        ]),
    )
    .unwrap();
    assert_eq!(extracted_unknown, vec![unknown]);
}

#[test]
fn extract_box_as_bytes_returns_typed_payloads_without_cursor() {
    let mut tkhd_a = Tkhd::default();
    tkhd_a.track_id = 1;
    let mut tkhd_b = Tkhd::default();
    tkhd_b.track_id = 2;
    let trak_a = encode_supported_box(&Trak, &encode_supported_box(&tkhd_a, &[]));
    let trak_b = encode_supported_box(&Trak, &encode_supported_box(&tkhd_b, &[]));
    let moov = encode_supported_box(&Moov, &[trak_a, trak_b].concat());

    let extracted = extract_box_as_bytes::<Tkhd>(
        &moov,
        BoxPath::from([fourcc("moov"), fourcc("trak"), fourcc("tkhd")]),
    )
    .unwrap();

    assert_eq!(extracted.len(), 2);
    assert_eq!(
        extracted
            .iter()
            .map(|tkhd| tkhd.track_id)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn extract_box_as_decodes_fragmented_encrypted_metadata_boxes() {
    let file = build_encrypted_fragmented_video_file();

    let tenc = extract_box_as::<_, Tenc>(
        &mut Cursor::new(file.clone()),
        None,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("encv"),
            fourcc("sinf"),
            fourcc("schi"),
            fourcc("tenc"),
        ]),
    )
    .unwrap();
    assert_eq!(tenc.len(), 1);
    assert_eq!(tenc[0].default_is_protected, 1);
    assert_eq!(tenc[0].default_per_sample_iv_size, 8);
    assert_eq!(
        tenc[0].default_kid,
        [
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x10, 0x32, 0x54, 0x76, 0x98, 0xba,
            0xdc, 0xfe,
        ]
    );

    let saiz = extract_box_as::<_, Saiz>(
        &mut Cursor::new(file.clone()),
        None,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("saiz")]),
    )
    .unwrap();
    assert_eq!(saiz.len(), 1);
    assert_eq!(saiz[0].sample_count, 1);
    assert_eq!(saiz[0].sample_info_size, vec![16]);

    let saio = extract_box_as::<_, Saio>(
        &mut Cursor::new(file.clone()),
        None,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("saio")]),
    )
    .unwrap();
    assert_eq!(saio.len(), 1);
    assert_eq!(saio[0].entry_count, 1);
    assert_eq!(saio[0].offset(0), 0);

    let senc = extract_box_as::<_, Senc>(
        &mut Cursor::new(file.clone()),
        None,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("senc")]),
    )
    .unwrap();
    assert_eq!(senc.len(), 1);
    assert!(senc[0].uses_subsample_encryption());
    assert_eq!(senc[0].sample_count, 1);
    assert_eq!(
        senc[0].samples[0].initialization_vector,
        vec![1, 2, 3, 4, 5, 6, 7, 8]
    );
    assert_eq!(senc[0].samples[0].subsamples.len(), 1);
    assert_eq!(senc[0].samples[0].subsamples[0].bytes_of_clear_data, 32);
    assert_eq!(
        senc[0].samples[0].subsamples[0].bytes_of_protected_data,
        480
    );

    let sgpd = extract_box_as::<_, Sgpd>(
        &mut Cursor::new(file.clone()),
        None,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("sgpd")]),
    )
    .unwrap();
    assert_eq!(sgpd.len(), 1);
    assert_eq!(sgpd[0].grouping_type, fourcc("seig"));
    assert_eq!(sgpd[0].seig_entries_l.len(), 1);
    assert_eq!(sgpd[0].seig_entries_l[0].description_length, 20);
    assert_eq!(sgpd[0].seig_entries_l[0].seig_entry.per_sample_iv_size, 8);
    assert_eq!(sgpd[0].seig_entries_l[0].seig_entry.crypt_byte_block, 1);
    assert_eq!(sgpd[0].seig_entries_l[0].seig_entry.skip_byte_block, 9);

    let sbgp = extract_box_as::<_, Sbgp>(
        &mut Cursor::new(file),
        None,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("sbgp")]),
    )
    .unwrap();
    assert_eq!(sbgp.len(), 1);
    assert_eq!(sbgp[0].grouping_type, u32::from_be_bytes(*b"seig"));
    assert_eq!(sbgp[0].entries.len(), 1);
    assert_eq!(sbgp[0].entries[0].sample_count, 1);
    assert_eq!(sbgp[0].entries[0].group_description_index, 65_537);
}

#[test]
fn extract_box_as_decodes_compact_metadata_boxes() {
    let mut elng = Elng::default();
    elng.extended_language = "en-US".into();
    let elng = encode_supported_box(&elng, &[]);

    let mut subs = Subs::default();
    subs.entry_count = 1;
    subs.entries = vec![SubsEntry {
        sample_delta: 7,
        subsample_count: 1,
        subsamples: vec![SubsSample {
            subsample_size: 11,
            subsample_priority: 2,
            discardable: 0,
            codec_specific_parameters: 0x01020304,
        }],
    }];
    let subs = encode_supported_box(&subs, &[]);

    let stbl = encode_supported_box(&Stbl, &subs);
    let minf = encode_supported_box(&Minf, &stbl);
    let mdia = encode_supported_box(&Mdia, &[elng, minf].concat());
    let trak = encode_supported_box(&Trak, &mdia);

    let mut leva = Leva::default();
    leva.level_count = 1;
    leva.levels = vec![LevaLevel {
        track_id: 9,
        assignment_type: 4,
        sub_track_id: 11,
        ..LevaLevel::default()
    }];
    let mut trep = Trep::default();
    trep.track_id = 9;
    let trep = encode_supported_box(&trep, &encode_supported_box(&leva, &[]));
    let mvex = encode_supported_box(&Mvex, &trep);

    let mut ssix = Ssix::default();
    ssix.subsegment_count = 1;
    ssix.subsegments = vec![SsixSubsegment {
        range_count: 1,
        ranges: vec![SsixRange {
            level: 3,
            range_size: 0x44,
        }],
    }];
    let ssix = encode_supported_box(&ssix, &[]);

    let moov = encode_supported_box(&Moov, &[trak, mvex].concat());
    let file = [moov, ssix].concat();

    let extracted_elng = extract_box_as::<_, Elng>(
        &mut Cursor::new(file.clone()),
        None,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("elng"),
        ]),
    )
    .unwrap();
    assert_eq!(extracted_elng.len(), 1);
    assert_eq!(extracted_elng[0].extended_language, "en-US");

    let extracted_subs = extract_box_as::<_, Subs>(
        &mut Cursor::new(file.clone()),
        None,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("subs"),
        ]),
    )
    .unwrap();
    assert_eq!(extracted_subs.len(), 1);
    assert_eq!(extracted_subs[0].entries[0].sample_delta, 7);
    assert_eq!(
        extracted_subs[0].entries[0].subsamples[0].codec_specific_parameters,
        0x01020304
    );

    let extracted_leva = extract_box_as::<_, Leva>(
        &mut Cursor::new(file.clone()),
        None,
        BoxPath::from([
            fourcc("moov"),
            fourcc("mvex"),
            fourcc("trep"),
            fourcc("leva"),
        ]),
    )
    .unwrap();
    assert_eq!(extracted_leva.len(), 1);
    assert_eq!(extracted_leva[0].levels[0].track_id, 9);
    assert_eq!(extracted_leva[0].levels[0].sub_track_id, 11);

    let extracted_ssix = extract_box_as::<_, Ssix>(
        &mut Cursor::new(file),
        None,
        BoxPath::from([fourcc("ssix")]),
    )
    .unwrap();
    assert_eq!(extracted_ssix.len(), 1);
    assert_eq!(extracted_ssix[0].subsegments[0].ranges[0].level, 3);
    assert_eq!(extracted_ssix[0].subsegments[0].ranges[0].range_size, 0x44);
}

#[test]
fn extract_box_as_decodes_event_message_boxes() {
    let file = build_event_message_movie_file();

    let evte = extract_box_as::<_, EventMessageSampleEntry>(
        &mut Cursor::new(file.clone()),
        None,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("evte"),
        ]),
    )
    .unwrap();
    assert_eq!(evte.len(), 1);
    assert_eq!(evte[0].sample_entry.data_reference_index, 1);

    let silb = extract_box_as::<_, Silb>(
        &mut Cursor::new(file.clone()),
        None,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("evte"),
            fourcc("silb"),
        ]),
    )
    .unwrap();
    assert_eq!(silb.len(), 1);
    assert_eq!(silb[0].scheme_count, 2);
    assert_eq!(silb[0].schemes[0].scheme_id_uri, "urn:mpeg:dash:event:2012");
    assert_eq!(silb[0].schemes[1].value, "splice");
    assert!(silb[0].schemes[1].at_least_one_flag);
    assert!(silb[0].other_schemes_flag);

    let emib = extract_box_as::<_, Emib>(
        &mut Cursor::new(file.clone()),
        None,
        BoxPath::from([fourcc("emib")]),
    )
    .unwrap();
    assert_eq!(emib.len(), 1);
    assert_eq!(emib[0].presentation_time_delta, -1_000);
    assert_eq!(emib[0].event_duration, 2_000);
    assert_eq!(emib[0].scheme_id_uri, "urn:scte:scte35:2013:bin");
    assert_eq!(emib[0].message_data, vec![0x01, 0x02, 0x03]);

    let emeb = extract_box_as::<_, Emeb>(
        &mut Cursor::new(file),
        None,
        BoxPath::from([fourcc("emeb")]),
    )
    .unwrap();
    assert_eq!(emeb.len(), 1);
}

#[test]
fn extract_boxes_bytes_match_shared_fixture_box_ranges() {
    let sample = std::fs::read(fixture_path("sample.mp4")).unwrap();
    let paths = [
        BoxPath::from([fourcc("ftyp")]),
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("mdhd"),
        ]),
    ];

    let infos = extract_boxes(&mut Cursor::new(sample.clone()), None, &paths).unwrap();
    let extracted = extract_boxes_bytes(&mut Cursor::new(sample.clone()), None, &paths).unwrap();

    assert_eq!(infos.len(), extracted.len());
    for (info, bytes) in infos.iter().zip(extracted.iter()) {
        assert_eq!(bytes, &box_bytes_from_file(&sample, info));
    }
}

#[test]
fn extract_boxes_payload_bytes_match_shared_fixture_payload_ranges() {
    let sample = std::fs::read(fixture_path("sample.mp4")).unwrap();
    let paths = [BoxPath::from([
        fourcc("moov"),
        fourcc("trak"),
        fourcc("mdia"),
        fourcc("mdhd"),
    ])];

    let infos = extract_boxes(&mut Cursor::new(sample.clone()), None, &paths).unwrap();
    let extracted =
        extract_boxes_payload_bytes(&mut Cursor::new(sample.clone()), None, &paths).unwrap();

    assert_eq!(infos.len(), extracted.len());
    for (info, bytes) in infos.iter().zip(extracted.iter()) {
        assert_eq!(bytes, &payload_bytes_from_file(&sample, info));
    }
}

#[test]
fn extract_boxes_as_bytes_supports_multiple_root_paths() {
    let mut root_tkhd = Tkhd::default();
    root_tkhd.track_id = 1;
    let root_trak = encode_supported_box(&Trak, &encode_supported_box(&root_tkhd, &[]));

    let mut nested_tkhd = Tkhd::default();
    nested_tkhd.track_id = 2;
    let nested_trak = encode_supported_box(&Trak, &encode_supported_box(&nested_tkhd, &[]));
    let moov = encode_supported_box(&Moov, &nested_trak);

    let file = [root_trak, moov].concat();
    let extracted = extract_boxes_as_bytes::<Tkhd>(
        &file,
        &[
            BoxPath::from([fourcc("trak"), fourcc("tkhd")]),
            BoxPath::from([fourcc("moov"), fourcc("trak"), fourcc("tkhd")]),
        ],
    )
    .unwrap();

    assert_eq!(
        extracted
            .iter()
            .map(|tkhd| tkhd.track_id)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn extract_box_payload_bytes_return_empty_when_nothing_matches() {
    let moov = encode_supported_box(&Moov, &[]);
    let extracted = extract_box_payload_bytes(
        &mut Cursor::new(moov),
        None,
        BoxPath::from([fourcc("zzzz")]),
    )
    .unwrap();

    assert!(extracted.is_empty());
}

#[test]
fn extract_box_as_uses_walked_lookup_context() {
    let qt = fourcc("qt  ");
    let ftyp = Ftyp {
        major_brand: qt,
        minor_version: 0x0200,
        compatible_brands: vec![qt],
    };
    let mut keys = Keys::default();
    keys.entry_count = 1;
    keys.entries = vec![Key {
        key_size: 9,
        key_namespace: fourcc("mdta"),
        key_value: vec![b'x'],
    }];

    let mut numbered = NumberedMetadataItem::default();
    numbered.set_box_type(FourCc::from_u32(1));
    numbered.item_name = fourcc("data");
    numbered.data = Data {
        data_type: DATA_TYPE_STRING_UTF8,
        data_lang: 0,
        data: b"1.0.0".to_vec(),
    };

    let keys_box = encode_supported_box(&keys, &[]);
    let numbered_box = encode_supported_box(&numbered, &[]);
    let ilst_box = encode_supported_box(&Ilst, &numbered_box);
    let meta_box = encode_supported_box(&Meta::default(), &[keys_box, ilst_box].concat());
    let moov_box = encode_supported_box(&Moov, &meta_box);
    let file = [encode_supported_box(&ftyp, &[]), moov_box].concat();

    let extracted = extract_box_as::<_, NumberedMetadataItem>(
        &mut Cursor::new(file),
        None,
        BoxPath::from([
            fourcc("moov"),
            fourcc("meta"),
            fourcc("ilst"),
            FourCc::from_u32(1),
        ]),
    )
    .unwrap();

    assert_eq!(extracted.len(), 1);
    assert_eq!(extracted[0].item_name, fourcc("data"));
    assert_eq!(extracted[0].data.data, b"1.0.0");
}

#[test]
fn extract_box_as_bytes_reports_payload_type_context() {
    let mut tkhd = Tkhd::default();
    tkhd.track_id = 7;
    let trak = encode_supported_box(&Trak, &encode_supported_box(&tkhd, &[]));
    let moov = encode_supported_box(&Moov, &trak);

    let error = extract_box_as_bytes::<Meta>(
        &moov,
        BoxPath::from([fourcc("moov"), fourcc("trak"), fourcc("tkhd")]),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        ExtractError::UnexpectedPayloadType {
            ref path,
            box_type,
            offset,
            expected_type
        } if path.as_slice() == [fourcc("moov"), fourcc("trak"), fourcc("tkhd")]
            && box_type == fourcc("tkhd")
            && offset == 16
            && expected_type == std::any::type_name::<Meta>()
    ));
}

#[test]
fn extract_box_as_reports_payload_type_context() {
    let mut tkhd = Tkhd::default();
    tkhd.track_id = 7;
    let trak = encode_supported_box(&Trak, &encode_supported_box(&tkhd, &[]));
    let moov = encode_supported_box(&Moov, &trak);

    let error = extract_box_as::<_, Meta>(
        &mut Cursor::new(moov),
        None,
        BoxPath::from([fourcc("moov"), fourcc("trak"), fourcc("tkhd")]),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        ExtractError::UnexpectedPayloadType {
            ref path,
            box_type,
            offset,
            expected_type
        } if path.as_slice() == [fourcc("moov"), fourcc("trak"), fourcc("tkhd")]
            && box_type == fourcc("tkhd")
            && offset == 16
            && expected_type == std::any::type_name::<Meta>()
    ));
    assert_eq!(
        error.to_string(),
        format!(
            "unexpected decoded payload type at moov/trak/tkhd (type=tkhd, offset=16): expected {}",
            std::any::type_name::<Meta>()
        )
    );
}

#[test]
fn extract_box_rejects_empty_paths() {
    let error =
        extract_box(&mut Cursor::new(Vec::<u8>::new()), None, BoxPath::default()).unwrap_err();
    assert!(matches!(error, ExtractError::EmptyPath));
}

#[test]
fn extract_boxes_match_shared_fixture_expected_paths() {
    let sample = std::fs::read(fixture_path("sample.mp4")).unwrap();
    let ftyp = extract_box(
        &mut Cursor::new(sample.clone()),
        None,
        BoxPath::from([fourcc("ftyp")]),
    )
    .unwrap();
    assert_eq!(box_types(&ftyp), vec![fourcc("ftyp")]);
    assert_eq!(ftyp[0].size(), 32);

    let mdhd = extract_box(
        &mut Cursor::new(sample),
        None,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("mdhd"),
        ]),
    )
    .unwrap();
    assert_eq!(box_types(&mdhd), vec![fourcc("mdhd"), fourcc("mdhd")]);
    assert_eq!(mdhd.iter().map(BoxInfo::size).sum::<u64>(), 64);

    let fragmented = std::fs::read(fixture_path("sample_fragmented.mp4")).unwrap();
    let trun = extract_box(
        &mut Cursor::new(fragmented),
        None,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("trun")]),
    )
    .unwrap();
    assert_eq!(trun.len(), 8);
    assert!(trun.iter().all(|info| info.box_type() == fourcc("trun")));
    assert_eq!(trun.iter().map(BoxInfo::size).sum::<u64>(), 452);
}

#[test]
fn extract_box_with_payload_normalizes_nested_quicktime_numbered_items() {
    let sample = std::fs::read(fixture_path("sample_qt.mp4")).unwrap();
    let extracted = extract_box_with_payload(
        &mut Cursor::new(sample),
        None,
        BoxPath::from([
            fourcc("moov"),
            fourcc("udta"),
            fourcc("meta"),
            fourcc("ilst"),
            FourCc::from_u32(1),
        ]),
    )
    .unwrap();

    assert_eq!(extracted.len(), 1);
    let numbered = extracted[0]
        .payload
        .as_ref()
        .as_any()
        .downcast_ref::<NumberedMetadataItem>()
        .unwrap();

    assert_eq!(
        stringify(numbered, None).unwrap(),
        "Version=0 Flags=0x000000 ItemName=\"data\" Data={DataType=UTF8 DataLang=0 Data=\"1.0.0\"}"
    );
}

fn box_types(boxes: &[BoxInfo]) -> Vec<FourCc> {
    boxes.iter().map(BoxInfo::box_type).collect()
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

fn box_bytes_from_file(file: &[u8], info: &BoxInfo) -> Vec<u8> {
    let start = usize::try_from(info.offset()).unwrap();
    let end = usize::try_from(info.offset() + info.size()).unwrap();
    file[start..end].to_vec()
}

fn payload_bytes_from_file(file: &[u8], info: &BoxInfo) -> Vec<u8> {
    let start = usize::try_from(info.offset() + info.header_size()).unwrap();
    let end = usize::try_from(info.offset() + info.size()).unwrap();
    file[start..end].to_vec()
}
