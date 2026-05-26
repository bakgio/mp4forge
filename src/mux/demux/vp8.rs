use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::boxes::vp::VpCodecConfiguration;
use crate::codec::MutableBox;

use super::super::import::{
    StagedSample, build_btrt_from_sample_sizes, build_visual_sample_entry_box_with_compressor_name,
};
use super::super::{MuxError, MuxRawCodec};
#[cfg(feature = "async")]
use super::ivf_common::read_indexed_sample_async;
#[cfg(feature = "async")]
use super::ivf_common::scan_ivf_video_file_async;
use super::ivf_common::{ParsedIvfTrack, read_indexed_sample_sync, scan_ivf_video_file_sync};
#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;
#[cfg(feature = "async")]
use tokio::io::{AsyncReadExt, AsyncSeekExt};

const VP8_SYNC_CODE: [u8; 3] = [0x9D, 0x01, 0x2A];
const VP8_COMPRESSOR_NAME: &[u8] = b"VPC Coding";

pub(in crate::mux) fn scan_vp8_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedIvfTrack, MuxError> {
    let mut indexed = scan_ivf_video_file_sync(path, MuxRawCodec::Vp8, spec)?;
    let first_sample = read_indexed_sample_sync(
        path,
        indexed.first_sample_span,
        spec,
        "IVF VP8 sample payload is truncated",
    )?;
    annotate_vp8_sync_samples_sync(path, spec, &mut indexed.samples)?;
    let sample_entry_box = build_vp8_sample_entry_box(
        indexed.width,
        indexed.height,
        &first_sample,
        &indexed.samples,
        indexed.timescale,
        spec,
    )?;
    Ok(ParsedIvfTrack {
        width: indexed.width,
        height: indexed.height,
        timescale: indexed.timescale,
        sample_entry_box,
        samples: indexed.samples,
    })
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_vp8_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedIvfTrack, MuxError> {
    let mut indexed = scan_ivf_video_file_async(path, MuxRawCodec::Vp8, spec).await?;
    let first_sample = read_indexed_sample_async(
        path,
        indexed.first_sample_span,
        spec,
        "IVF VP8 sample payload is truncated",
    )
    .await?;
    annotate_vp8_sync_samples_async(path, spec, &mut indexed.samples).await?;
    let sample_entry_box = build_vp8_sample_entry_box(
        indexed.width,
        indexed.height,
        &first_sample,
        &indexed.samples,
        indexed.timescale,
        spec,
    )?;
    Ok(ParsedIvfTrack {
        width: indexed.width,
        height: indexed.height,
        timescale: indexed.timescale,
        sample_entry_box,
        samples: indexed.samples,
    })
}

fn build_vp8_sample_entry_box(
    width: u16,
    height: u16,
    sample: &[u8],
    samples: &[StagedSample],
    timescale: u32,
    spec: &str,
) -> Result<Vec<u8>, MuxError> {
    let config = parse_vp8_config(width, height, sample, spec)?;
    let btrt = build_btrt_from_sample_sizes(
        samples
            .iter()
            .map(|sample| (sample.data_size, sample.duration)),
        timescale,
    )?;
    let child_boxes = vec![
        super::super::mp4::encode_typed_box(&config, &[])?,
        super::super::mp4::encode_typed_box(&btrt, &[])?,
    ];
    build_visual_sample_entry_box_with_compressor_name(
        crate::FourCc::from_bytes(*b"vp08"),
        width,
        height,
        VP8_COMPRESSOR_NAME,
        &child_boxes,
    )
}

fn annotate_vp8_sync_samples_sync(
    path: &Path,
    spec: &str,
    samples: &mut [StagedSample],
) -> Result<(), MuxError> {
    for sample in samples {
        sample.is_sync_sample = read_vp8_sync_flag_sync(path, *sample, spec)?;
    }
    Ok(())
}

#[cfg(feature = "async")]
async fn annotate_vp8_sync_samples_async(
    path: &Path,
    spec: &str,
    samples: &mut [StagedSample],
) -> Result<(), MuxError> {
    for sample in samples {
        sample.is_sync_sample = read_vp8_sync_flag_async(path, *sample, spec).await?;
    }
    Ok(())
}

fn read_vp8_sync_flag_sync(
    path: &Path,
    sample: StagedSample,
    spec: &str,
) -> Result<bool, MuxError> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(sample.data_offset))?;
    let mut sample_bytes = vec![
        0_u8;
        usize::try_from(sample.data_size)
            .map_err(|_| MuxError::LayoutOverflow("IVF VP8 sample size"))?
    ];
    file.read_exact(&mut sample_bytes).map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            unsupported(spec, "IVF VP8 sample payload is truncated")
        } else {
            MuxError::Io(error)
        }
    })?;
    vp8_sample_is_keyframe(&sample_bytes, spec)
}

#[cfg(feature = "async")]
async fn read_vp8_sync_flag_async(
    path: &Path,
    sample: StagedSample,
    spec: &str,
) -> Result<bool, MuxError> {
    let mut file = TokioFile::open(path).await?;
    file.seek(SeekFrom::Start(sample.data_offset)).await?;
    let mut sample_bytes = vec![
        0_u8;
        usize::try_from(sample.data_size)
            .map_err(|_| MuxError::LayoutOverflow("IVF VP8 sample size"))?
    ];
    file.read_exact(&mut sample_bytes).await.map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            unsupported(spec, "IVF VP8 sample payload is truncated")
        } else {
            MuxError::Io(error)
        }
    })?;
    vp8_sample_is_keyframe(&sample_bytes, spec)
}

fn vp8_sample_is_keyframe(sample: &[u8], spec: &str) -> Result<bool, MuxError> {
    if sample.is_empty() {
        return Err(unsupported(spec, "IVF VP8 sample payload was empty"));
    }
    Ok(sample[0] & 0x80 != 0)
}

fn parse_vp8_config(
    width: u16,
    height: u16,
    sample: &[u8],
    spec: &str,
) -> Result<VpCodecConfiguration, MuxError> {
    if sample.len() < 10 {
        return Err(unsupported(
            spec,
            "VP8 keyframe payload is truncated before the frame header",
        ));
    }

    let frame_tag =
        u32::from(sample[0]) | (u32::from(sample[1]) << 8) | (u32::from(sample[2]) << 16);
    let frame_type = frame_tag & 1;
    if frame_type != 0 {
        return Err(unsupported(
            spec,
            "VP8 direct input must start with one keyframe so container dimensions can be validated",
        ));
    }
    let _profile = u8::try_from((frame_tag >> 1) & 0x07)
        .map_err(|_| MuxError::LayoutOverflow("VP8 profile"))?;
    if sample[3..6] != VP8_SYNC_CODE {
        return Err(unsupported(
            spec,
            "VP8 keyframe payload did not contain the expected sync code",
        ));
    }
    let parsed_width = u16::from_le_bytes([sample[6], sample[7]]) & 0x3FFF;
    let parsed_height = u16::from_le_bytes([sample[8], sample[9]]) & 0x3FFF;
    if parsed_width == 0 || parsed_height == 0 {
        return Err(unsupported(
            spec,
            "VP8 keyframe declared zero width or height",
        ));
    }
    if parsed_width != width || parsed_height != height {
        return Err(unsupported(
            spec,
            "VP8 keyframe dimensions did not match the IVF header dimensions",
        ));
    }

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
    Ok(config)
}

fn unsupported(spec: &str, message: &str) -> MuxError {
    MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: message.to_string(),
    }
}
