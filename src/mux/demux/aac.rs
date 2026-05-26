use std::fs::File;
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use crate::FourCc;
use crate::boxes::AnyTypeBox;
use crate::boxes::iso14496_12::{AudioSampleEntry, SampleEntry};
use crate::boxes::iso14496_14::{
    DECODER_CONFIG_DESCRIPTOR_TAG, DECODER_SPECIFIC_INFO_TAG, DecoderConfigDescriptor, Descriptor,
    ES_DESCRIPTOR_TAG, EsDescriptor, Esds, SL_CONFIG_DESCRIPTOR_TAG,
};

use super::super::MuxError;
#[cfg(feature = "async")]
use super::super::import::read_exact_at_async;
use super::super::import::{SegmentedMuxSourceSegment, StagedSample, read_exact_at_sync};
#[cfg(feature = "async")]
use super::container_common::read_segmented_bytes_async;
use super::container_common::read_segmented_bytes_sync;

pub(in crate::mux) struct ParsedAdtsTrack {
    pub(in crate::mux) sample_rate: u32,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) samples: Vec<StagedSample>,
}

pub(in crate::mux) fn scan_adts_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedAdtsTrack, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut expected = None::<(u8, u8, u32, u16)>;
    while offset < file_size {
        if file_size - offset < 7 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated ADTS header".to_string(),
            });
        }
        let mut header = [0_u8; 7];
        read_exact_at_sync(
            &mut file,
            offset,
            &mut header,
            spec,
            "truncated ADTS header",
        )?;
        if header[0] != 0xFF || header[1] & 0xF0 != 0xF0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("missing ADTS sync word at byte offset {offset}"),
            });
        }

        let protection_absent = header[1] & 0x01 != 0;
        let header_length = if protection_absent { 7 } else { 9 };
        if file_size - offset < u64::from(header_length as u32) {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated ADTS header".to_string(),
            });
        }
        let profile = ((header[2] >> 6) & 0x03) + 1;
        let sampling_frequency_index = (header[2] >> 2) & 0x0F;
        let channel_configuration = u16::from((header[2] & 0x01) << 2 | ((header[3] >> 6) & 0x03));
        let sample_rate = adts_sample_rate(sampling_frequency_index).ok_or_else(|| {
            MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "unsupported ADTS sampling-frequency index {sampling_frequency_index}"
                ),
            }
        })?;
        let frame_length = usize::from(
            ((u16::from(header[3] & 0x03)) << 11)
                | (u16::from(header[4]) << 3)
                | u16::from(header[5] >> 5),
        );
        let raw_blocks = u32::from(header[6] & 0x03) + 1;
        if frame_length < header_length
            || offset
                .checked_add(u64::try_from(frame_length).unwrap_or(u64::MAX))
                .is_none_or(|end| end > file_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated ADTS frame at byte offset {offset}"),
            });
        }

        let descriptor = (
            profile,
            sampling_frequency_index,
            sample_rate,
            channel_configuration,
        );
        if let Some(expected) = expected {
            if expected != descriptor {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message:
                        "AAC frames changed profile, sample rate, or channel layout mid-stream"
                            .to_string(),
                });
            }
        } else {
            expected = Some(descriptor);
        }

        let payload_size = frame_length - header_length;
        samples.push(StagedSample {
            data_offset: offset + u64::from(header_length as u32),
            data_size: u32::try_from(payload_size)
                .map_err(|_| MuxError::LayoutOverflow("AAC frame size"))?,
            duration: 1024 * raw_blocks,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(
                u64::try_from(frame_length)
                    .map_err(|_| MuxError::LayoutOverflow("AAC frame size"))?,
            )
            .ok_or(MuxError::LayoutOverflow("AAC frame offset"))?;
    }
    let (audio_object_type, sampling_frequency_index, sample_rate, channel_configuration) =
        expected.ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AAC input contained no ADTS frames".to_string(),
        })?;
    Ok(ParsedAdtsTrack {
        sample_rate,
        sample_entry_box: build_aac_sample_entry_box(
            audio_object_type,
            sampling_frequency_index,
            channel_configuration,
            sample_rate,
            &samples,
        )?,
        samples,
    })
}

