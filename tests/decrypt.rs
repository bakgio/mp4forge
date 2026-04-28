#![cfg(feature = "decrypt")]

use aes::Aes128;
use aes::cipher::{Block, BlockEncrypt, KeyInit};

use mp4forge::FourCc;
use mp4forge::boxes::iso23001_7::SencSubsample;
use mp4forge::decrypt::{
    CommonEncryptionDecryptError, DecryptionKey, NativeCommonEncryptionScheme,
    decrypt_common_encryption_sample, decrypt_common_encryption_sample_by_scheme_type_with_keys,
    decrypt_common_encryption_sample_with_keys, select_decryption_key,
};
use mp4forge::encryption::{ResolvedSampleEncryptionSample, ResolvedSampleEncryptionSource};

#[test]
fn decrypt_cenc_audio_fragment_roundtrips_full_sample_ctr() {
    let key = [0x11; 16];
    let sample = resolved_sample(SampleSpec {
        is_protected: true,
        initialization_vector: vec![1, 2, 3, 4, 5, 6, 7, 8],
        constant_iv: None,
        per_sample_iv_size: Some(8),
        crypt_byte_block: 0,
        skip_byte_block: 0,
        kid: [0xaa; 16],
        subsamples: vec![],
    });
    let plaintext = (0u8..37).collect::<Vec<_>>();
    let ciphertext = encrypt_sample(NativeCommonEncryptionScheme::Cenc, key, &sample, &plaintext);

    let decrypted = decrypt_common_encryption_sample(
        NativeCommonEncryptionScheme::Cenc,
        key,
        &sample,
        &ciphertext,
    )
    .unwrap();
    assert_eq!(decrypted, plaintext);
}

#[test]
fn decrypt_cens_video_fragment_keeps_pattern_position_across_subsamples() {
    let key = [0x22; 16];
    let sample = resolved_sample(SampleSpec {
        is_protected: true,
        initialization_vector: vec![9, 8, 7, 6, 5, 4, 3, 2],
        constant_iv: None,
        per_sample_iv_size: Some(8),
        crypt_byte_block: 1,
        skip_byte_block: 1,
        kid: [0xbb; 16],
        subsamples: vec![
            SencSubsample {
                bytes_of_clear_data: 4,
                bytes_of_protected_data: 48,
            },
            SencSubsample {
                bytes_of_clear_data: 2,
                bytes_of_protected_data: 32,
            },
        ],
    });
    let plaintext = (0u8..86)
        .map(|value| value.wrapping_mul(3))
        .collect::<Vec<_>>();
    let ciphertext = encrypt_sample(NativeCommonEncryptionScheme::Cens, key, &sample, &plaintext);

    let decrypted = decrypt_common_encryption_sample(
        NativeCommonEncryptionScheme::Cens,
        key,
        &sample,
        &ciphertext,
    )
    .unwrap();
    assert_eq!(decrypted, plaintext);
}

#[test]
fn decrypt_cbc1_audio_fragment_leaves_partial_tail_clear() {
    let key = [0x33; 16];
    let sample = resolved_sample(SampleSpec {
        is_protected: true,
        initialization_vector: vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
        constant_iv: None,
        per_sample_iv_size: Some(16),
        crypt_byte_block: 0,
        skip_byte_block: 0,
        kid: [0xcc; 16],
        subsamples: vec![],
    });
    let plaintext = (0u8..37).map(|value| value ^ 0x5a).collect::<Vec<_>>();
    let ciphertext = encrypt_sample(NativeCommonEncryptionScheme::Cbc1, key, &sample, &plaintext);

    let decrypted = decrypt_common_encryption_sample(
        NativeCommonEncryptionScheme::Cbc1,
        key,
        &sample,
        &ciphertext,
    )
    .unwrap();
    assert_eq!(decrypted, plaintext);
    assert_eq!(&ciphertext[32..], &plaintext[32..]);
}

#[test]
fn decrypt_cbcs_video_fragment_resets_iv_at_each_subsample() {
    let key = [0x44; 16];
    let sample = resolved_sample(SampleSpec {
        is_protected: true,
        initialization_vector: vec![],
        constant_iv: Some(vec![
            0x10, 0x32, 0x54, 0x76, 0x98, 0xba, 0xdc, 0xfe, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef,
        ]),
        per_sample_iv_size: None,
        crypt_byte_block: 1,
        skip_byte_block: 1,
        kid: [0xdd; 16],
        subsamples: vec![
            SencSubsample {
                bytes_of_clear_data: 4,
                bytes_of_protected_data: 48,
            },
            SencSubsample {
                bytes_of_clear_data: 2,
                bytes_of_protected_data: 32,
            },
        ],
    });
    let plaintext = (0u8..86)
        .map(|value| value.wrapping_mul(5))
        .collect::<Vec<_>>();
    let ciphertext = encrypt_sample(NativeCommonEncryptionScheme::Cbcs, key, &sample, &plaintext);

    let decrypted = decrypt_common_encryption_sample(
        NativeCommonEncryptionScheme::Cbcs,
        key,
        &sample,
        &ciphertext,
    )
    .unwrap();
    assert_eq!(decrypted, plaintext);
}

