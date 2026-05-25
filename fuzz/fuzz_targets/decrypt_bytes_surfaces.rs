#![no_main]

mod support;

use libfuzzer_sys::fuzz_target;
use mp4forge::FourCc;
use mp4forge::boxes::iso23001_7::SencSubsample;
use mp4forge::decrypt::{
    DecryptOptions, DecryptionKey, NativeCommonEncryptionScheme, decrypt_bytes,
    decrypt_bytes_with_progress, decrypt_common_encryption_file_bytes,
    decrypt_common_encryption_init_bytes, decrypt_common_encryption_media_segment_bytes,
    decrypt_common_encryption_sample, decrypt_common_encryption_sample_by_scheme_type_with_keys,
    decrypt_common_encryption_sample_with_keys, select_decryption_key,
};
use mp4forge::encryption::{ResolvedSampleEncryptionSample, ResolvedSampleEncryptionSource};

use support::{FuzzInput, seeded_small_mp4_bytes};

const MAX_SAMPLE_LEN: usize = 8192;

fuzz_target!(|data: &[u8]| {
    let mut input = FuzzInput::new(data);
    let bytes = seeded_small_mp4_bytes(&mut input);
    let fragments_info = seeded_small_mp4_bytes(&mut input);
    let keys = take_keys(&mut input);
    let sample_bytes = input.take_bytes(MAX_SAMPLE_LEN);
    let sample = take_resolved_sample(&mut input);
    let scheme = take_scheme(&mut input);

    let _ = select_decryption_key(&keys, take_track_id(&mut input), &sample);
    let _ = decrypt_common_encryption_sample(scheme, input.take_exact(), &sample, &sample_bytes);
    let _ = decrypt_common_encryption_sample_with_keys(
        scheme,
        take_track_id(&mut input),
        &keys,
        &sample,
        &sample_bytes,
    );
    let _ = decrypt_common_encryption_sample_by_scheme_type_with_keys(
        take_scheme_type(&mut input),
        take_track_id(&mut input),
        &keys,
        &sample,
        &sample_bytes,
    );

    let _ = decrypt_common_encryption_init_bytes(&bytes, &keys);
    let _ = decrypt_common_encryption_media_segment_bytes(&fragments_info, &bytes, &keys);
    let _ = decrypt_common_encryption_file_bytes(&bytes, &keys);

    let mut options = DecryptOptions::new();
    for key in &keys {
        options.add_key(*key);
    }
    if input.take_bool() {
        options.set_fragments_info_bytes(&fragments_info);
    }

    let _ = decrypt_bytes(&bytes, &options);
    let _ = decrypt_bytes_with_progress(&bytes, &options, |_| {});
});

fn take_keys(input: &mut FuzzInput<'_>) -> Vec<DecryptionKey> {
    let mut keys = Vec::new();
    for _ in 0..input.take_usize(4) {
        let key = input.take_exact();
        if input.take_bool() {
            keys.push(DecryptionKey::track(input.take_u32(), key));
        } else {
            keys.push(DecryptionKey::kid(input.take_exact(), key));
        }
    }
    keys
}

fn take_resolved_sample(input: &mut FuzzInput<'_>) -> ResolvedSampleEncryptionSample<'static> {
    let initialization_vector = leak_bytes(input.take_bytes(16));
    let constant_iv = if input.take_bool() {
        Some(leak_bytes(input.take_bytes(16)))
    } else {
        None
    };
    let mut subsamples = Vec::new();
    for _ in 0..input.take_usize(4) {
        subsamples.push(SencSubsample {
            bytes_of_clear_data: input.take_u16(),
            bytes_of_protected_data: input.take_u32(),
        });
    }
    let subsamples = Box::leak(subsamples.into_boxed_slice());

    ResolvedSampleEncryptionSample {
        sample_index: input.take_u32(),
        metadata_source: ResolvedSampleEncryptionSource::TrackEncryptionBox,
        is_protected: input.take_bool(),
        crypt_byte_block: input.take_u8() & 0x0f,
        skip_byte_block: input.take_u8() & 0x0f,
        per_sample_iv_size: match input.take_u8() % 4 {
            0 => None,
            1 => Some(0),
            2 => Some(8),
            _ => Some(16),
        },
        initialization_vector,
        constant_iv,
        kid: input.take_exact(),
        subsamples,
        auxiliary_info_size: input.take_u32(),
    }
}

fn leak_bytes(bytes: Vec<u8>) -> &'static [u8] {
    Box::leak(bytes.into_boxed_slice())
}

fn take_scheme(input: &mut FuzzInput<'_>) -> NativeCommonEncryptionScheme {
    match input.take_u8() % 4 {
        0 => NativeCommonEncryptionScheme::Cenc,
        1 => NativeCommonEncryptionScheme::Cens,
        2 => NativeCommonEncryptionScheme::Cbc1,
        _ => NativeCommonEncryptionScheme::Cbcs,
    }
}

fn take_scheme_type(input: &mut FuzzInput<'_>) -> FourCc {
    match input.take_u8() % 5 {
        0 => FourCc::from_bytes(*b"cenc"),
        1 => FourCc::from_bytes(*b"cens"),
        2 => FourCc::from_bytes(*b"cbc1"),
        3 => FourCc::from_bytes(*b"cbcs"),
        _ => input.take_fourcc(),
    }
}

fn take_track_id(input: &mut FuzzInput<'_>) -> Option<u32> {
    if input.take_bool() {
        Some(input.take_u32())
    } else {
        None
    }
}
