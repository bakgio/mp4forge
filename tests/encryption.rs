use std::io::Cursor;

use mp4forge::boxes::iso14496_12::{Saiz, Sbgp, SbgpEntry, SeigEntry, Sgpd};
use mp4forge::boxes::iso23001_7::{
    SENC_USE_SUBSAMPLE_ENCRYPTION, Senc, SencSample, SencSubsample, Tenc,
};
use mp4forge::codec::MutableBox;
use mp4forge::encryption::{
    ResolveSampleEncryptionError, ResolvedSampleEncryptionSource, SampleEncryptionContext,
    resolve_sample_encryption,
};
use mp4forge::extract::extract_box_as;
use mp4forge::walk::BoxPath;

mod support;

use support::{build_encrypted_fragmented_video_file, fourcc};

#[test]
fn resolve_sample_encryption_uses_fragment_local_seig_from_extracted_boxes() {
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
    let saiz = extract_box_as::<_, Saiz>(
        &mut Cursor::new(file.clone()),
        None,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("saiz")]),
    )
    .unwrap();
    let senc = extract_box_as::<_, Senc>(
        &mut Cursor::new(file.clone()),
        None,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("senc")]),
    )
    .unwrap();
    let sgpd = extract_box_as::<_, Sgpd>(
        &mut Cursor::new(file.clone()),
        None,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("sgpd")]),
    )
    .unwrap();
    let sbgp = extract_box_as::<_, Sbgp>(
        &mut Cursor::new(file),
        None,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("sbgp")]),
    )
    .unwrap();

    let resolved = resolve_sample_encryption(
        &senc[0],
        SampleEncryptionContext {
            tenc: Some(&tenc[0]),
            sgpd: Some(&sgpd[0]),
            sbgp: Some(&sbgp[0]),
            saiz: Some(&saiz[0]),
        },
    )
    .unwrap();

    assert!(resolved.uses_subsample_encryption);
    assert_eq!(resolved.samples.len(), 1);

    let sample = &resolved.samples[0];
    assert_eq!(sample.sample_index, 1);
    assert!(sample.is_protected);
    assert_eq!(sample.crypt_byte_block, 1);
    assert_eq!(sample.skip_byte_block, 9);
    assert_eq!(sample.per_sample_iv_size, Some(8));
    assert_eq!(sample.initialization_vector, &[1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(
        sample.effective_initialization_vector(),
        &[1, 2, 3, 4, 5, 6, 7, 8]
    );
    assert_eq!(sample.subsamples.len(), 1);
    assert_eq!(sample.auxiliary_info_size, 16);
    assert!(matches!(
        sample.metadata_source,
        ResolvedSampleEncryptionSource::SampleGroupDescription {
            group_description_index: 65_537,
            description_index: 1,
            fragment_local: true,
        }
    ));
}

#[test]
fn resolve_sample_encryption_falls_back_to_tenc_when_group_mapping_runs_out() {
    let tenc = sample_tenc();
    let mut senc = Senc::default();
    senc.sample_count = 2;
    senc.samples = vec![
        sample_with_iv([1, 2, 3, 4, 5, 6, 7, 8]),
        sample_with_iv([8, 7, 6, 5, 4, 3, 2, 1]),
    ];
    let mut sgpd = Sgpd::default();
    sgpd.grouping_type = fourcc("seig");
    sgpd.entry_count = 1;
    sgpd.seig_entries = vec![SeigEntry {
        crypt_byte_block: 5,
        skip_byte_block: 3,
        is_protected: 1,
        per_sample_iv_size: 8,
        kid: [0xaa; 16],
        ..SeigEntry::default()
    }];
    let mut sbgp = Sbgp::default();
    sbgp.grouping_type = u32::from_be_bytes(*b"seig");
    sbgp.entry_count = 1;
    sbgp.entries = vec![SbgpEntry {
        sample_count: 1,
        group_description_index: 1,
    }];

    let resolved = resolve_sample_encryption(
        &senc,
        SampleEncryptionContext {
            tenc: Some(&tenc),
            sgpd: Some(&sgpd),
            sbgp: Some(&sbgp),
            saiz: None,
        },
    )
    .unwrap();

    assert_eq!(resolved.samples.len(), 2);
    assert!(matches!(
        resolved.samples[0].metadata_source,
        ResolvedSampleEncryptionSource::SampleGroupDescription {
            group_description_index: 1,
            description_index: 1,
            fragment_local: false,
        }
    ));
    assert_eq!(resolved.samples[0].crypt_byte_block, 5);
    assert_eq!(resolved.samples[0].skip_byte_block, 3);
    assert_eq!(resolved.samples[0].kid, [0xaa; 16]);

    assert_eq!(
        resolved.samples[1].metadata_source,
        ResolvedSampleEncryptionSource::TrackEncryptionBox
    );
    assert_eq!(resolved.samples[1].crypt_byte_block, 1);
    assert_eq!(resolved.samples[1].skip_byte_block, 9);
    assert_eq!(resolved.samples[1].kid, [0x11; 16]);
}

#[test]
fn resolve_sample_encryption_reports_missing_fragment_local_description() {
    let tenc = sample_tenc();
    let mut senc = Senc::default();
    senc.sample_count = 1;
    senc.samples = vec![sample_with_iv([1, 2, 3, 4, 5, 6, 7, 8])];
    let mut sbgp = Sbgp::default();
    sbgp.grouping_type = u32::from_be_bytes(*b"seig");
    sbgp.entry_count = 1;
    sbgp.entries = vec![SbgpEntry {
        sample_count: 1,
        group_description_index: 65_537,
    }];

    let error = resolve_sample_encryption(
        &senc,
        SampleEncryptionContext {
            tenc: Some(&tenc),
            sgpd: None,
            sbgp: Some(&sbgp),
            saiz: None,
        },
    )
    .unwrap_err();

    assert_eq!(
        error,
        ResolveSampleEncryptionError::MissingSampleGroupDescription {
            sample_index: 1,
            group_description_index: 65_537,
            description_index: 1,
            fragment_local: true,
        }
    );
}

#[test]
fn resolve_sample_encryption_validates_inline_iv_sizes_against_defaults() {
    let tenc = sample_tenc();
    let mut senc = Senc::default();
    senc.sample_count = 1;
    senc.samples = vec![SencSample {
        initialization_vector: vec![1, 2, 3, 4, 5, 6, 7],
        subsamples: Vec::new(),
    }];

    let error = resolve_sample_encryption(
        &senc,
        SampleEncryptionContext {
            tenc: Some(&tenc),
            sgpd: None,
            sbgp: None,
            saiz: None,
        },
    )
    .unwrap_err();

    assert_eq!(
        error,
        ResolveSampleEncryptionError::SampleInitializationVectorSizeMismatch {
            sample_index: 1,
            expected: 8,
            actual: 7,
        }
    );
}

#[test]
fn resolve_sample_encryption_validates_saiz_sample_sizes() {
    let tenc = sample_tenc();
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

    let mut saiz = Saiz::default();
    saiz.sample_count = 1;
    saiz.sample_info_size = vec![15];

    let error = resolve_sample_encryption(
        &senc,
        SampleEncryptionContext {
            tenc: Some(&tenc),
            sgpd: None,
            sbgp: None,
            saiz: Some(&saiz),
        },
    )
    .unwrap_err();

    assert_eq!(
        error,
        ResolveSampleEncryptionError::SaizSampleInfoSizeMismatch {
            sample_index: 1,
            expected: 16,
            actual: 15,
        }
    );
}

fn sample_tenc() -> Tenc {
    let mut tenc = Tenc::default();
    tenc.set_version(1);
    tenc.default_crypt_byte_block = 1;
    tenc.default_skip_byte_block = 9;
    tenc.default_is_protected = 1;
    tenc.default_per_sample_iv_size = 8;
    tenc.default_kid = [0x11; 16];
    tenc
}

fn sample_with_iv(iv: [u8; 8]) -> SencSample {
    SencSample {
        initialization_vector: iv.to_vec(),
        subsamples: Vec::new(),
    }
}
