use std::path::Path;

use crate::boxes::vp::VpCodecConfiguration;
use crate::codec::MutableBox;

use super::super::import::build_visual_sample_entry_box_with_compressor_name;
use super::super::{MuxError, MuxRawCodec};
#[cfg(feature = "async")]
use super::ivf_common::read_indexed_sample_async;
#[cfg(feature = "async")]
use super::ivf_common::scan_ivf_video_file_async;
use super::ivf_common::{ParsedIvfTrack, read_indexed_sample_sync, scan_ivf_video_file_sync};

const VP10_COMPRESSOR_NAME: &[u8] = b"VPC Coding";

pub(in crate::mux) fn scan_vp10_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedIvfTrack, MuxError> {
    let indexed = scan_ivf_video_file_sync(path, MuxRawCodec::Vp10, spec)?;
    let first_sample = read_indexed_sample_sync(
        path,
        indexed.first_sample_span,
        spec,
        "IVF VP10 sample payload is truncated",
    )?;
    let sample_entry_box =
        build_vp10_sample_entry_box(indexed.width, indexed.height, &first_sample, spec)?;
    Ok(ParsedIvfTrack {
        width: indexed.width,
        height: indexed.height,
        timescale: indexed.timescale,
        sample_entry_box,
        samples: indexed.samples,
    })
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_vp10_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedIvfTrack, MuxError> {
    let indexed = scan_ivf_video_file_async(path, MuxRawCodec::Vp10, spec).await?;
    let first_sample = read_indexed_sample_async(
        path,
        indexed.first_sample_span,
        spec,
        "IVF VP10 sample payload is truncated",
    )
    .await?;
    let sample_entry_box =
        build_vp10_sample_entry_box(indexed.width, indexed.height, &first_sample, spec)?;
    Ok(ParsedIvfTrack {
        width: indexed.width,
        height: indexed.height,
        timescale: indexed.timescale,
        sample_entry_box,
        samples: indexed.samples,
    })
}

fn build_vp10_sample_entry_box(
    width: u16,
    height: u16,
    sample: &[u8],
    spec: &str,
) -> Result<Vec<u8>, MuxError> {
    if sample.is_empty() {
        return Err(unsupported(
            spec,
            "VP10 direct input must include at least one IVF frame payload",
        ));
    }
    let child_boxes = vec![super::super::mp4::encode_typed_box(
        &default_vp10_config(),
        &[],
    )?];
    build_visual_sample_entry_box_with_compressor_name(
        crate::FourCc::from_bytes(*b"vp10"),
        width,
        height,
        VP10_COMPRESSOR_NAME,
        &child_boxes,
    )
}

fn default_vp10_config() -> VpCodecConfiguration {
    let mut config = VpCodecConfiguration::default();
    config.set_version(1);
    config.profile = 1;
    config.level = 10;
    config.bit_depth = 8;
    config.chroma_subsampling = 0;
    config.video_full_range_flag = 0;
    config.colour_primaries = 0;
    config.transfer_characteristics = 0;
    config.matrix_coefficients = 0;
    config.codec_initialization_data_size = 0;
    config.codec_initialization_data = Vec::new();
    config
}

fn unsupported(spec: &str, message: &str) -> MuxError {
    MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: message.to_string(),
    }
}
