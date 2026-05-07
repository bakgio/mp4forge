use std::fs::File;
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use crate::FourCc;
use crate::boxes::threegpp::Damr;

use super::super::MuxError;
#[cfg(feature = "async")]
use super::super::import::read_exact_at_async;
use super::super::import::{
    StagedSample, build_generic_audio_sample_entry_box, read_exact_at_sync,
};

const AMR_MAGIC: &[u8; 6] = b"#!AMR\n";
const AMR_WB_MAGIC: &[u8; 9] = b"#!AMR-WB\n";
const SAMPLE_ENTRY_SAMR: FourCc = FourCc::from_bytes(*b"samr");
const SAMPLE_ENTRY_SAWB: FourCc = FourCc::from_bytes(*b"sawb");
const AMR_SAMPLE_RATE: u32 = 8_000;
const AMR_WB_SAMPLE_RATE: u32 = 16_000;
const AMR_SAMPLES_PER_FRAME: u32 = 160;
const AMR_WB_SAMPLES_PER_FRAME: u32 = 320;
const AMR_FRAME_SIZES: [u8; 16] = [12, 13, 15, 17, 19, 20, 26, 31, 5, 0, 0, 0, 0, 0, 0, 0];
const AMR_WB_FRAME_SIZES: [u8; 16] = [17, 23, 32, 36, 40, 46, 50, 58, 60, 5, 5, 0, 0, 0, 0, 0];
const THREE_GPP_VENDOR_CODE: u32 = 0x4750_4143;

pub(in crate::mux) struct ParsedAmrTrack {
    pub(in crate::mux) sample_rate: u32,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) samples: Vec<StagedSample>,
    pub(in crate::mux) handler_label: &'static str,
}

pub(in crate::mux) fn scan_amr_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedAmrTrack, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    parse_amr_stream_sync(
        &mut file,
        file_size,
        spec,
        AmrStreamKind {
            magic: AMR_MAGIC,
            sample_entry_type: SAMPLE_ENTRY_SAMR,
            sample_rate: AMR_SAMPLE_RATE,
            sample_duration: AMR_SAMPLES_PER_FRAME,
            frame_sizes: &AMR_FRAME_SIZES,
            handler_label: "amr",
            format_label: "AMR",
        },
    )
}

pub(in crate::mux) fn scan_amr_wb_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedAmrTrack, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    parse_amr_stream_sync(
        &mut file,
        file_size,
        spec,
        AmrStreamKind {
            magic: AMR_WB_MAGIC,
            sample_entry_type: SAMPLE_ENTRY_SAWB,
            sample_rate: AMR_WB_SAMPLE_RATE,
            sample_duration: AMR_WB_SAMPLES_PER_FRAME,
            frame_sizes: &AMR_WB_FRAME_SIZES,
            handler_label: "amr-wb",
            format_label: "AMR-WB",
        },
    )
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_amr_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedAmrTrack, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    parse_amr_stream_async(
        &mut file,
        file_size,
        spec,
        AmrStreamKind {
            magic: AMR_MAGIC,
            sample_entry_type: SAMPLE_ENTRY_SAMR,
            sample_rate: AMR_SAMPLE_RATE,
            sample_duration: AMR_SAMPLES_PER_FRAME,
            frame_sizes: &AMR_FRAME_SIZES,
            handler_label: "amr",
            format_label: "AMR",
        },
    )
    .await
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_amr_wb_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedAmrTrack, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    parse_amr_stream_async(
        &mut file,
        file_size,
        spec,
        AmrStreamKind {
            magic: AMR_WB_MAGIC,
            sample_entry_type: SAMPLE_ENTRY_SAWB,
            sample_rate: AMR_WB_SAMPLE_RATE,
            sample_duration: AMR_WB_SAMPLES_PER_FRAME,
            frame_sizes: &AMR_WB_FRAME_SIZES,
            handler_label: "amr-wb",
            format_label: "AMR-WB",
        },
    )
    .await
}

#[derive(Clone, Copy)]
struct AmrStreamKind {
    magic: &'static [u8],
    sample_entry_type: FourCc,
    sample_rate: u32,
    sample_duration: u32,
    frame_sizes: &'static [u8; 16],
    handler_label: &'static str,
    format_label: &'static str,
}

