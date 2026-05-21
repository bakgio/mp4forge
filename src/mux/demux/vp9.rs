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

const VP9_FRAME_MARKER: u32 = 0b10;
const VP9_KEYFRAME_SYNC: u32 = 0x49_83_42;
const VP9_COMPRESSOR_NAME: &[u8] = b"VPC Coding";

pub(in crate::mux) fn scan_vp9_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedIvfTrack, MuxError> {
    let mut indexed = scan_ivf_video_file_sync(path, MuxRawCodec::Vp9, spec)?;
    let first_sample = read_indexed_sample_sync(
        path,
        indexed.first_sample_span,
        spec,
        "IVF VP9 sample payload is truncated",
    )?;
    annotate_vp9_sync_samples_sync(path, spec, &mut indexed.samples)?;
    let sample_entry_box = build_vp9_sample_entry_box(
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
pub(in crate::mux) async fn scan_vp9_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedIvfTrack, MuxError> {
    let mut indexed = scan_ivf_video_file_async(path, MuxRawCodec::Vp9, spec).await?;
    let first_sample = read_indexed_sample_async(
        path,
        indexed.first_sample_span,
        spec,
        "IVF VP9 sample payload is truncated",
    )
    .await?;
    annotate_vp9_sync_samples_async(path, spec, &mut indexed.samples).await?;
    let sample_entry_box = build_vp9_sample_entry_box(
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

fn build_vp9_sample_entry_box(
    width: u16,
    height: u16,
    sample: &[u8],
    samples: &[StagedSample],
    timescale: u32,
    spec: &str,
) -> Result<Vec<u8>, MuxError> {
    let config = parse_vp9_config(width, height, sample, spec)?;
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
        crate::FourCc::from_bytes(*b"vp09"),
        width,
        height,
        VP9_COMPRESSOR_NAME,
        &child_boxes,
    )
}

fn annotate_vp9_sync_samples_sync(
    path: &Path,
    spec: &str,
    samples: &mut [StagedSample],
) -> Result<(), MuxError> {
    for sample in samples {
        sample.is_sync_sample = read_vp9_sync_flag_sync(path, *sample, spec)?;
    }
    Ok(())
}

#[cfg(feature = "async")]
async fn annotate_vp9_sync_samples_async(
    path: &Path,
    spec: &str,
    samples: &mut [StagedSample],
) -> Result<(), MuxError> {
    for sample in samples {
        sample.is_sync_sample = read_vp9_sync_flag_async(path, *sample, spec).await?;
    }
    Ok(())
}

fn read_vp9_sync_flag_sync(
    path: &Path,
    sample: StagedSample,
    spec: &str,
) -> Result<bool, MuxError> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(sample.data_offset))?;
    let mut sample_bytes = vec![
        0_u8;
        usize::try_from(sample.data_size)
            .map_err(|_| MuxError::LayoutOverflow("IVF VP9 sample size"))?
    ];
    file.read_exact(&mut sample_bytes).map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            unsupported(spec, "IVF VP9 sample payload is truncated")
        } else {
            MuxError::Io(error)
        }
    })?;
    Ok(vp9_sample_is_sync(&sample_bytes))
}

#[cfg(feature = "async")]
async fn read_vp9_sync_flag_async(
    path: &Path,
    sample: StagedSample,
    spec: &str,
) -> Result<bool, MuxError> {
    let mut file = TokioFile::open(path).await?;
    file.seek(SeekFrom::Start(sample.data_offset)).await?;
    let mut sample_bytes = vec![
        0_u8;
        usize::try_from(sample.data_size)
            .map_err(|_| MuxError::LayoutOverflow("IVF VP9 sample size"))?
    ];
    file.read_exact(&mut sample_bytes).await.map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            unsupported(spec, "IVF VP9 sample payload is truncated")
        } else {
            MuxError::Io(error)
        }
    })?;
    Ok(vp9_sample_is_sync(&sample_bytes))
}

fn vp9_sample_is_sync(sample: &[u8]) -> bool {
    let mut bits = BitCursor::new(sample);
    if bits.read_bits_u8(2).map(u32::from) != Some(VP9_FRAME_MARKER) {
        return false;
    }
    let profile_low = bits.read_bit().unwrap_or(false);
    let profile_high = bits.read_bit().unwrap_or(false);
    let profile = u8::from(profile_low) | (u8::from(profile_high) << 1);
    if profile == 3 {
        let _reserved_profile_bit = bits.read_bit().unwrap_or(false);
    }
    if bits.read_bit().unwrap_or(false) {
        return false;
    }
    let frame_type = bits.read_bit().unwrap_or(true);
    let _show_frame = bits.read_bit().unwrap_or(false);
    let _error_resilient_mode = bits.read_bit().unwrap_or(false);
    !frame_type
}