#[test]
fn decrypt_with_keys_prefers_track_id_before_kid() {
    let track_key = [0x55; 16];
    let kid_key = [0x66; 16];
    let sample = resolved_sample(SampleSpec {
        is_protected: true,
        initialization_vector: vec![1, 1, 1, 1, 1, 1, 1, 1],
        constant_iv: None,
        per_sample_iv_size: Some(8),
        crypt_byte_block: 0,
        skip_byte_block: 0,
        kid: [0xee; 16],
        subsamples: vec![],
    });
    let keys = vec![
        DecryptionKey::kid([0xee; 16], kid_key),
        DecryptionKey::track(7, track_key),
    ];
    let plaintext = (0u8..32).collect::<Vec<_>>();
    let ciphertext = encrypt_sample(
        NativeCommonEncryptionScheme::Cenc,
        track_key,
        &sample,
        &plaintext,
    );

    assert_eq!(
        select_decryption_key(&keys, Some(7), &sample).unwrap(),
        track_key
    );
    let decrypted = decrypt_common_encryption_sample_with_keys(
        NativeCommonEncryptionScheme::Cenc,
        Some(7),
        &keys,
        &sample,
        &ciphertext,
    )
    .unwrap();
    assert_eq!(decrypted, plaintext);
}

#[test]
fn decrypt_with_keys_falls_back_to_kid_for_multi_key_layouts() {
    let key = [0x77; 16];
    let sample = resolved_sample(SampleSpec {
        is_protected: true,
        initialization_vector: vec![2, 2, 2, 2, 2, 2, 2, 2],
        constant_iv: None,
        per_sample_iv_size: Some(8),
        crypt_byte_block: 0,
        skip_byte_block: 0,
        kid: [0xfa; 16],
        subsamples: vec![],
    });
    let keys = vec![DecryptionKey::kid([0xfa; 16], key)];
    let plaintext = (0u8..48).map(|value| value ^ 0xa5).collect::<Vec<_>>();
    let ciphertext = encrypt_sample(NativeCommonEncryptionScheme::Cenc, key, &sample, &plaintext);

    let decrypted = decrypt_common_encryption_sample_by_scheme_type_with_keys(
        FourCc::from_bytes(*b"cenc"),
        None,
        &keys,
        &sample,
        &ciphertext,
    )
    .unwrap();
    assert_eq!(decrypted, plaintext);
}

#[test]
fn decrypt_reports_missing_key_and_invalid_iv_and_invalid_regions() {
    let sample = resolved_sample(SampleSpec {
        is_protected: true,
        initialization_vector: vec![1, 2, 3, 4, 5, 6, 7, 8],
        constant_iv: None,
        per_sample_iv_size: Some(8),
        crypt_byte_block: 0,
        skip_byte_block: 0,
        kid: [0x12; 16],
        subsamples: vec![],
    });
    let missing = decrypt_common_encryption_sample_with_keys(
        NativeCommonEncryptionScheme::Cenc,
        Some(99),
        &[],
        &sample,
        &[0u8; 8],
    )
    .unwrap_err();
    assert_eq!(
        missing,
        CommonEncryptionDecryptError::MissingDecryptionKey {
            track_id: Some(99),
            kid: [0x12; 16],
        }
    );

    let invalid_iv = decrypt_common_encryption_sample(
        NativeCommonEncryptionScheme::Cbc1,
        [0x88; 16],
        &sample,
        &[0u8; 8],
    )
    .unwrap_err();
    assert_eq!(
        invalid_iv,
        CommonEncryptionDecryptError::InvalidInitializationVectorSize {
            scheme: NativeCommonEncryptionScheme::Cbc1,
            actual: 8,
            expected: "exactly 16",
        }
    );

    let invalid_region_sample = resolved_sample(SampleSpec {
        is_protected: true,
        initialization_vector: vec![1, 2, 3, 4, 5, 6, 7, 8],
        constant_iv: None,
        per_sample_iv_size: Some(8),
        crypt_byte_block: 0,
        skip_byte_block: 0,
        kid: [0x34; 16],
        subsamples: vec![SencSubsample {
            bytes_of_clear_data: 4,
            bytes_of_protected_data: 16,
        }],
    });
    let invalid_region = decrypt_common_encryption_sample(
        NativeCommonEncryptionScheme::Cenc,
        [0x99; 16],
        &invalid_region_sample,
        &[0u8; 8],
    )
    .unwrap_err();
    assert_eq!(
        invalid_region,
        CommonEncryptionDecryptError::InvalidProtectedRegion {
            remaining: 8,
            clear_bytes: 4,
            protected_bytes: 16,
        }
    );
}

