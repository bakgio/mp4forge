#![no_main]

mod support;

use libfuzzer_sys::fuzz_target;
use mp4forge::boxes::iso14496_12::{AVCDecoderConfiguration, HEVCDecoderConfiguration};
use mp4forge::boxes::iso14496_15::VVCDecoderConfiguration;
use mp4forge::mux::rewrite::{
    rewrite_aac_sample_to_adts, rewrite_av1_sample_to_annex_b, rewrite_avc_sample_to_annex_b,
    rewrite_hevc_sample_to_annex_b, rewrite_mhas_samples_to_stream, rewrite_vvc_sample_to_annex_b,
};

use support::FuzzInput;

const MAX_SAMPLE_LEN: usize = 16 * 1024;
const MAX_CONFIG_LEN: usize = 256;
const MAX_MHAS_SAMPLE_LEN: usize = 4096;

fuzz_target!(|data: &[u8]| {
    let mut input = FuzzInput::new(data);
    let sample = input.take_bytes(MAX_SAMPLE_LEN);

    match input.take_u8() % 6 {
        0 => {
            let avcc = AVCDecoderConfiguration {
                length_size_minus_one: input.take_u8() & 0x07,
                ..Default::default()
            };
            let _ = rewrite_avc_sample_to_annex_b(&sample, &avcc);
        }
        1 => {
            let hvcc = HEVCDecoderConfiguration {
                length_size_minus_one: input.take_u8() & 0x07,
                ..Default::default()
            };
            let _ = rewrite_hevc_sample_to_annex_b(&sample, &hvcc);
        }
        2 => {
            let vvcc = VVCDecoderConfiguration {
                decoder_configuration_record: input.take_bytes(MAX_CONFIG_LEN),
                ..Default::default()
            };
            let _ = rewrite_vvc_sample_to_annex_b(&sample, &vvcc);
        }
        3 => {
            let _ = rewrite_av1_sample_to_annex_b(&sample);
        }
        4 => {
            let audio_specific_config = input.take_bytes(MAX_CONFIG_LEN);
            let _ = rewrite_aac_sample_to_adts(&sample, &audio_specific_config);
        }
        _ => {
            let samples = take_mhas_samples(&mut input, sample);
            let refs = samples.iter().map(Vec::as_slice).collect::<Vec<_>>();
            let _ = rewrite_mhas_samples_to_stream(&refs);
        }
    }
});

fn take_mhas_samples(input: &mut FuzzInput<'_>, first_sample: Vec<u8>) -> Vec<Vec<u8>> {
    let mut samples = vec![first_sample];
    for _ in 0..input.take_usize(3) {
        samples.push(input.take_bytes(MAX_MHAS_SAMPLE_LEN));
    }
    samples
}