fn parse_amr_stream_sync(
    file: &mut File,
    file_size: u64,
    spec: &str,
    stream: AmrStreamKind,
) -> Result<ParsedAmrTrack, MuxError> {
    validate_amr_magic_sync(file, file_size, spec, stream)?;

    let mut offset = u64::try_from(stream.magic.len())
        .map_err(|_| MuxError::LayoutOverflow("AMR magic size"))?;
    let mut samples = Vec::new();
    let mut mode_set = 0_u16;
    while offset < file_size {
        let mut toc = [0_u8; 1];
        read_exact_at_sync(file, offset, &mut toc, spec, "truncated AMR frame header")?;
        if toc[0] == 0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "{} input carried a zero TOC byte at byte offset {}",
                    stream.format_label, offset
                ),
            });
        }
        let frame_type = usize::from((toc[0] >> 3) & 0x0F);
        let payload_size = u32::from(stream.frame_sizes[frame_type]);
        if payload_size == 0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "{} input carried unsupported frame type {} at byte offset {}",
                    stream.format_label, frame_type, offset
                ),
            });
        }
        let frame_size = payload_size
            .checked_add(1)
            .ok_or(MuxError::LayoutOverflow("AMR frame size"))?;
        if offset
            .checked_add(u64::from(frame_size))
            .is_none_or(|end| end > file_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "truncated {} frame at byte offset {}",
                    stream.format_label, offset
                ),
            });
        }
        mode_set |= 1_u16 << frame_type;
        samples.push(StagedSample {
            data_offset: offset,
            data_size: frame_size,
            duration: stream.sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(u64::from(frame_size))
            .ok_or(MuxError::LayoutOverflow("AMR frame offset"))?;
    }

    if samples.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "{} input contained no codec frames after the magic header",
                stream.format_label
            ),
        });
    }

    Ok(ParsedAmrTrack {
        sample_rate: stream.sample_rate,
        sample_entry_box: build_amr_sample_entry_box(stream, mode_set)?,
        samples,
        handler_label: stream.handler_label,
    })
}

#[cfg(feature = "async")]
async fn parse_amr_stream_async(
    file: &mut TokioFile,
    file_size: u64,
    spec: &str,
    stream: AmrStreamKind,
) -> Result<ParsedAmrTrack, MuxError> {
    validate_amr_magic_async(file, file_size, spec, stream).await?;

    let mut offset = u64::try_from(stream.magic.len())
        .map_err(|_| MuxError::LayoutOverflow("AMR magic size"))?;
    let mut samples = Vec::new();
    let mut mode_set = 0_u16;
    while offset < file_size {
        let mut toc = [0_u8; 1];
        read_exact_at_async(file, offset, &mut toc, spec, "truncated AMR frame header").await?;
        if toc[0] == 0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "{} input carried a zero TOC byte at byte offset {}",
                    stream.format_label, offset
                ),
            });
        }
        let frame_type = usize::from((toc[0] >> 3) & 0x0F);
        let payload_size = u32::from(stream.frame_sizes[frame_type]);
        if payload_size == 0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "{} input carried unsupported frame type {} at byte offset {}",
                    stream.format_label, frame_type, offset
                ),
            });
        }
        let frame_size = payload_size
            .checked_add(1)
            .ok_or(MuxError::LayoutOverflow("AMR frame size"))?;
        if offset
            .checked_add(u64::from(frame_size))
            .is_none_or(|end| end > file_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "truncated {} frame at byte offset {}",
                    stream.format_label, offset
                ),
            });
        }
        mode_set |= 1_u16 << frame_type;
        samples.push(StagedSample {
            data_offset: offset,
            data_size: frame_size,
            duration: stream.sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(u64::from(frame_size))
            .ok_or(MuxError::LayoutOverflow("AMR frame offset"))?;
    }

    if samples.is_empty() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "{} input contained no codec frames after the magic header",
                stream.format_label
            ),
        });
    }

    Ok(ParsedAmrTrack {
        sample_rate: stream.sample_rate,
        sample_entry_box: build_amr_sample_entry_box(stream, mode_set)?,
        samples,
        handler_label: stream.handler_label,
    })
}

fn validate_amr_magic_sync(
    file: &mut File,
    file_size: u64,
    spec: &str,
    stream: AmrStreamKind,
) -> Result<(), MuxError> {
    if file_size < u64::try_from(stream.magic.len()).unwrap() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "{} input is truncated before the magic header",
                stream.format_label
            ),
        });
    }
    let mut magic = vec![0_u8; stream.magic.len()];
    read_exact_at_sync(file, 0, &mut magic, spec, "truncated AMR magic header")?;
    if magic != stream.magic {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "{} input did not start with the expected magic header",
                stream.format_label
            ),
        });
    }
    Ok(())
}

#[cfg(feature = "async")]
async fn validate_amr_magic_async(
    file: &mut TokioFile,
    file_size: u64,
    spec: &str,
    stream: AmrStreamKind,
) -> Result<(), MuxError> {
    if file_size < u64::try_from(stream.magic.len()).unwrap() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "{} input is truncated before the magic header",
                stream.format_label
            ),
        });
    }
    let mut magic = vec![0_u8; stream.magic.len()];
    read_exact_at_async(file, 0, &mut magic, spec, "truncated AMR magic header").await?;
    if magic != stream.magic {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "{} input did not start with the expected magic header",
                stream.format_label
            ),
        });
    }
    Ok(())
}

fn build_amr_sample_entry_box(stream: AmrStreamKind, mode_set: u16) -> Result<Vec<u8>, MuxError> {
    let damr_box = super::super::mp4::encode_typed_box(
        &Damr {
            vendor: THREE_GPP_VENDOR_CODE,
            decoder_version: 0,
            mode_set,
            mode_change_period: 0,
            frames_per_sample: 1,
        },
        &[],
    )?;
    build_generic_audio_sample_entry_box(
        stream.sample_entry_type,
        stream.sample_rate,
        1,
        16,
        &[damr_box],
    )
}
