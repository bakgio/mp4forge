use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;
#[cfg(feature = "async")]
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::FourCc;
use crate::boxes::av1::AV1CodecConfiguration;
use crate::boxes::iso14496_12::Colr;

use super::super::import::build_visual_sample_entry_box;
use super::super::{MuxError, MuxRawCodec};
#[cfg(feature = "async")]
use super::ivf_common::read_indexed_sample_async;
#[cfg(feature = "async")]
use super::ivf_common::scan_ivf_video_file_async;
use super::ivf_common::{
    IndexedIvfSample, IndexedIvfTrack, ParsedIvfTrack, read_indexed_sample_sync,
    scan_ivf_video_file_sync,
};

const AV1_COLOUR_TYPE_NCLX: FourCc = FourCc::from_bytes(*b"nclx");
const OBU_SEQUENCE_HEADER: u8 = 1;
const OBU_TEMPORAL_DELIMITER: u8 = 2;

pub(in crate::mux) fn scan_av1_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedIvfTrack, MuxError> {
    let mut indexed = scan_ivf_video_file_sync(path, MuxRawCodec::Av1, spec)?;
    normalize_av1_sample_spans_sync(path, &mut indexed, spec)?;
    let first_sample = read_indexed_sample_sync(
        path,
        indexed.first_sample_span,
        spec,
        "IVF AV1 sample payload is truncated",
    )?;
    let (config, colr) = parse_av1_sample_entry_details(&first_sample, spec)?;
    let child_boxes = vec![
        super::super::mp4::encode_typed_box(&config, &[])?,
        super::super::mp4::encode_typed_box(&colr, &[])?,
    ];
    let sample_entry_box = build_visual_sample_entry_box(
        indexed.sample_entry_type,
        indexed.width,
        indexed.height,
        &child_boxes,
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
pub(in crate::mux) async fn scan_av1_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedIvfTrack, MuxError> {
    let mut indexed = scan_ivf_video_file_async(path, MuxRawCodec::Av1, spec).await?;
    normalize_av1_sample_spans_async(path, &mut indexed, spec).await?;
    let first_sample = read_indexed_sample_async(
        path,
        indexed.first_sample_span,
        spec,
        "IVF AV1 sample payload is truncated",
    )
    .await?;
    let (config, colr) = parse_av1_sample_entry_details(&first_sample, spec)?;
    let child_boxes = vec![
        super::super::mp4::encode_typed_box(&config, &[])?,
        super::super::mp4::encode_typed_box(&colr, &[])?,
    ];
    let sample_entry_box = build_visual_sample_entry_box(
        indexed.sample_entry_type,
        indexed.width,
        indexed.height,
        &child_boxes,
    )?;
    Ok(ParsedIvfTrack {
        width: indexed.width,
        height: indexed.height,
        timescale: indexed.timescale,
        sample_entry_box,
        samples: indexed.samples,
    })
}

fn parse_av1_sample_entry_details(
    sample: &[u8],
    spec: &str,
) -> Result<(AV1CodecConfiguration, Colr), MuxError> {
    let (config_obus, sequence_header) = find_av1_sequence_header_obu(sample, spec)?;
    let mut config = AV1CodecConfiguration {
        seq_profile: sequence_header.seq_profile,
        seq_level_idx_0: sequence_header.seq_level_idx_0,
        seq_tier_0: sequence_header.seq_tier_0,
        high_bitdepth: u8::from(sequence_header.high_bitdepth),
        twelve_bit: u8::from(sequence_header.twelve_bit),
        monochrome: u8::from(sequence_header.monochrome),
        chroma_subsampling_x: sequence_header.chroma_subsampling_x,
        chroma_subsampling_y: sequence_header.chroma_subsampling_y,
        chroma_sample_position: sequence_header.chroma_sample_position,
        initial_presentation_delay_present: u8::from(
            sequence_header
                .initial_presentation_delay_minus_one
                .is_some(),
        ),
        initial_presentation_delay_minus_one: sequence_header
            .initial_presentation_delay_minus_one
            .unwrap_or(0),
        config_obus,
    };
    if config.initial_presentation_delay_present == 0 {
        config.initial_presentation_delay_minus_one = 0;
    }
    let colr = Colr {
        colour_type: AV1_COLOUR_TYPE_NCLX,
        colour_primaries: sequence_header.colour_primaries,
        transfer_characteristics: sequence_header.transfer_characteristics,
        matrix_coefficients: sequence_header.matrix_coefficients,
        full_range_flag: sequence_header.full_range_flag,
        reserved: 0,
        profile: Vec::new(),
        unknown: Vec::new(),
    };
    Ok((config, colr))
}

fn normalize_av1_sample_spans_sync(
    path: &Path,
    indexed: &mut IndexedIvfTrack,
    spec: &str,
) -> Result<(), MuxError> {
    let mut file = File::open(path)?;
    for sample in &mut indexed.samples {
        let trim = scan_leading_temporal_delimiter_bytes_sync(
            &mut file,
            sample.data_offset,
            sample.data_size,
            spec,
        )?;
        apply_av1_sample_trim(sample, trim, spec)?;
    }
    let trim = scan_leading_temporal_delimiter_bytes_sync(
        &mut file,
        indexed.first_sample_span.data_offset,
        indexed.first_sample_span.data_size,
        spec,
    )?;
    apply_av1_indexed_sample_trim(&mut indexed.first_sample_span, trim, spec)?;
    Ok(())
}

#[cfg(feature = "async")]
async fn normalize_av1_sample_spans_async(
    path: &Path,
    indexed: &mut IndexedIvfTrack,
    spec: &str,
) -> Result<(), MuxError> {
    let mut file = TokioFile::open(path).await?;
    for sample in &mut indexed.samples {
        let trim = scan_leading_temporal_delimiter_bytes_async(
            &mut file,
            sample.data_offset,
            sample.data_size,
            spec,
        )
        .await?;
        apply_av1_sample_trim(sample, trim, spec)?;
    }
    let trim = scan_leading_temporal_delimiter_bytes_async(
        &mut file,
        indexed.first_sample_span.data_offset,
        indexed.first_sample_span.data_size,
        spec,
    )
    .await?;
    apply_av1_indexed_sample_trim(&mut indexed.first_sample_span, trim, spec)?;
    Ok(())
}

fn apply_av1_sample_trim(
    sample: &mut crate::mux::import::StagedSample,
    trim: u32,
    spec: &str,
) -> Result<(), MuxError> {
    if trim == 0 {
        return Ok(());
    }
    if trim >= sample.data_size {
        return Err(unsupported(
            spec,
            "AV1 sample payload only contained temporal-delimiter OBUs",
        ));
    }
    sample.data_offset = sample
        .data_offset
        .checked_add(u64::from(trim))
        .ok_or(MuxError::LayoutOverflow("AV1 sample trim offset"))?;
    sample.data_size -= trim;
    Ok(())
}

fn apply_av1_indexed_sample_trim(
    sample: &mut IndexedIvfSample,
    trim: u32,
    spec: &str,
) -> Result<(), MuxError> {
    if trim == 0 {
        return Ok(());
    }
    if trim >= sample.data_size {
        return Err(unsupported(
            spec,
            "AV1 sample payload only contained temporal-delimiter OBUs",
        ));
    }
    sample.data_offset = sample
        .data_offset
        .checked_add(u64::from(trim))
        .ok_or(MuxError::LayoutOverflow("AV1 sample trim offset"))?;
    sample.data_size -= trim;
    Ok(())
}

fn scan_leading_temporal_delimiter_bytes_sync(
    file: &mut File,
    sample_offset: u64,
    sample_size: u32,
    spec: &str,
) -> Result<u32, MuxError> {
    let mut trimmed = 0_u32;
    loop {
        let remaining = sample_size
            .checked_sub(trimmed)
            .ok_or(MuxError::LayoutOverflow("AV1 sample trim remainder"))?;
        if remaining == 0 {
            return Err(unsupported(
                spec,
                "AV1 sample payload only contained temporal-delimiter OBUs",
            ));
        }
        file.seek(SeekFrom::Start(
            sample_offset
                .checked_add(u64::from(trimmed))
                .ok_or(MuxError::LayoutOverflow("AV1 sample trim seek"))?,
        ))?;
        let prefix_len = usize::try_from(remaining.min(8))
            .map_err(|_| MuxError::LayoutOverflow("AV1 prefix read size"))?;
        let mut prefix = vec![0_u8; prefix_len];
        file.read_exact(&mut prefix).map_err(|error| {
            map_temporal_delimiter_io_error(
                error,
                spec,
                "AV1 sample payload is truncated while reading a temporal-delimiter prefix",
            )
        })?;
        match leading_temporal_delimiter_len(&prefix, spec, sample_offset + u64::from(trimmed))? {
            Some(length) => {
                trimmed = trimmed
                    .checked_add(length)
                    .ok_or(MuxError::LayoutOverflow("AV1 sample trim length"))?;
            }
            None => return Ok(trimmed),
        }
    }
}

#[cfg(feature = "async")]
async fn scan_leading_temporal_delimiter_bytes_async(
    file: &mut TokioFile,
    sample_offset: u64,
    sample_size: u32,
    spec: &str,
) -> Result<u32, MuxError> {
    let mut trimmed = 0_u32;
    loop {
        let remaining = sample_size
            .checked_sub(trimmed)
            .ok_or(MuxError::LayoutOverflow("AV1 sample trim remainder"))?;
        if remaining == 0 {
            return Err(unsupported(
                spec,
                "AV1 sample payload only contained temporal-delimiter OBUs",
            ));
        }
        file.seek(SeekFrom::Start(
            sample_offset
                .checked_add(u64::from(trimmed))
                .ok_or(MuxError::LayoutOverflow("AV1 sample trim seek"))?,
        ))
        .await?;
        let prefix_len = usize::try_from(remaining.min(8))
            .map_err(|_| MuxError::LayoutOverflow("AV1 prefix read size"))?;
        let mut prefix = vec![0_u8; prefix_len];
        file.read_exact(&mut prefix).await.map_err(|error| {
            map_temporal_delimiter_io_error(
                error,
                spec,
                "AV1 sample payload is truncated while reading a temporal-delimiter prefix",
            )
        })?;
        match leading_temporal_delimiter_len(&prefix, spec, sample_offset + u64::from(trimmed))? {
            Some(length) => {
                trimmed = trimmed
                    .checked_add(length)
                    .ok_or(MuxError::LayoutOverflow("AV1 sample trim length"))?;
            }
            None => return Ok(trimmed),
        }
    }
}

fn leading_temporal_delimiter_len(
    prefix: &[u8],
    spec: &str,
    offset: u64,
) -> Result<Option<u32>, MuxError> {
    let mut cursor = 0usize;
    let header = *prefix.get(cursor).ok_or_else(|| {
        unsupported(
            spec,
            "AV1 temporal-delimiter prefix is truncated before the OBU header",
        )
    })?;
    if header >> 7 != 0 {
        return Err(unsupported(
            spec,
            "AV1 OBU header used a non-zero forbidden bit",
        ));
    }
    let obu_type = (header >> 3) & 0x0F;
    if obu_type != OBU_TEMPORAL_DELIMITER {
        return Ok(None);
    }
    cursor += 1;
    let extension_flag = (header >> 2) & 0x01 != 0;
    let has_size_field = (header >> 1) & 0x01 != 0;
    if header & 0x01 != 0 {
        return Err(unsupported(
            spec,
            "AV1 OBU header used a non-zero reserved bit",
        ));
    }
    if extension_flag {
        if prefix.get(cursor).is_none() {
            return Err(unsupported(
                spec,
                "AV1 temporal-delimiter OBU extension header is truncated",
            ));
        }
        cursor += 1;
    }
    if !has_size_field {
        return Err(unsupported(
            spec,
            "AV1 temporal-delimiter OBUs without explicit size fields are not supported",
        ));
    }
    let (obu_size, leb_bytes) = read_leb128_from_slice(
        prefix.get(cursor..).unwrap_or_default(),
        spec,
        "AV1 temporal-delimiter OBU size",
        offset + u64::try_from(cursor).unwrap_or(u64::MAX),
    )?;
    if obu_size != 0 {
        return Err(unsupported(
            spec,
            "AV1 temporal-delimiter OBU payloads must have zero length",
        ));
    }
    cursor = cursor
        .checked_add(leb_bytes)
        .ok_or(MuxError::LayoutOverflow(
            "AV1 temporal-delimiter size field",
        ))?;
    Ok(Some(u32::try_from(cursor).map_err(|_| {
        MuxError::LayoutOverflow("AV1 temporal delimiter")
    })?))
}

fn map_temporal_delimiter_io_error(
    error: std::io::Error,
    spec: &str,
    truncated_message: &'static str,
) -> MuxError {
    if error.kind() == std::io::ErrorKind::UnexpectedEof {
        unsupported(spec, truncated_message)
    } else {
        MuxError::Io(error)
    }
}

fn find_av1_sequence_header_obu(
    sample: &[u8],
    spec: &str,
) -> Result<(Vec<u8>, ParsedAv1SequenceHeader), MuxError> {
    let mut offset = 0usize;
    while offset < sample.len() {
        let start = offset;
        let header = *sample
            .get(offset)
            .ok_or_else(|| unsupported(spec, "AV1 OBU header is truncated"))?;
        offset += 1;
        if header >> 7 != 0 {
            return Err(unsupported(
                spec,
                "AV1 OBU header used a non-zero forbidden bit",
            ));
        }
        let obu_type = (header >> 3) & 0x0F;
        let extension_flag = (header >> 2) & 0x01 != 0;
        let has_size_field = (header >> 1) & 0x01 != 0;
        if header & 0x01 != 0 {
            return Err(unsupported(
                spec,
                "AV1 OBU header used a non-zero reserved bit",
            ));
        }
        if extension_flag {
            if sample.get(offset).is_none() {
                return Err(unsupported(spec, "AV1 OBU extension header is truncated"));
            }
            offset += 1;
        }
        if !has_size_field {
            return Err(unsupported(
                spec,
                "AV1 sequence OBUs without explicit size fields are not supported",
            ));
        }
        let (obu_size, leb_bytes) = read_leb128_from_slice(
            sample.get(offset..).unwrap_or_default(),
            spec,
            "AV1 OBU size",
            u64::try_from(offset).unwrap_or(u64::MAX),
        )?;
        offset = offset
            .checked_add(leb_bytes)
            .ok_or(MuxError::LayoutOverflow("AV1 OBU header size"))?;
        let payload_end = offset
            .checked_add(
                usize::try_from(obu_size).map_err(|_| MuxError::LayoutOverflow("AV1 OBU size"))?,
            )
            .ok_or(MuxError::LayoutOverflow("AV1 OBU size"))?;
        if payload_end > sample.len() {
            return Err(unsupported(
                spec,
                "AV1 OBU payload overruns the sample payload",
            ));
        }
        if obu_type == OBU_SEQUENCE_HEADER {
            let obu_bytes = sample[start..payload_end].to_vec();
            let sequence_header = parse_av1_sequence_header(&sample[offset..payload_end], spec)?;
            return Ok((obu_bytes, sequence_header));
        }
        offset = payload_end;
    }
    Err(unsupported(
        spec,
        "AV1 input did not contain a sequence-header OBU in its first sample",
    ))
}

#[derive(Clone, Copy)]
struct ParsedAv1SequenceHeader {
    seq_profile: u8,
    seq_level_idx_0: u8,
    seq_tier_0: u8,
    high_bitdepth: bool,
    twelve_bit: bool,
    monochrome: bool,
    chroma_subsampling_x: u8,
    chroma_subsampling_y: u8,
    chroma_sample_position: u8,
    initial_presentation_delay_minus_one: Option<u8>,
    colour_primaries: u16,
    transfer_characteristics: u16,
    matrix_coefficients: u16,
    full_range_flag: bool,
}

fn parse_av1_sequence_header(
    bytes: &[u8],
    spec: &str,
) -> Result<ParsedAv1SequenceHeader, MuxError> {
    let mut bits = BitCursor::new(bytes);
    let seq_profile = bits.read_bits_u8(3, spec, "AV1 seq_profile")?;
    let still_picture = bits.read_bit(spec, "AV1 still_picture")?;
    let reduced_still_picture_header = bits.read_bit(spec, "AV1 reduced_still_picture_header")?;
    if reduced_still_picture_header && !still_picture {
        return Err(unsupported(
            spec,
            "AV1 reduced still-picture headers must also set the still-picture flag",
        ));
    }

    let mut seq_tier_0 = 0;
    let mut initial_presentation_delay_minus_one = None;
    let seq_level_idx_0;
    let decoder_model_info = if reduced_still_picture_header {
        seq_level_idx_0 = bits.read_bits_u8(5, spec, "AV1 seq_level_idx_0")?;
        None
    } else {
        let timing_info_present_flag = bits.read_bit(spec, "AV1 timing_info_present_flag")?;
        let decoder_model_info_present_flag = if timing_info_present_flag {
            bits.read_bit(spec, "AV1 decoder_model_info_present_flag")?
        } else {
            false
        };
        let decoder_model_info = if timing_info_present_flag && decoder_model_info_present_flag {
            skip_timing_info_and_decoder_model(&mut bits, spec)?
        } else if timing_info_present_flag {
            skip_timing_info_only(&mut bits, spec)?;
            None
        } else {
            None
        };
        let initial_display_delay_present_flag =
            bits.read_bit(spec, "AV1 initial_display_delay_present_flag")?;
        let operating_points_cnt_minus_1 =
            bits.read_bits_u8(5, spec, "AV1 operating_points_cnt_minus_1")?;
        let mut seq_level = 0;
        let mut seq_tier = 0;
        let mut initial_delay = None;
        for index in 0..=operating_points_cnt_minus_1 {
            bits.skip_bits(12, spec, "AV1 operating_point_idc")?;
            let level = bits.read_bits_u8(5, spec, "AV1 seq_level_idx")?;
            let tier = if level > 7 {
                u8::from(bits.read_bit(spec, "AV1 seq_tier")?)
            } else {
                0
            };
            if let Some(info) = decoder_model_info
                && bits.read_bit(spec, "AV1 decoder_model_present_for_this_op")?
            {
                bits.skip_bits(
                    usize::from(info.buffer_delay_length_minus_one) + 1,
                    spec,
                    "AV1 decoder_buffer_delay",
                )?;
                bits.skip_bits(
                    usize::from(info.buffer_delay_length_minus_one) + 1,
                    spec,
                    "AV1 encoder_buffer_delay",
                )?;
                bits.skip_bits(1, spec, "AV1 low_delay_mode_flag")?;
            }
            let op_delay = if initial_display_delay_present_flag
                && bits.read_bit(spec, "AV1 initial_display_delay_present_for_this_op")?
            {
                Some(bits.read_bits_u8(4, spec, "AV1 initial_display_delay_minus_one")?)
            } else {
                None
            };
            if index == 0 {
                seq_level = level;
                seq_tier = tier;
                initial_delay = op_delay;
            }
        }
        seq_level_idx_0 = seq_level;
        seq_tier_0 = seq_tier;
        initial_presentation_delay_minus_one = initial_delay;
        decoder_model_info
    };

    let frame_width_bits_minus_1 = bits.read_bits_u8(4, spec, "AV1 frame_width_bits_minus_1")?;
    let frame_height_bits_minus_1 = bits.read_bits_u8(4, spec, "AV1 frame_height_bits_minus_1")?;
    bits.skip_bits(
        usize::from(frame_width_bits_minus_1) + 1,
        spec,
        "AV1 max_frame_width_minus_1",
    )?;
    bits.skip_bits(
        usize::from(frame_height_bits_minus_1) + 1,
        spec,
        "AV1 max_frame_height_minus_one",
    )?;
    if !reduced_still_picture_header && bits.read_bit(spec, "AV1 frame_id_numbers_present_flag")? {
        bits.skip_bits(4, spec, "AV1 delta_frame_id_length_minus_2")?;
        bits.skip_bits(3, spec, "AV1 additional_frame_id_length_minus_1")?;
    }

    bits.skip_bits(1, spec, "AV1 use_128x128_superblock")?;
    bits.skip_bits(1, spec, "AV1 enable_filter_intra")?;
    bits.skip_bits(1, spec, "AV1 enable_intra_edge_filter")?;
    if !reduced_still_picture_header {
        bits.skip_bits(1, spec, "AV1 enable_interintra_compound")?;
        bits.skip_bits(1, spec, "AV1 enable_masked_compound")?;
        bits.skip_bits(1, spec, "AV1 enable_warped_motion")?;
        let enable_dual_filter = bits.read_bit(spec, "AV1 enable_dual_filter")?;
        let enable_order_hint = bits.read_bit(spec, "AV1 enable_order_hint")?;
        if enable_order_hint {
            bits.skip_bits(1, spec, "AV1 enable_jnt_comp")?;
            bits.skip_bits(1, spec, "AV1 enable_ref_frame_mvs")?;
        }
        let seq_choose_screen_content_tools =
            bits.read_bit(spec, "AV1 seq_choose_screen_content_tools")?;
        let seq_force_screen_content_tools = if seq_choose_screen_content_tools {
            None
        } else {
            Some(bits.read_bit(spec, "AV1 seq_force_screen_content_tools")?)
        };
        if seq_force_screen_content_tools == Some(true) {
            let seq_choose_integer_mv = bits.read_bit(spec, "AV1 seq_choose_integer_mv")?;
            if !seq_choose_integer_mv {
                bits.skip_bits(1, spec, "AV1 seq_force_integer_mv")?;
            }
        }
        if enable_order_hint || enable_dual_filter {
            let _ = decoder_model_info;
        }
        if enable_order_hint {
            bits.skip_bits(3, spec, "AV1 order_hint_bits_minus_1")?;
        }
    }
    bits.skip_bits(1, spec, "AV1 enable_superres")?;
    bits.skip_bits(1, spec, "AV1 enable_cdef")?;
    bits.skip_bits(1, spec, "AV1 enable_restoration")?;

    let color_info = parse_av1_color_config(&mut bits, seq_profile, spec)?;
    bits.skip_bits(1, spec, "AV1 film_grain_params_present")?;

    Ok(ParsedAv1SequenceHeader {
        seq_profile,
        seq_level_idx_0,
        seq_tier_0,
        high_bitdepth: color_info.high_bitdepth,
        twelve_bit: color_info.twelve_bit,
        monochrome: color_info.monochrome,
        chroma_subsampling_x: color_info.chroma_subsampling_x,
        chroma_subsampling_y: color_info.chroma_subsampling_y,
        chroma_sample_position: color_info.chroma_sample_position,
        initial_presentation_delay_minus_one,
        colour_primaries: color_info.colour_primaries,
        transfer_characteristics: color_info.transfer_characteristics,
        matrix_coefficients: color_info.matrix_coefficients,
        full_range_flag: color_info.full_range_flag,
    })
}

#[derive(Clone, Copy)]
struct Av1DecoderModelInfo {
    buffer_delay_length_minus_one: u8,
}

fn skip_timing_info_and_decoder_model(
    bits: &mut BitCursor<'_>,
    spec: &str,
) -> Result<Option<Av1DecoderModelInfo>, MuxError> {
    skip_timing_info_only(bits, spec)?;
    let buffer_delay_length_minus_one =
        bits.read_bits_u8(5, spec, "AV1 buffer_delay_length_minus_one")?;
    bits.skip_bits(32, spec, "AV1 num_units_in_decoding_tick")?;
    bits.skip_bits(5, spec, "AV1 buffer_removal_time_length_minus_1")?;
    bits.skip_bits(5, spec, "AV1 frame_presentation_time_length_minus_1")?;
    Ok(Some(Av1DecoderModelInfo {
        buffer_delay_length_minus_one,
    }))
}

fn skip_timing_info_only(bits: &mut BitCursor<'_>, spec: &str) -> Result<(), MuxError> {
    bits.skip_bits(32, spec, "AV1 num_units_in_display_tick")?;
    bits.skip_bits(32, spec, "AV1 time_scale")?;
    if bits.read_bit(spec, "AV1 equal_picture_interval")? {
        let _ = read_uvlc(bits, spec, "AV1 num_ticks_per_picture_minus_1")?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct ParsedAv1ColorInfo {
    high_bitdepth: bool,
    twelve_bit: bool,
    monochrome: bool,
    chroma_subsampling_x: u8,
    chroma_subsampling_y: u8,
    chroma_sample_position: u8,
    colour_primaries: u16,
    transfer_characteristics: u16,
    matrix_coefficients: u16,
    full_range_flag: bool,
}

fn parse_av1_color_config(
    bits: &mut BitCursor<'_>,
    seq_profile: u8,
    spec: &str,
) -> Result<ParsedAv1ColorInfo, MuxError> {
    let high_bitdepth = bits.read_bit(spec, "AV1 high_bitdepth")?;
    let twelve_bit = seq_profile == 2 && high_bitdepth && bits.read_bit(spec, "AV1 twelve_bit")?;
    let monochrome = if seq_profile == 1 {
        false
    } else {
        bits.read_bit(spec, "AV1 monochrome")?
    };
    let mut colour_primaries = 2_u16;
    let mut transfer_characteristics = 2_u16;
    let mut matrix_coefficients = 2_u16;
    if bits.read_bit(spec, "AV1 color_description_present_flag")? {
        colour_primaries = u16::from(bits.read_bits_u8(8, spec, "AV1 colour_primaries")?);
        transfer_characteristics =
            u16::from(bits.read_bits_u8(8, spec, "AV1 transfer_characteristics")?);
        matrix_coefficients = u16::from(bits.read_bits_u8(8, spec, "AV1 matrix_coefficients")?);
    }

    let full_range_flag;
    let (chroma_subsampling_x, chroma_subsampling_y, chroma_sample_position) = if monochrome {
        full_range_flag = bits.read_bit(spec, "AV1 color_range")?;
        (1, 1, 0)
    } else if colour_primaries == 1 && transfer_characteristics == 13 && matrix_coefficients == 0 {
        full_range_flag = true;
        (0, 0, 0)
    } else {
        full_range_flag = bits.read_bit(spec, "AV1 color_range")?;
        let chroma = if seq_profile == 0 {
            (1, 1)
        } else if seq_profile == 1 {
            (0, 0)
        } else if twelve_bit {
            let chroma_x = u8::from(bits.read_bit(spec, "AV1 chroma_subsampling_x")?);
            let chroma_y = if chroma_x == 1 {
                u8::from(bits.read_bit(spec, "AV1 chroma_subsampling_y")?)
            } else {
                0
            };
            (chroma_x, chroma_y)
        } else {
            (1, 0)
        };
        let chroma_sample_position = if chroma.0 == 1 && chroma.1 == 1 {
            bits.read_bits_u8(2, spec, "AV1 chroma_sample_position")?
        } else {
            0
        };
        bits.skip_bits(1, spec, "AV1 separate_uv_delta_q")?;
        return Ok(ParsedAv1ColorInfo {
            high_bitdepth,
            twelve_bit,
            monochrome,
            chroma_subsampling_x: chroma.0,
            chroma_subsampling_y: chroma.1,
            chroma_sample_position,
            colour_primaries,
            transfer_characteristics,
            matrix_coefficients,
            full_range_flag,
        });
    };
    bits.skip_bits(1, spec, "AV1 separate_uv_delta_q")?;
    Ok(ParsedAv1ColorInfo {
        high_bitdepth,
        twelve_bit,
        monochrome,
        chroma_subsampling_x,
        chroma_subsampling_y,
        chroma_sample_position,
        colour_primaries,
        transfer_characteristics,
        matrix_coefficients,
        full_range_flag,
    })
}

fn read_leb128_from_slice(
    bytes: &[u8],
    spec: &str,
    field_name: &str,
    offset: u64,
) -> Result<(u64, usize), MuxError> {
    let mut value = 0_u64;
    let mut shift = 0_u32;
    for (index, byte) in bytes.iter().copied().enumerate() {
        value |= u64::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, index + 1));
        }
        shift += 7;
        if shift >= 63 {
            break;
        }
    }
    Err(unsupported(
        spec,
        &format!(
            "{field_name} at byte offset {offset} used an unterminated or unsupported leb128 value"
        ),
    ))
}

fn read_uvlc(bits: &mut BitCursor<'_>, spec: &str, label: &str) -> Result<u32, MuxError> {
    let mut leading_zeroes = 0usize;
    while !bits.read_bit(spec, label)? {
        leading_zeroes += 1;
    }
    if leading_zeroes == 0 {
        return Ok(0);
    }
    let remainder = bits.read_bits_u32(leading_zeroes, spec, label)?;
    Ok((1_u32 << leading_zeroes) - 1 + remainder)
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

    fn read_bit(&mut self, spec: &str, label: &str) -> Result<bool, MuxError> {
        Ok(self.read_bits_u32(1, spec, label)? != 0)
    }

    fn read_bits_u8(&mut self, width: usize, spec: &str, label: &str) -> Result<u8, MuxError> {
        u8::try_from(self.read_bits_u32(width, spec, label)?)
            .map_err(|_| MuxError::LayoutOverflow("AV1 bit width conversion"))
    }

    fn read_bits_u32(&mut self, width: usize, spec: &str, label: &str) -> Result<u32, MuxError> {
        let end = self
            .bit_offset
            .checked_add(width)
            .ok_or(MuxError::LayoutOverflow("AV1 bit reader position"))?;
        if end > self.data.len() * 8 {
            return Err(unsupported(
                spec,
                &format!("{label} is truncated in the AV1 sequence header"),
            ));
        }

        let mut value = 0_u32;
        for _ in 0..width {
            let byte = self.data[self.bit_offset / 8];
            let shift = 7 - (self.bit_offset % 8);
            value = (value << 1) | u32::from((byte >> shift) & 1);
            self.bit_offset += 1;
        }
        Ok(value)
    }

    fn skip_bits(&mut self, width: usize, spec: &str, label: &str) -> Result<(), MuxError> {
        let _ = self.read_bits_u32(width, spec, label)?;
        Ok(())
    }
}

fn unsupported(spec: &str, message: &str) -> MuxError {
    MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: message.to_string(),
    }
}
