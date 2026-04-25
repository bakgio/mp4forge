use std::any::type_name;
use std::fmt::Debug;
use std::io::Cursor;

use mp4forge::FourCc;
use mp4forge::boxes::default_registry;
use mp4forge::boxes::iso23001_7::{
    Pssh, PsshKid, SENC_USE_SUBSAMPLE_ENCRYPTION, Senc, SencSample, SencSubsample, Tenc,
};
use mp4forge::codec::{CodecBox, MutableBox, marshal, unmarshal, unmarshal_any};
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

#[test]
fn protection_catalog_roundtrips() {
    let system_id = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
        0x10,
    ];
    let default_kid = [
        0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd,
        0xef,
    ];

    let mut pssh_v0 = Pssh::default();
    pssh_v0.set_version(0);
    pssh_v0.system_id = system_id;
    pssh_v0.data_size = 5;
    pssh_v0.data = vec![0x21, 0x22, 0x23, 0x24, 0x25];

    assert_box_roundtrip(
        pssh_v0,
        &[
            0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a,
            0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x00, 0x00, 0x00, 0x05, 0x21, 0x22, 0x23, 0x24,
            0x25,
        ],
        "Version=0 Flags=0x000000 SystemID=01020304-0506-0708-090a-0b0c0d0e0f10 DataSize=5 Data=[0x21, 0x22, 0x23, 0x24, 0x25]",
    );

    let mut pssh_v1 = Pssh::default();
    pssh_v1.set_version(1);
    pssh_v1.system_id = system_id;
    pssh_v1.kid_count = 2;
    pssh_v1.kids = vec![
        PsshKid {
            kid: [
                0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e,
                0x1f, 0x10,
            ],
        },
        PsshKid {
            kid: [
                0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2d, 0x2e,
                0x2f, 0x20,
            ],
        },
    ];
    pssh_v1.data_size = 5;
    pssh_v1.data = vec![0x21, 0x22, 0x23, 0x24, 0x25];

    assert_box_roundtrip(
        pssh_v1,
        &[
            0x01, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a,
            0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x00, 0x00, 0x00, 0x02, 0x11, 0x12, 0x13, 0x14,
            0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x10, 0x21, 0x22,
            0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2d, 0x2e, 0x2f, 0x20,
            0x00, 0x00, 0x00, 0x05, 0x21, 0x22, 0x23, 0x24, 0x25,
        ],
        "Version=1 Flags=0x000000 SystemID=01020304-0506-0708-090a-0b0c0d0e0f10 KIDCount=2 KIDs=[11121314-1516-1718-191a-1b1c1d1e1f10, 21222324-2526-2728-292a-2b2c2d2e2f20] DataSize=5 Data=[0x21, 0x22, 0x23, 0x24, 0x25]",
    );

    let mut senc = Senc::default();
    senc.set_version(0);
    senc.sample_count = 2;
    senc.samples = vec![
        SencSample {
            initialization_vector: vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08],
            subsamples: Vec::new(),
        },
        SencSample {
            initialization_vector: vec![0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18],
            subsamples: Vec::new(),
        },
    ];

    assert_box_roundtrip(
        senc,
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06,
            0x07, 0x08, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18,
        ],
        "Version=0 Flags=0x000000 SampleCount=2 Samples=[{InitializationVector=[0x1, 0x2, 0x3, 0x4, 0x5, 0x6, 0x7, 0x8]}, {InitializationVector=[0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18]}]",
    );

    let mut senc_subsamples = Senc::default();
    senc_subsamples.set_version(0);
    senc_subsamples.set_flags(SENC_USE_SUBSAMPLE_ENCRYPTION);
    senc_subsamples.sample_count = 2;
    senc_subsamples.samples = vec![
        SencSample {
            initialization_vector: vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08],
            subsamples: vec![
                SencSubsample {
                    bytes_of_clear_data: 1,
                    bytes_of_protected_data: 2,
                },
                SencSubsample {
                    bytes_of_clear_data: 3,
                    bytes_of_protected_data: 4,
                },
            ],
        },
        SencSample {
            initialization_vector: vec![0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18],
            subsamples: vec![SencSubsample {
                bytes_of_clear_data: 5,
                bytes_of_protected_data: 6,
            }],
        },
    ];

    assert_box_roundtrip(
        senc_subsamples,
        &[
            0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06,
            0x07, 0x08, 0x00, 0x02, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x03, 0x00, 0x00,
            0x00, 0x04, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x00, 0x01, 0x00, 0x05,
            0x00, 0x00, 0x00, 0x06,
        ],
        "Version=0 Flags=0x000002 SampleCount=2 Samples=[{InitializationVector=[0x1, 0x2, 0x3, 0x4, 0x5, 0x6, 0x7, 0x8] Subsamples=[{BytesOfClearData=1 BytesOfProtectedData=2}, {BytesOfClearData=3 BytesOfProtectedData=4}]}, {InitializationVector=[0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18] Subsamples=[{BytesOfClearData=5 BytesOfProtectedData=6}]}]",
    );

    let mut tenc_constant_iv = Tenc::default();
    tenc_constant_iv.set_version(1);
    tenc_constant_iv.reserved = 0x00;
    tenc_constant_iv.default_crypt_byte_block = 0x0a;
    tenc_constant_iv.default_skip_byte_block = 0x0b;
    tenc_constant_iv.default_is_protected = 1;
    tenc_constant_iv.default_per_sample_iv_size = 0;
    tenc_constant_iv.default_kid = default_kid;
    tenc_constant_iv.default_constant_iv_size = 4;
    tenc_constant_iv.default_constant_iv = vec![0x01, 0x23, 0x45, 0x67];

    assert_box_roundtrip(
        tenc_constant_iv,
        &[
            0x01, 0x00, 0x00, 0x00, 0x00, 0xab, 0x01, 0x00, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x04, 0x01, 0x23, 0x45,
            0x67,
        ],
        "Version=1 Flags=0x000000 Reserved=0 DefaultCryptByteBlock=10 DefaultSkipByteBlock=11 DefaultIsProtected=1 DefaultPerSampleIVSize=0 DefaultKID=01234567-89ab-cdef-0123-456789abcdef DefaultConstantIVSize=4 DefaultConstantIV=[0x1, 0x23, 0x45, 0x67]",
    );

    let mut tenc_unprotected = Tenc::default();
    tenc_unprotected.set_version(1);
    tenc_unprotected.reserved = 0x00;
    tenc_unprotected.default_crypt_byte_block = 0x0a;
    tenc_unprotected.default_skip_byte_block = 0x0b;
    tenc_unprotected.default_is_protected = 0;
    tenc_unprotected.default_per_sample_iv_size = 0;
    tenc_unprotected.default_kid = default_kid;

    assert_box_roundtrip(
        tenc_unprotected,
        &[
            0x01, 0x00, 0x00, 0x00, 0x00, 0xab, 0x00, 0x00, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
        ],
        "Version=1 Flags=0x000000 Reserved=0 DefaultCryptByteBlock=10 DefaultSkipByteBlock=11 DefaultIsProtected=0 DefaultPerSampleIVSize=0 DefaultKID=01234567-89ab-cdef-0123-456789abcdef",
    );

    let mut tenc_per_sample_iv = Tenc::default();
    tenc_per_sample_iv.set_version(1);
    tenc_per_sample_iv.reserved = 0x00;
    tenc_per_sample_iv.default_crypt_byte_block = 0x0a;
    tenc_per_sample_iv.default_skip_byte_block = 0x0b;
    tenc_per_sample_iv.default_is_protected = 1;
    tenc_per_sample_iv.default_per_sample_iv_size = 1;
    tenc_per_sample_iv.default_kid = default_kid;

    assert_box_roundtrip(
        tenc_per_sample_iv,
        &[
            0x01, 0x00, 0x00, 0x00, 0x00, 0xab, 0x01, 0x01, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
        ],
        "Version=1 Flags=0x000000 Reserved=0 DefaultCryptByteBlock=10 DefaultSkipByteBlock=11 DefaultIsProtected=1 DefaultPerSampleIVSize=1 DefaultKID=01234567-89ab-cdef-0123-456789abcdef",
    );
}