pub(in crate::mux) fn scan_adts_segmented_sync(
    file: &mut File,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<ParsedAdtsTrack, MuxError> {
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut expected = None::<(u8, u8, u32, u16)>;
    while offset < total_size {
        if total_size - offset < 7 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated ADTS header".to_string(),
            });
        }
        let mut header = [0_u8; 7];
        read_segmented_bytes_sync(
            file,
            segments,
            total_size,
            offset,
            &mut header,
            spec,
            "truncated ADTS header",
        )?;
        if header[0] != 0xFF || header[1] & 0xF0 != 0xF0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("missing ADTS sync word at logical byte offset {offset}"),
            });
        }

        let protection_absent = header[1] & 0x01 != 0;
        let header_length = if protection_absent { 7 } else { 9 };
        if total_size - offset < u64::from(header_length as u32) {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated ADTS header".to_string(),
            });
        }
        let profile = ((header[2] >> 6) & 0x03) + 1;
        let sampling_frequency_index = (header[2] >> 2) & 0x0F;
        let channel_configuration = u16::from((header[2] & 0x01) << 2 | ((header[3] >> 6) & 0x03));
        let sample_rate = adts_sample_rate(sampling_frequency_index).ok_or_else(|| {
            MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "unsupported ADTS sampling-frequency index {sampling_frequency_index}"
                ),
            }
        })?;
        let frame_length = usize::from(
            ((u16::from(header[3] & 0x03)) << 11)
                | (u16::from(header[4]) << 3)
                | u16::from(header[5] >> 5),
        );
        let raw_blocks = u32::from(header[6] & 0x03) + 1;
        if frame_length < header_length
            || offset
                .checked_add(u64::try_from(frame_length).unwrap_or(u64::MAX))
                .is_none_or(|end| end > total_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated ADTS frame at logical byte offset {offset}"),
            });
        }

        let descriptor = (
            profile,
            sampling_frequency_index,
            sample_rate,
            channel_configuration,
        );
        if let Some(expected) = expected {
            if expected != descriptor {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message:
                        "AAC frames changed profile, sample rate, or channel layout mid-stream"
                            .to_string(),
                });
            }
        } else {
            expected = Some(descriptor);
        }

        let payload_size = frame_length - header_length;
        samples.push(StagedSample {
            data_offset: offset + u64::from(header_length as u32),
            data_size: u32::try_from(payload_size)
                .map_err(|_| MuxError::LayoutOverflow("AAC frame size"))?,
            duration: 1024 * raw_blocks,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(
                u64::try_from(frame_length)
                    .map_err(|_| MuxError::LayoutOverflow("AAC frame size"))?,
            )
            .ok_or(MuxError::LayoutOverflow("AAC frame offset"))?;
    }
    let (audio_object_type, sampling_frequency_index, sample_rate, channel_configuration) =
        expected.ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AAC input contained no ADTS frames".to_string(),
        })?;
    Ok(ParsedAdtsTrack {
        sample_rate,
        sample_entry_box: build_aac_sample_entry_box(
            audio_object_type,
            sampling_frequency_index,
            channel_configuration,
            sample_rate,
            &samples,
        )?,
        samples,
    })
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_adts_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedAdtsTrack, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut expected = None::<(u8, u8, u32, u16)>;
    while offset < file_size {
        if file_size - offset < 7 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated ADTS header".to_string(),
            });
        }
        let mut header = [0_u8; 7];
        read_exact_at_async(
            &mut file,
            offset,
            &mut header,
            spec,
            "truncated ADTS header",
        )
        .await?;
        if header[0] != 0xFF || header[1] & 0xF0 != 0xF0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("missing ADTS sync word at byte offset {offset}"),
            });
        }

        let protection_absent = header[1] & 0x01 != 0;
        let header_length = if protection_absent { 7 } else { 9 };
        if file_size - offset < u64::from(header_length as u32) {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated ADTS header".to_string(),
            });
        }
        let profile = ((header[2] >> 6) & 0x03) + 1;
        let sampling_frequency_index = (header[2] >> 2) & 0x0F;
        let channel_configuration = u16::from((header[2] & 0x01) << 2 | ((header[3] >> 6) & 0x03));
        let sample_rate = adts_sample_rate(sampling_frequency_index).ok_or_else(|| {
            MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "unsupported ADTS sampling-frequency index {sampling_frequency_index}"
                ),
            }
        })?;
        let frame_length = usize::from(
            ((u16::from(header[3] & 0x03)) << 11)
                | (u16::from(header[4]) << 3)
                | u16::from(header[5] >> 5),
        );
        let raw_blocks = u32::from(header[6] & 0x03) + 1;
        if frame_length < header_length
            || offset
                .checked_add(u64::try_from(frame_length).unwrap_or(u64::MAX))
                .is_none_or(|end| end > file_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated ADTS frame at byte offset {offset}"),
            });
        }

        let descriptor = (
            profile,
            sampling_frequency_index,
            sample_rate,
            channel_configuration,
        );
        if let Some(expected) = expected {
            if expected != descriptor {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message:
                        "AAC frames changed profile, sample rate, or channel layout mid-stream"
                            .to_string(),
                });
            }
        } else {
            expected = Some(descriptor);
        }

        let payload_size = frame_length - header_length;
        samples.push(StagedSample {
            data_offset: offset + u64::from(header_length as u32),
            data_size: u32::try_from(payload_size)
                .map_err(|_| MuxError::LayoutOverflow("AAC frame size"))?,
            duration: 1024 * raw_blocks,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(
                u64::try_from(frame_length)
                    .map_err(|_| MuxError::LayoutOverflow("AAC frame size"))?,
            )
            .ok_or(MuxError::LayoutOverflow("AAC frame offset"))?;
    }
    let (audio_object_type, sampling_frequency_index, sample_rate, channel_configuration) =
        expected.ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AAC input contained no ADTS frames".to_string(),
        })?;
    Ok(ParsedAdtsTrack {
        sample_rate,
        sample_entry_box: build_aac_sample_entry_box(
            audio_object_type,
            sampling_frequency_index,
            channel_configuration,
            sample_rate,
            &samples,
        )?,
        samples,
    })
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_adts_segmented_async(
    file: &mut TokioFile,
    segments: &[SegmentedMuxSourceSegment],
    total_size: u64,
    spec: &str,
) -> Result<ParsedAdtsTrack, MuxError> {
    let mut offset = 0_u64;
    let mut samples = Vec::new();
    let mut expected = None::<(u8, u8, u32, u16)>;
    while offset < total_size {
        if total_size - offset < 7 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated ADTS header".to_string(),
            });
        }
        let mut header = [0_u8; 7];
        read_segmented_bytes_async(
            file,
            segments,
            total_size,
            offset,
            &mut header,
            spec,
            "truncated ADTS header",
        )
        .await?;
        if header[0] != 0xFF || header[1] & 0xF0 != 0xF0 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("missing ADTS sync word at logical byte offset {offset}"),
            });
        }

        let protection_absent = header[1] & 0x01 != 0;
        let header_length = if protection_absent { 7 } else { 9 };
        if total_size - offset < u64::from(header_length as u32) {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "truncated ADTS header".to_string(),
            });
        }
        let profile = ((header[2] >> 6) & 0x03) + 1;
        let sampling_frequency_index = (header[2] >> 2) & 0x0F;
        let channel_configuration = u16::from((header[2] & 0x01) << 2 | ((header[3] >> 6) & 0x03));
        let sample_rate = adts_sample_rate(sampling_frequency_index).ok_or_else(|| {
            MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "unsupported ADTS sampling-frequency index {sampling_frequency_index}"
                ),
            }
        })?;
        let frame_length = usize::from(
            ((u16::from(header[3] & 0x03)) << 11)
                | (u16::from(header[4]) << 3)
                | u16::from(header[5] >> 5),
        );
        let raw_blocks = u32::from(header[6] & 0x03) + 1;
        if frame_length < header_length
            || offset
                .checked_add(u64::try_from(frame_length).unwrap_or(u64::MAX))
                .is_none_or(|end| end > total_size)
        {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("truncated ADTS frame at logical byte offset {offset}"),
            });
        }

        let descriptor = (
            profile,
            sampling_frequency_index,
            sample_rate,
            channel_configuration,
        );
        if let Some(expected) = expected {
            if expected != descriptor {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message:
                        "AAC frames changed profile, sample rate, or channel layout mid-stream"
                            .to_string(),
                });
            }
        } else {
            expected = Some(descriptor);
        }

        let payload_size = frame_length - header_length;
        samples.push(StagedSample {
            data_offset: offset + u64::from(header_length as u32),
            data_size: u32::try_from(payload_size)
                .map_err(|_| MuxError::LayoutOverflow("AAC frame size"))?,
            duration: 1024 * raw_blocks,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(
                u64::try_from(frame_length)
                    .map_err(|_| MuxError::LayoutOverflow("AAC frame size"))?,
            )
            .ok_or(MuxError::LayoutOverflow("AAC frame offset"))?;
    }
    let (audio_object_type, sampling_frequency_index, sample_rate, channel_configuration) =
        expected.ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "AAC input contained no ADTS frames".to_string(),
        })?;
    Ok(ParsedAdtsTrack {
        sample_rate,
        sample_entry_box: build_aac_sample_entry_box(
            audio_object_type,
            sampling_frequency_index,
            channel_configuration,
            sample_rate,
            &samples,
        )?,
        samples,
    })
}

