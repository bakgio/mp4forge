#[cfg(feature = "mux")]
use mp4forge::boxes::iso14496_12::AVCDecoderConfiguration;
#[cfg(feature = "mux")]
use mp4forge::mux::rewrite::rewrite_avc_sample_to_annex_b;

#[cfg(feature = "mux")]
fn main() {
    let avcc = AVCDecoderConfiguration {
        length_size_minus_one: 3,
        ..Default::default()
    };
    let sample = [
        0x00, 0x00, 0x00, 0x02, 0x65, 0x88, 0x00, 0x00, 0x00, 0x01, 0x06,
    ];
    let rewritten = rewrite_avc_sample_to_annex_b(&sample, &avcc).unwrap();

    println!(
        "rewrote one {}-byte AVC sample into {} Annex B bytes",
        sample.len(),
        rewritten.len()
    );
}

#[cfg(not(feature = "mux"))]
fn main() {
    eprintln!("Enable the `mux` feature to run this example.");
}