fn parse_vp9_config(
    width: u16,
    height: u16,
    sample: &[u8],
    spec: &str,
) -> Result<VpCodecConfiguration, MuxError> {
    let mut bits = BitCursor::new(sample);
    let frame_marker = match bits.read_bits_u8(2) {
        Some(value) => value,
        None => return Ok(default_vp9_config(0)),
    };
    if u32::from(frame_marker) != VP9_FRAME_MARKER {
        return Err(unsupported(
            spec,
            "VP9 frame did not start with the expected frame marker",
        ));
    }

    let profile_low = bits.read_bit().unwrap_or(false);
    let profile_high = bits.read_bit().unwrap_or(false);
    let mut profile = u8::from(profile_low) | (u8::from(profile_high) << 1);
    if profile == 3 {
        profile += u8::from(bits.read_bit().unwrap_or(false));
    }
    if bits.read_bit().unwrap_or(false) {
        return Ok(default_vp9_config(profile));
    }

    let frame_type = bits.read_bit().unwrap_or(false);
    let _show_frame = bits.read_bit().unwrap_or(false);
    let _error_resilient_mode = bits.read_bit().unwrap_or(false);
    if frame_type {
        return Ok(default_vp9_config(profile));
    }
    let sync_code = match bits.read_bits_u32(24) {
        Some(value) => value,
        None => return Ok(default_vp9_config(profile)),
    };
    if sync_code != VP9_KEYFRAME_SYNC {
        return Err(unsupported(
            spec,
            "VP9 keyframe did not contain the expected sync code",
        ));
    }

    let mut bit_depth = 8_u8;
    if profile >= 2 {
        bit_depth = if bits.read_bit().unwrap_or(false) {
            12
        } else {
            10
        };
    }
    let color_space = match bits.read_bits_u8(3) {
        Some(value) => value,
        None => return Ok(default_vp9_config(profile)),
    };
    let (
        video_full_range_flag,
        chroma_subsampling,
        colour_primaries,
        transfer_characteristics,
        matrix_coefficients,
    ) = if color_space == 7 {
        if profile == 1 || profile == 3 {
            let _reserved_zero = bits.read_bit().unwrap_or(false);
        }
        (1_u8, 3_u8, 1_u8, 13_u8, 0_u8)
    } else {
        let video_full_range_flag = u8::from(bits.read_bit().unwrap_or(false));
        let chroma_subsampling = if profile != 1 && profile != 3 {
            0_u8
        } else {
            let subsampling_x = u8::from(bits.read_bit().unwrap_or(false));
            let subsampling_y = u8::from(bits.read_bit().unwrap_or(false));
            let _reserved_zero = bits.read_bit().unwrap_or(false);
            ((subsampling_x << 1) | subsampling_y) + 1
        };
        (video_full_range_flag, chroma_subsampling, 5_u8, 5_u8, 6_u8)
    };

    let parsed_width = match bits.read_bits_u16(16) {
        Some(value) => value.saturating_add(1),
        None => return Ok(default_vp9_config(profile)),
    };
    let parsed_height = match bits.read_bits_u16(16) {
        Some(value) => value.saturating_add(1),
        None => return Ok(default_vp9_config(profile)),
    };
    if parsed_width != width || parsed_height != height {
        return Err(unsupported(
            spec,
            "VP9 frame dimensions did not match the IVF header dimensions",
        ));
    }

    let mut config = VpCodecConfiguration::default();
    config.set_version(1);
    config.profile = profile;
    config.level = 0;
    config.bit_depth = bit_depth;
    config.chroma_subsampling = chroma_subsampling;
    config.video_full_range_flag = video_full_range_flag;
    config.colour_primaries = colour_primaries;
    config.transfer_characteristics = transfer_characteristics;
    config.matrix_coefficients = matrix_coefficients;
    config.codec_initialization_data_size = 0;
    config.codec_initialization_data = Vec::new();
    Ok(config)
}

fn default_vp9_config(profile: u8) -> VpCodecConfiguration {
    let mut config = VpCodecConfiguration::default();
    config.set_version(1);
    config.profile = profile;
    config.level = 0;
    config.bit_depth = 0;
    config.chroma_subsampling = 0;
    config.video_full_range_flag = 0;
    config.colour_primaries = 0;
    config.transfer_characteristics = 0;
    config.matrix_coefficients = 0;
    config.codec_initialization_data_size = 0;
    config.codec_initialization_data = Vec::new();
    config
}

struct BitCursor<'a> {
    data: &'a [u8],
    bit_offset: usize,
}

impl<'a> BitCursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            bit_offset: 0,
        }
    }

    fn read_bit(&mut self) -> Option<bool> {
        self.read_bits_u32(1).map(|value| value != 0)
    }

    fn read_bits_u8(&mut self, width: usize) -> Option<u8> {
        u8::try_from(self.read_bits_u32(width)?).ok()
    }

    fn read_bits_u16(&mut self, width: usize) -> Option<u16> {
        u16::try_from(self.read_bits_u32(width)?).ok()
    }

    fn read_bits_u32(&mut self, width: usize) -> Option<u32> {
        let end = self.bit_offset.checked_add(width)?;
        if end > self.data.len() * 8 {
            return None;
        }
        let mut value = 0_u32;
        for _ in 0..width {
            let byte = self.data[self.bit_offset / 8];
            let shift = 7 - (self.bit_offset % 8);
            value = (value << 1) | u32::from((byte >> shift) & 1);
            self.bit_offset += 1;
        }
        Some(value)
    }
}

fn unsupported(spec: &str, message: &str) -> MuxError {
    MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: message.to_string(),
    }
}