fn build_aac_sample_entry_box(
    audio_object_type: u8,
    sampling_frequency_index: u8,
    channel_configuration: u16,
    sample_rate: u32,
    samples: &[StagedSample],
) -> Result<Vec<u8>, MuxError> {
    let mut mp4a = AudioSampleEntry::default();
    mp4a.set_box_type(FourCc::from_bytes(*b"mp4a"));
    mp4a.sample_entry = SampleEntry {
        box_type: FourCc::from_bytes(*b"mp4a"),
        data_reference_index: 1,
    };
    mp4a.channel_count = channel_configuration;
    mp4a.sample_size = 16;
    mp4a.sample_rate = sample_rate << 16;

    let mut esds = aac_profile_esds(
        audio_object_type,
        sampling_frequency_index,
        channel_configuration,
        sample_rate,
        samples,
    );
    esds.normalize_descriptor_sizes_for_mux()
        .map_err(|_| MuxError::LayoutOverflow("AAC esds normalization"))?;

    super::super::mp4::encode_typed_box(&mp4a, &super::super::mp4::encode_typed_box(&esds, &[])?)
}

pub(in crate::mux) fn build_aac_lc_sample_entry_box(
    sample_rate: u32,
    channel_configuration: u16,
) -> Result<Vec<u8>, MuxError> {
    let sampling_frequency_index =
        aac_sample_rate_index(sample_rate).ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: format!("AAC {sample_rate} Hz"),
            message: format!(
                "unsupported AAC sample rate {sample_rate} for direct sample-entry signaling"
            ),
        })?;
    build_aac_sample_entry_box(
        2,
        sampling_frequency_index,
        channel_configuration,
        sample_rate,
        &[],
    )
}