#[test]
fn decrypt_by_scheme_type_rejects_non_native_codes() {
    let sample = resolved_sample(SampleSpec {
        is_protected: false,
        initialization_vector: vec![],
        constant_iv: None,
        per_sample_iv_size: None,
        crypt_byte_block: 0,
        skip_byte_block: 0,
        kid: [0u8; 16],
        subsamples: vec![],
    });
    let error = decrypt_common_encryption_sample_by_scheme_type_with_keys(
        FourCc::from_bytes(*b"piff"),
        None,
        &[],
        &sample,
        &[],
    )
    .unwrap_err();
    assert_eq!(
        error,
        CommonEncryptionDecryptError::UnsupportedNativeSchemeType {
            scheme_type: FourCc::from_bytes(*b"piff"),
        }
    );
}

struct SampleSpec {
    is_protected: bool,
    initialization_vector: Vec<u8>,
    constant_iv: Option<Vec<u8>>,
    per_sample_iv_size: Option<u8>,
    crypt_byte_block: u8,
    skip_byte_block: u8,
    kid: [u8; 16],
    subsamples: Vec<SencSubsample>,
}

#[derive(Clone, Copy)]
struct EncryptPattern {
    crypt_byte_block: u8,
    skip_byte_block: u8,
}

struct EncryptState {
    ctr_offset: u64,
    pattern_offset: u64,
    chain_block: [u8; 16],
}

fn resolved_sample(spec: SampleSpec) -> ResolvedSampleEncryptionSample<'static> {
    let initialization_vector = Box::leak(spec.initialization_vector.into_boxed_slice());
    let constant_iv = spec
        .constant_iv
        .map(|bytes| Box::leak(bytes.into_boxed_slice()) as &'static [u8]);
    let subsamples = Box::leak(spec.subsamples.into_boxed_slice());
    ResolvedSampleEncryptionSample {
        sample_index: 1,
        metadata_source: ResolvedSampleEncryptionSource::TrackEncryptionBox,
        is_protected: spec.is_protected,
        crypt_byte_block: spec.crypt_byte_block,
        skip_byte_block: spec.skip_byte_block,
        per_sample_iv_size: spec.per_sample_iv_size,
        initialization_vector,
        constant_iv,
        kid: spec.kid,
        subsamples,
        auxiliary_info_size: 0,
    }
}

fn encrypt_sample(
    scheme: NativeCommonEncryptionScheme,
    key: [u8; 16],
    sample: &ResolvedSampleEncryptionSample<'_>,
    plaintext: &[u8],
) -> Vec<u8> {
    if !sample.is_protected {
        return plaintext.to_vec();
    }

    let iv = sample.effective_initialization_vector();
    let pattern = EncryptPattern {
        crypt_byte_block: sample.crypt_byte_block,
        skip_byte_block: sample.skip_byte_block,
    };
    let mut output = plaintext.to_vec();
    if sample.subsamples.is_empty() {
        encrypt_region(
            scheme,
            key,
            iv.try_into().unwrap_or_else(|_| {
                let mut padded = [0u8; 16];
                padded[..iv.len()].copy_from_slice(iv);
                padded
            }),
            pattern,
            plaintext,
            &mut output,
        );
        return output;
    }

    let iv_block = if iv.len() == 16 {
        iv.try_into().unwrap()
    } else {
        let mut padded = [0u8; 16];
        padded[..iv.len()].copy_from_slice(iv);
        padded
    };
    let mut cursor = 0usize;
    let mut state = EncryptState {
        ctr_offset: 0,
        pattern_offset: 0,
        chain_block: iv_block,
    };
    for subsample in sample.subsamples {
        let clear = usize::from(subsample.bytes_of_clear_data);
        cursor += clear;
        let protected = usize::try_from(subsample.bytes_of_protected_data).unwrap();
        if scheme == NativeCommonEncryptionScheme::Cbcs {
            state.ctr_offset = 0;
            state.pattern_offset = 0;
            state.chain_block = iv_block;
        }
        encrypt_region_with_state(
            scheme,
            key,
            iv_block,
            pattern,
            &mut state,
            &plaintext[cursor..cursor + protected],
            &mut output[cursor..cursor + protected],
        );
        cursor += protected;
    }
    output
}

fn encrypt_region(
    scheme: NativeCommonEncryptionScheme,
    key: [u8; 16],
    iv: [u8; 16],
    pattern: EncryptPattern,
    plaintext: &[u8],
    output: &mut [u8],
) {
    let mut state = EncryptState {
        ctr_offset: 0,
        pattern_offset: 0,
        chain_block: iv,
    };
    encrypt_region_with_state(scheme, key, iv, pattern, &mut state, plaintext, output);
}