#[test]
fn built_in_registry_reports_supported_versions_for_landed_protection_types() {
    let registry = default_registry();

    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"pssh")),
        Some(&[0, 1][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"senc")),
        Some(&[0][..])
    );
    assert_eq!(
        registry.supported_versions(FourCc::from_bytes(*b"tenc")),
        Some(&[0, 1][..])
    );
    assert!(registry.is_registered(FourCc::from_bytes(*b"pssh")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"senc")));
    assert!(registry.is_registered(FourCc::from_bytes(*b"tenc")));
}

#[test]
fn pssh_rejects_kid_count_mismatch_during_marshal() {
    let mut pssh = Pssh::default();
    pssh.set_version(1);
    pssh.system_id = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
        0x10,
    ];
    pssh.kid_count = 2;
    pssh.kids = vec![PsshKid {
        kid: [
            0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e,
            0x1f, 0x10,
        ],
    }];

    let error = marshal(&mut Vec::new(), &pssh, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid element count for field KIDs: expected 32, got 16"
    );
}

#[test]
fn tenc_rejects_constant_iv_length_mismatch_during_marshal() {
    let mut tenc = Tenc::default();
    tenc.set_version(1);
    tenc.default_is_protected = 1;
    tenc.default_per_sample_iv_size = 0;
    tenc.default_kid = [
        0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd,
        0xef,
    ];
    tenc.default_constant_iv_size = 4;
    tenc.default_constant_iv = vec![0x01, 0x23, 0x45];

    let error = marshal(&mut Vec::new(), &tenc, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid element count for field DefaultConstantIV: expected 4, got 3"
    );
}

#[test]
fn senc_rejects_sample_count_mismatch_during_marshal() {
    let mut senc = Senc::default();
    senc.set_version(0);
    senc.sample_count = 2;
    senc.samples = vec![SencSample {
        initialization_vector: vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08],
        subsamples: Vec::new(),
    }];

    let error = marshal(&mut Vec::new(), &senc, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid element count for field Samples: expected 2, got 1"
    );
}

#[test]
fn senc_rejects_subsample_records_without_flag_during_marshal() {
    let mut senc = Senc::default();
    senc.set_version(0);
    senc.sample_count = 1;
    senc.samples = vec![SencSample {
        initialization_vector: vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08],
        subsamples: vec![SencSubsample {
            bytes_of_clear_data: 1,
            bytes_of_protected_data: 2,
        }],
    }];

    let error = marshal(&mut Vec::new(), &senc, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid field value for Samples: subsample records require the UseSubSampleEncryption flag"
    );
}

#[test]
fn senc_rejects_unsupported_versions_during_unmarshal() {
    let payload = [0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    let mut decoded = Senc::default();
    let error = unmarshal(
        &mut Cursor::new(payload),
        payload.len() as u64,
        &mut decoded,
        None,
    )
    .unwrap_err();

    assert_eq!(error.to_string(), "unsupported box version 1 for type senc");
}