const fn adts_sample_rate(index: u8) -> Option<u32> {
    match index {
        0 => Some(96_000),
        1 => Some(88_200),
        2 => Some(64_000),
        3 => Some(48_000),
        4 => Some(44_100),
        5 => Some(32_000),
        6 => Some(24_000),
        7 => Some(22_050),
        8 => Some(16_000),
        9 => Some(12_000),
        10 => Some(11_025),
        11 => Some(8_000),
        12 => Some(7_350),
        _ => None,
    }
}

const fn aac_sample_rate_index(sample_rate: u32) -> Option<u8> {
    match sample_rate {
        96_000 => Some(0),
        88_200 => Some(1),
        64_000 => Some(2),
        48_000 => Some(3),
        44_100 => Some(4),
        32_000 => Some(5),
        24_000 => Some(6),
        22_050 => Some(7),
        16_000 => Some(8),
        12_000 => Some(9),
        11_025 => Some(10),
        8_000 => Some(11),
        7_350 => Some(12),
        _ => None,
    }
}

fn aac_profile_esds(
    audio_object_type: u8,
    sampling_frequency_index: u8,
    channel_configuration: u16,
    sample_rate: u32,
    samples: &[StagedSample],
) -> Esds {
    let audio_specific_config = build_aac_audio_specific_config(
        audio_object_type,
        sampling_frequency_index,
        channel_configuration,
    );
    let buffer_size_db = samples
        .iter()
        .map(|sample| sample.data_size)
        .max()
        .unwrap_or(0);
    let total_payload_bytes = samples
        .iter()
        .fold(0_u64, |total, sample| total + u64::from(sample.data_size));
    let total_payload_bits = total_payload_bytes.saturating_mul(8);
    let total_duration = samples
        .iter()
        .fold(0_u64, |total, sample| total + u64::from(sample.duration));
    let avg_bitrate = total_payload_bytes
        .saturating_mul(8)
        .saturating_mul(u64::from(sample_rate))
        .checked_div(total_duration)
        .unwrap_or(0);
    let max_bitrate = total_payload_bits.max(avg_bitrate);
    let mut esds = Esds::default();
    esds.descriptors = vec![
        Descriptor {
            tag: ES_DESCRIPTOR_TAG,
            es_descriptor: Some(EsDescriptor::default()),
            ..Descriptor::default()
        },
        Descriptor {
            tag: DECODER_CONFIG_DESCRIPTOR_TAG,
            decoder_config_descriptor: Some(DecoderConfigDescriptor {
                object_type_indication: 0x40,
                stream_type: 5,
                buffer_size_db,
                max_bitrate: u32::try_from(max_bitrate).unwrap_or(u32::MAX),
                avg_bitrate: u32::try_from(avg_bitrate).unwrap_or(u32::MAX),
                reserved: true,
                ..DecoderConfigDescriptor::default()
            }),
            ..Descriptor::default()
        },
        Descriptor {
            tag: DECODER_SPECIFIC_INFO_TAG,
            size: audio_specific_config.len() as u32,
            data: audio_specific_config,
            ..Descriptor::default()
        },
        Descriptor {
            tag: SL_CONFIG_DESCRIPTOR_TAG,
            size: 1,
            data: vec![0x02],
            ..Descriptor::default()
        },
    ];
    esds
}

fn build_aac_audio_specific_config(
    audio_object_type: u8,
    sampling_frequency_index: u8,
    channel_configuration: u16,
) -> Vec<u8> {
    let config = ((u16::from(audio_object_type) & 0x1F) << 11)
        | ((u16::from(sampling_frequency_index) & 0x0F) << 7)
        | ((channel_configuration & 0x0F) << 3);
    vec![(config >> 8) as u8, (config & 0xFF) as u8]
}