fn encrypt_region_with_state(
    scheme: NativeCommonEncryptionScheme,
    key: [u8; 16],
    iv: [u8; 16],
    pattern: EncryptPattern,
    state: &mut EncryptState,
    plaintext: &[u8],
    output: &mut [u8],
) {
    if pattern.crypt_byte_block != 0 && pattern.skip_byte_block != 0 {
        let pattern_span =
            usize::from(pattern.crypt_byte_block) + usize::from(pattern.skip_byte_block);
        let mut cursor = 0usize;
        while cursor < plaintext.len() {
            let block_position = usize::try_from(state.pattern_offset / 16).unwrap();
            let pattern_position = block_position % pattern_span;
            let mut crypt_size = 0usize;
            let mut skip_size = usize::from(pattern.skip_byte_block) * 16;
            if pattern_position < usize::from(pattern.crypt_byte_block) {
                crypt_size = (usize::from(pattern.crypt_byte_block) - pattern_position) * 16;
            } else {
                skip_size = (pattern_span - pattern_position) * 16;
            }

            let remain = plaintext.len() - cursor;
            if crypt_size > remain {
                crypt_size = 16 * (remain / 16);
                skip_size = remain - crypt_size;
            }
            if crypt_size + skip_size > remain {
                skip_size = remain - crypt_size;
            }

            if crypt_size != 0 {
                encrypt_chunk(
                    scheme,
                    key,
                    iv,
                    &mut state.ctr_offset,
                    &mut state.chain_block,
                    &plaintext[cursor..cursor + crypt_size],
                    &mut output[cursor..cursor + crypt_size],
                );
                cursor += crypt_size;
                state.pattern_offset += crypt_size as u64;
            }

            if skip_size != 0 {
                output[cursor..cursor + skip_size]
                    .copy_from_slice(&plaintext[cursor..cursor + skip_size]);
                cursor += skip_size;
                state.pattern_offset += skip_size as u64;
            }
        }
    } else {
        encrypt_chunk(
            scheme,
            key,
            iv,
            &mut state.ctr_offset,
            &mut state.chain_block,
            plaintext,
            output,
        );
    }
}

fn encrypt_chunk(
    scheme: NativeCommonEncryptionScheme,
    key: [u8; 16],
    iv: [u8; 16],
    ctr_offset: &mut u64,
    chain_block: &mut [u8; 16],
    plaintext: &[u8],
    output: &mut [u8],
) {
    match scheme {
        NativeCommonEncryptionScheme::Cenc | NativeCommonEncryptionScheme::Cens => {
            let aes = Aes128::new(&key.into());
            let mut cursor = 0usize;
            while cursor < plaintext.len() {
                let block_offset = usize::try_from(*ctr_offset % 16).unwrap();
                let chunk_len = (16 - block_offset).min(plaintext.len() - cursor);
                let mut counter_block = compute_ctr_counter_block(iv, *ctr_offset);
                aes.encrypt_block(&mut counter_block);
                for index in 0..chunk_len {
                    output[cursor + index] =
                        plaintext[cursor + index] ^ counter_block[block_offset + index];
                }
                cursor += chunk_len;
                *ctr_offset += chunk_len as u64;
            }
        }
        NativeCommonEncryptionScheme::Cbc1 | NativeCommonEncryptionScheme::Cbcs => {
            let aes = Aes128::new(&key.into());
            let full_blocks_len = plaintext.len() - (plaintext.len() % 16);
            let mut cursor = 0usize;
            while cursor < full_blocks_len {
                let mut block = Block::<Aes128>::clone_from_slice(&plaintext[cursor..cursor + 16]);
                for index in 0..16 {
                    block[index] ^= chain_block[index];
                }
                aes.encrypt_block(&mut block);
                output[cursor..cursor + 16].copy_from_slice(&block);
                chain_block.copy_from_slice(&block);
                cursor += 16;
            }
            output[full_blocks_len..].copy_from_slice(&plaintext[full_blocks_len..]);
        }
    }
}

fn compute_ctr_counter_block(iv: [u8; 16], stream_offset: u64) -> Block<Aes128> {
    let counter_offset = stream_offset / 16;
    let counter_offset_bytes = counter_offset.to_be_bytes();
    let mut counter_block = Block::<Aes128>::default();

    let mut carry = 0u16;
    for index in 0..8 {
        let offset = 15 - index;
        let sum = u16::from(iv[offset]) + u16::from(counter_offset_bytes[7 - index]) + carry;
        counter_block[offset] = (sum & 0xff) as u8;
        carry = if sum >= 0x100 { 1 } else { 0 };
    }
    for index in 8..16 {
        let offset = 15 - index;
        counter_block[offset] = iv[offset];
    }

    counter_block
}
