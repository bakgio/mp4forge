//! Core ISO BMFF timing and structure boxes.

use std::io::{Cursor, SeekFrom, Write};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::boxes::iso23001_7::{
    Senc, decode_senc_payload, encode_senc_payload, render_senc_samples_display,
};
use crate::boxes::{AnyTypeBox, BoxLookupContext, BoxRegistry};
use crate::codec::{
    ANY_VERSION, CodecBox, CodecError, FieldHooks, FieldTable, FieldValue, FieldValueError,
    FieldValueRead, FieldValueWrite, ImmutableBox, MutableBox, ReadSeek, StringFieldMode,
    read_exact_vec_untrusted, untrusted_prealloc_hint,
};
use crate::header::{BoxInfo, SMALL_HEADER_SIZE};
use crate::{FourCc, codec_field};

const URL_SELF_CONTAINED: u32 = 0x000001;
const URN_SELF_CONTAINED: u32 = 0x000001;
const AUX_INFO_TYPE_PRESENT: u32 = 0x000001;
const SCHEME_URI_PRESENT: u32 = 0x000001;

const COLR_NCLX: FourCc = FourCc::from_bytes(*b"nclx");
const COLR_RICC: FourCc = FourCc::from_bytes(*b"rICC");
const COLR_PROF: FourCc = FourCc::from_bytes(*b"prof");

/// User-type identifier for the spherical-video XML payload carried in `uuid` boxes.
pub const UUID_SPHERICAL_VIDEO_V1: [u8; 16] = [
    0xff, 0xcc, 0x82, 0x63, 0xf8, 0x55, 0x4a, 0x93, 0x88, 0x14, 0x58, 0x7a, 0x02, 0x52, 0x1f, 0xdd,
];
/// User-type identifier for the fragment-absolute-timing payload carried in `uuid` boxes.
pub const UUID_FRAGMENT_ABSOLUTE_TIMING: [u8; 16] = [
    0x6d, 0x1d, 0x9b, 0x05, 0x42, 0xd5, 0x44, 0xe6, 0x80, 0xe2, 0x14, 0x1d, 0xaf, 0xf7, 0x57, 0xb2,
];
/// User-type identifier for the fragment-run table payload carried in `uuid` boxes.
pub const UUID_FRAGMENT_RUN_TABLE: [u8; 16] = [
    0xd4, 0x80, 0x7e, 0xf2, 0xca, 0x39, 0x46, 0x95, 0x8e, 0x54, 0x26, 0xcb, 0x9e, 0x46, 0xa7, 0x9f,
];
/// User-type identifier for the sample-encryption payload carried in `uuid` boxes.
pub const UUID_SAMPLE_ENCRYPTION: [u8; 16] = [
    0xa2, 0x39, 0x4f, 0x52, 0x5a, 0x9b, 0x4f, 0x14, 0xa2, 0x44, 0x6c, 0x42, 0x7c, 0x64, 0x8d, 0xf4,
];

/// `tfhd` flag indicating that `base_data_offset` is present.
pub const TFHD_BASE_DATA_OFFSET_PRESENT: u32 = 0x000001;
/// `tfhd` flag indicating that `sample_description_index` is present.
pub const TFHD_SAMPLE_DESCRIPTION_INDEX_PRESENT: u32 = 0x000002;
/// `tfhd` flag indicating that `default_sample_duration` is present.
pub const TFHD_DEFAULT_SAMPLE_DURATION_PRESENT: u32 = 0x000008;
/// `tfhd` flag indicating that `default_sample_size` is present.
pub const TFHD_DEFAULT_SAMPLE_SIZE_PRESENT: u32 = 0x000010;
/// `tfhd` flag indicating that `default_sample_flags` is present.
pub const TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT: u32 = 0x000020;
/// `tfhd` flag indicating that the fragment duration is empty.
pub const TFHD_DURATION_IS_EMPTY: u32 = 0x010000;
/// `tfhd` flag indicating that the default base is the containing `moof`.
pub const TFHD_DEFAULT_BASE_IS_MOOF: u32 = 0x020000;

/// `trun` flag indicating that `data_offset` is present.
pub const TRUN_DATA_OFFSET_PRESENT: u32 = 0x000001;
/// `trun` flag indicating that `first_sample_flags` is present.
pub const TRUN_FIRST_SAMPLE_FLAGS_PRESENT: u32 = 0x000004;
/// `trun` flag indicating that each entry carries `sample_duration`.
pub const TRUN_SAMPLE_DURATION_PRESENT: u32 = 0x000100;
/// `trun` flag indicating that each entry carries `sample_size`.
pub const TRUN_SAMPLE_SIZE_PRESENT: u32 = 0x000200;
/// `trun` flag indicating that each entry carries `sample_flags`.
pub const TRUN_SAMPLE_FLAGS_PRESENT: u32 = 0x000400;
/// `trun` flag indicating that each entry carries a composition time offset.
pub const TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT: u32 = 0x000800;
/// Known `prft` flags value for timestamps captured at encoder input.
pub const PRFT_TIME_ENCODER_INPUT: u32 = 0x000000;
/// Known `prft` flags value for timestamps captured at encoder output.
pub const PRFT_TIME_ENCODER_OUTPUT: u32 = 0x000001;
/// Known `prft` flags value for timestamps captured when the containing `moof` was finalized.
pub const PRFT_TIME_MOOF_FINALIZED: u32 = 0x000002;
/// Known `prft` flags value for timestamps captured when the containing `moof` was written.
pub const PRFT_TIME_MOOF_WRITTEN: u32 = 0x000004;
/// Known `prft` flags value for timestamps captured at an arbitrary but internally consistent point.
pub const PRFT_TIME_ARBITRARY_CONSISTENT: u32 = 0x000008;
/// Known `prft` flags value for timestamps captured by an external time source.
pub const PRFT_TIME_CAPTURED: u32 = 0x000018;
/// Number of NTP whole seconds between `1900-01-01` and the UNIX epoch.
pub const PRFT_NTP_UNIX_EPOCH_OFFSET_SECONDS: u64 = 2_208_988_800;

const PRFT_NTP_FRACTION_SCALE: u128 = 1u128 << 32;
const NANOS_PER_SECOND: u128 = 1_000_000_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct FullBoxState {
    version: u8,
    flags: u32,
}

fn missing_field(field_name: &'static str) -> FieldValueError {
    FieldValueError::MissingField { field_name }
}

fn unexpected_field(field_name: &'static str, value: FieldValue) -> FieldValueError {
    FieldValueError::UnexpectedType {
        field_name,
        expected: "matching codec field value",
        actual: value.kind_name(),
    }
}

fn invalid_value(field_name: &'static str, reason: &'static str) -> FieldValueError {
    FieldValueError::InvalidValue { field_name, reason }
}

fn u8_from_unsigned(field_name: &'static str, value: u64) -> Result<u8, FieldValueError> {
    u8::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in u8"))
}

fn u16_from_unsigned(field_name: &'static str, value: u64) -> Result<u16, FieldValueError> {
    u16::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in u16"))
}

fn u32_from_unsigned(field_name: &'static str, value: u64) -> Result<u32, FieldValueError> {
    u32::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in u32"))
}

fn i16_from_signed(field_name: &'static str, value: i64) -> Result<i16, FieldValueError> {
    i16::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in i16"))
}

fn i32_from_signed(field_name: &'static str, value: i64) -> Result<i32, FieldValueError> {
    i32::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in i32"))
}

fn i64_from_signed(field_name: &'static str, value: i64) -> Result<i64, FieldValueError> {
    let _ = field_name;
    Ok(value)
}

fn bytes_to_fourcc(field_name: &'static str, bytes: Vec<u8>) -> Result<FourCc, FieldValueError> {
    let array: [u8; 4] = bytes
        .try_into()
        .map_err(|_| invalid_value(field_name, "value must be exactly 4 bytes"))?;
    Ok(FourCc::from_bytes(array))
}

fn bytes_to_zeroes(
    field_name: &'static str,
    bytes: &[u8],
    expected_len: usize,
) -> Result<(), FieldValueError> {
    if bytes.len() != expected_len {
        return Err(invalid_value(
            field_name,
            "value has an unexpected reserved-byte length",
        ));
    }
    if bytes.iter().any(|byte| *byte != 0) {
        return Err(invalid_value(field_name, "reserved bytes must be zero"));
    }
    Ok(())
}

fn bytes_to_fourcc_vec(
    field_name: &'static str,
    bytes: Vec<u8>,
) -> Result<Vec<FourCc>, FieldValueError> {
    parse_fixed_chunks(field_name, &bytes, 4, |chunk| {
        FourCc::from_bytes(chunk.try_into().unwrap())
    })
}

fn bytes_to_track_id_vec(
    field_name: &'static str,
    bytes: Vec<u8>,
) -> Result<Vec<u32>, FieldValueError> {
    parse_fixed_chunks(field_name, &bytes, 4, |chunk| read_u32(chunk, 0))
}

fn fourcc_vec_to_bytes(values: &[FourCc]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(value.as_bytes());
    }
    bytes
}

fn track_id_vec_to_bytes(values: &[u32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    bytes
}

fn parse_fixed_chunks<T, F>(
    field_name: &'static str,
    bytes: &[u8],
    chunk_size: usize,
    parse: F,
) -> Result<Vec<T>, FieldValueError>
where
    F: FnMut(&[u8]) -> T,
{
    let chunks = bytes.chunks_exact(chunk_size);
    if !chunks.remainder().is_empty() {
        return Err(invalid_value(
            field_name,
            "value does not align with entry size",
        ));
    }

    Ok(chunks.map(parse).collect())
}

fn field_len_bytes(count: usize, bytes_per_entry: usize) -> Option<u32> {
    count
        .checked_mul(bytes_per_entry)
        .and_then(|len| u32::try_from(len).ok())
}

fn render_array(values: impl IntoIterator<Item = String>) -> String {
    let items = values.into_iter().collect::<Vec<_>>();
    format!("[{}]", items.join(", "))
}

fn render_hex_bytes(bytes: &[u8]) -> String {
    render_array(bytes.iter().map(|byte| format!("0x{:x}", byte)))
}

fn render_uuid(value: &[u8; 16]) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        value[0],
        value[1],
        value[2],
        value[3],
        value[4],
        value[5],
        value[6],
        value[7],
        value[8],
        value[9],
        value[10],
        value[11],
        value[12],
        value[13],
        value[14],
        value[15],
    )
}

fn bytes_to_uuid(field_name: &'static str, bytes: Vec<u8>) -> Result<[u8; 16], FieldValueError> {
    bytes
        .try_into()
        .map_err(|_| invalid_value(field_name, "value must be exactly 16 bytes"))
}

fn encode_uuid_full_box_header(
    field_name: &'static str,
    version: u8,
    flags: u32,
) -> Result<[u8; 4], FieldValueError> {
    if flags & 0xff00_0000 != 0 {
        return Err(invalid_value(field_name, "flags exceed 24 bits"));
    }

    Ok([
        version,
        ((flags >> 16) & 0xff) as u8,
        ((flags >> 8) & 0xff) as u8,
        (flags & 0xff) as u8,
    ])
}

fn render_uuid_fragment_run_entries(entries: &[UuidFragmentRunEntry]) -> String {
    render_array(entries.iter().map(|entry| {
        format!(
            "{{FragmentAbsoluteTime={} FragmentAbsoluteDuration={}}}",
            entry.fragment_absolute_time, entry.fragment_absolute_duration
        )
    }))
}

fn encode_uuid_fragment_absolute_timing(
    field_name: &'static str,
    timing: &UuidFragmentAbsoluteTiming,
) -> Result<Vec<u8>, FieldValueError> {
    let mut payload = Vec::with_capacity(
        if timing.version == 1 {
            20_usize
        } else {
            12_usize
        }
        .max(4),
    );
    payload.extend_from_slice(&encode_uuid_full_box_header(
        field_name,
        timing.version,
        timing.flags,
    )?);
    match timing.version {
        0 => {
            payload.extend_from_slice(
                &u32::try_from(timing.fragment_absolute_time)
                    .map_err(|_| invalid_value(field_name, "version 0 time does not fit in u32"))?
                    .to_be_bytes(),
            );
            payload.extend_from_slice(
                &u32::try_from(timing.fragment_absolute_duration)
                    .map_err(|_| {
                        invalid_value(field_name, "version 0 duration does not fit in u32")
                    })?
                    .to_be_bytes(),
            );
        }
        1 => {
            payload.extend_from_slice(&timing.fragment_absolute_time.to_be_bytes());
            payload.extend_from_slice(&timing.fragment_absolute_duration.to_be_bytes());
        }
        _ => {
            return Err(invalid_value(
                field_name,
                "fragment timing payload version is not supported",
            ));
        }
    }
    Ok(payload)
}

fn decode_uuid_fragment_absolute_timing(
    field_name: &'static str,
    payload: &[u8],
) -> Result<UuidFragmentAbsoluteTiming, FieldValueError> {
    if payload.len() < 4 {
        return Err(invalid_value(
            field_name,
            "fragment timing payload is truncated",
        ));
    }

    let version = payload[0];
    let flags = u32::from_be_bytes([0, payload[1], payload[2], payload[3]]);
    match version {
        0 => {
            if payload.len() != 12 {
                return Err(invalid_value(
                    field_name,
                    "fragment timing payload length does not match version 0",
                ));
            }
            Ok(UuidFragmentAbsoluteTiming {
                version,
                flags,
                fragment_absolute_time: u64::from(read_u32(payload, 4)),
                fragment_absolute_duration: u64::from(read_u32(payload, 8)),
            })
        }
        1 => {
            if payload.len() != 20 {
                return Err(invalid_value(
                    field_name,
                    "fragment timing payload length does not match version 1",
                ));
            }
            Ok(UuidFragmentAbsoluteTiming {
                version,
                flags,
                fragment_absolute_time: read_u64(payload, 4),
                fragment_absolute_duration: read_u64(payload, 12),
            })
        }
        _ => Err(invalid_value(
            field_name,
            "fragment timing payload version is not supported",
        )),
    }
}

fn encode_uuid_fragment_run_entries(
    field_name: &'static str,
    table: &UuidFragmentRunTable,
) -> Result<Vec<u8>, FieldValueError> {
    if usize::from(table.fragment_count) != table.entries.len() {
        return Err(invalid_value(
            field_name,
            "fragment count does not match the number of entries",
        ));
    }

    let mut payload = Vec::new();
    for entry in &table.entries {
        match table.version {
            0 => {
                payload.extend_from_slice(
                    &u32::try_from(entry.fragment_absolute_time)
                        .map_err(|_| {
                            invalid_value(field_name, "version 0 time does not fit in u32")
                        })?
                        .to_be_bytes(),
                );
                payload.extend_from_slice(
                    &u32::try_from(entry.fragment_absolute_duration)
                        .map_err(|_| {
                            invalid_value(field_name, "version 0 duration does not fit in u32")
                        })?
                        .to_be_bytes(),
                );
            }
            1 => {
                payload.extend_from_slice(&entry.fragment_absolute_time.to_be_bytes());
                payload.extend_from_slice(&entry.fragment_absolute_duration.to_be_bytes());
            }
            _ => {
                return Err(invalid_value(
                    field_name,
                    "fragment run table payload version is not supported",
                ));
            }
        }
    }

    Ok(payload)
}

fn encode_uuid_fragment_run_table(
    field_name: &'static str,
    table: &UuidFragmentRunTable,
) -> Result<Vec<u8>, FieldValueError> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&encode_uuid_full_box_header(
        field_name,
        table.version,
        table.flags,
    )?);
    payload.push(table.fragment_count);
    payload.extend_from_slice(&encode_uuid_fragment_run_entries(field_name, table)?);
    Ok(payload)
}

fn decode_uuid_fragment_run_entries(
    field_name: &'static str,
    version: u8,
    fragment_count: u8,
    payload: &[u8],
) -> Result<Vec<UuidFragmentRunEntry>, FieldValueError> {
    let bytes_per_entry = match version {
        0 => 8_usize,
        1 => 16_usize,
        _ => {
            return Err(invalid_value(
                field_name,
                "fragment run table payload version is not supported",
            ));
        }
    };
    let expected_len = usize::from(fragment_count)
        .checked_mul(bytes_per_entry)
        .ok_or_else(|| invalid_value(field_name, "fragment run table payload is too large"))?;
    if payload.len() != expected_len {
        return Err(invalid_value(
            field_name,
            "fragment run table payload length does not match the fragment count",
        ));
    }

    let mut entries = Vec::with_capacity(untrusted_prealloc_hint(usize::from(fragment_count)));
    let mut offset = 0_usize;
    while offset < payload.len() {
        let (fragment_absolute_time, fragment_absolute_duration) = match version {
            0 => (
                u64::from(read_u32(payload, offset)),
                u64::from(read_u32(payload, offset + 4)),
            ),
            1 => (read_u64(payload, offset), read_u64(payload, offset + 8)),
            _ => unreachable!(),
        };
        entries.push(UuidFragmentRunEntry {
            fragment_absolute_time,
            fragment_absolute_duration,
        });
        offset += bytes_per_entry;
    }
    Ok(entries)
}

fn decode_uuid_fragment_run_table(
    field_name: &'static str,
    payload: &[u8],
) -> Result<UuidFragmentRunTable, FieldValueError> {
    if payload.len() < 5 {
        return Err(invalid_value(
            field_name,
            "fragment run table payload is truncated",
        ));
    }

    let version = payload[0];
    let flags = u32::from_be_bytes([0, payload[1], payload[2], payload[3]]);
    let fragment_count = payload[4];
    let entries =
        decode_uuid_fragment_run_entries(field_name, version, fragment_count, &payload[5..])?;
    Ok(UuidFragmentRunTable {
        version,
        flags,
        fragment_count,
        entries,
    })
}

fn encode_uuid_payload(
    user_type: [u8; 16],
    payload: &UuidPayload,
) -> Result<Vec<u8>, FieldValueError> {
    match payload {
        UuidPayload::Raw(bytes) => Ok(bytes.clone()),
        UuidPayload::SphericalVideoV1(data) => {
            if user_type != UUID_SPHERICAL_VIDEO_V1 {
                return Err(invalid_value(
                    "Payload",
                    "spherical payload requires the spherical UUID user type",
                ));
            }
            Ok(data.xml_data.clone())
        }
        UuidPayload::FragmentAbsoluteTiming(data) => {
            if user_type != UUID_FRAGMENT_ABSOLUTE_TIMING {
                return Err(invalid_value(
                    "Payload",
                    "fragment timing payload requires the fragment-timing UUID user type",
                ));
            }
            encode_uuid_fragment_absolute_timing("Payload", data)
        }
        UuidPayload::FragmentRunTable(data) => {
            if user_type != UUID_FRAGMENT_RUN_TABLE {
                return Err(invalid_value(
                    "Payload",
                    "fragment run table payload requires the fragment-run UUID user type",
                ));
            }
            encode_uuid_fragment_run_table("Payload", data)
        }
        UuidPayload::SampleEncryption(data) => {
            if user_type != UUID_SAMPLE_ENCRYPTION {
                return Err(invalid_value(
                    "Payload",
                    "sample encryption payload requires the sample-encryption UUID user type",
                ));
            }
            encode_senc_payload(data).map_err(|error| match error {
                CodecError::FieldValue(field_error) => field_error,
                CodecError::UnsupportedVersion { .. } => invalid_value(
                    "Payload",
                    "sample encryption payload version is not supported",
                ),
                CodecError::InvalidLength { .. } => invalid_value(
                    "Payload",
                    "sample count does not match the number of sample records",
                ),
                _ => invalid_value("Payload", "sample encryption payload is invalid"),
            })
        }
    }
}

fn decode_uuid_payload(
    user_type: [u8; 16],
    payload: &[u8],
) -> Result<UuidPayload, FieldValueError> {
    if user_type == UUID_SPHERICAL_VIDEO_V1 {
        return Ok(UuidPayload::SphericalVideoV1(SphericalVideoV1Metadata {
            xml_data: payload.to_vec(),
        }));
    }
    if user_type == UUID_FRAGMENT_ABSOLUTE_TIMING {
        return Ok(UuidPayload::FragmentAbsoluteTiming(
            decode_uuid_fragment_absolute_timing("Payload", payload)?,
        ));
    }
    if user_type == UUID_FRAGMENT_RUN_TABLE {
        return Ok(UuidPayload::FragmentRunTable(
            decode_uuid_fragment_run_table("Payload", payload)?,
        ));
    }
    if user_type == UUID_SAMPLE_ENCRYPTION {
        return Ok(UuidPayload::SampleEncryption(
            decode_senc_payload(payload).map_err(|error| match error {
                CodecError::FieldValue(field_error) => field_error,
                CodecError::UnsupportedVersion { .. } => invalid_value(
                    "Payload",
                    "sample encryption payload version is not supported",
                ),
                CodecError::InvalidLength { .. } => invalid_value(
                    "Payload",
                    "sample count does not match the number of sample records",
                ),
                _ => invalid_value("Payload", "sample encryption payload is invalid"),
            })?,
        ));
    }
    Ok(UuidPayload::Raw(payload.to_vec()))
}

fn encoded_loudness_entries_len(
    version: u8,
    entries: &[LoudnessEntry],
) -> Result<u32, FieldValueError> {
    let bytes = encode_loudness_entries("Entries", version, entries)?;
    u32::try_from(bytes.len())
        .map_err(|_| invalid_value("Entries", "encoded payload length does not fit in u32"))
}

fn encode_loudness_entries(
    field_name: &'static str,
    version: u8,
    entries: &[LoudnessEntry],
) -> Result<Vec<u8>, FieldValueError> {
    if version > 1 {
        return Err(invalid_value(
            field_name,
            "unsupported loudness box version",
        ));
    }
    if version == 0 && entries.len() != 1 {
        return Err(invalid_value(
            field_name,
            "version 0 loudness boxes must contain exactly one entry",
        ));
    }
    if version == 1 && entries.len() > 0x3f {
        return Err(invalid_value(
            field_name,
            "entry count does not fit in the loudness count field",
        ));
    }

    let mut bytes = Vec::new();
    if version >= 1 {
        bytes.push(entries.len() as u8);
    }

    for entry in entries {
        if version >= 1 {
            if entry.eq_set_id > 0x3f {
                return Err(invalid_value("EQSetID", "value does not fit in 6 bits"));
            }
            bytes.push(entry.eq_set_id & 0x3f);
        }
        if entry.downmix_id > 0x03ff {
            return Err(invalid_value("DownmixID", "value does not fit in 10 bits"));
        }
        if entry.drc_set_id > 0x3f {
            return Err(invalid_value("DRCSetID", "value does not fit in 6 bits"));
        }
        if entry.bs_sample_peak_level > 0x0fff {
            return Err(invalid_value(
                "BsSamplePeakLevel",
                "value does not fit in 12 bits",
            ));
        }
        if entry.bs_true_peak_level > 0x0fff {
            return Err(invalid_value(
                "BsTruePeakLevel",
                "value does not fit in 12 bits",
            ));
        }
        if entry.measurement_system_for_tp > 0x0f {
            return Err(invalid_value(
                "MeasurementSystemForTP",
                "value does not fit in 4 bits",
            ));
        }
        if entry.reliability_for_tp > 0x0f {
            return Err(invalid_value(
                "ReliabilityForTP",
                "value does not fit in 4 bits",
            ));
        }
        if entry.measurements.len() > usize::from(u8::MAX) {
            return Err(invalid_value(
                "Measurements",
                "entry count does not fit in u8",
            ));
        }

        let downmix_and_drc = (entry.downmix_id << 6) | u16::from(entry.drc_set_id & 0x3f);
        bytes.extend_from_slice(&downmix_and_drc.to_be_bytes());

        let peak_levels = (u32::from(entry.bs_sample_peak_level) << 12)
            | u32::from(entry.bs_true_peak_level & 0x0fff);
        push_uint("PeakLevels", &mut bytes, 3, u64::from(peak_levels))?;
        bytes.push((entry.measurement_system_for_tp << 4) | (entry.reliability_for_tp & 0x0f));
        bytes.push(entry.measurements.len() as u8);

        for measurement in &entry.measurements {
            if measurement.measurement_system > 0x0f {
                return Err(invalid_value(
                    "MeasurementSystem",
                    "value does not fit in 4 bits",
                ));
            }
            if measurement.reliability > 0x0f {
                return Err(invalid_value("Reliability", "value does not fit in 4 bits"));
            }

            bytes.push(measurement.method_definition);
            bytes.push(measurement.method_value);
            bytes.push((measurement.measurement_system << 4) | (measurement.reliability & 0x0f));
        }
    }

    Ok(bytes)
}

fn decode_loudness_entries(
    field_name: &'static str,
    version: u8,
    payload: &[u8],
) -> Result<Vec<LoudnessEntry>, FieldValueError> {
    if version > 1 {
        return Err(invalid_value(
            field_name,
            "unsupported loudness box version",
        ));
    }

    let mut offset = 0_usize;
    let entry_count = if version >= 1 {
        if payload.is_empty() {
            return Err(invalid_value(field_name, "payload is truncated"));
        }
        let info_type = payload[0] >> 6;
        if info_type != 0 {
            return Err(invalid_value(
                field_name,
                "loudness info type is not supported",
            ));
        }
        offset += 1;
        usize::from(payload[0] & 0x3f)
    } else {
        1
    };

    let mut entries = Vec::with_capacity(untrusted_prealloc_hint(entry_count));
    for _ in 0..entry_count {
        let eq_set_id = if version >= 1 {
            if offset >= payload.len() {
                return Err(invalid_value(field_name, "payload is truncated"));
            }
            let value = payload[offset] & 0x3f;
            offset += 1;
            value
        } else {
            0
        };

        if payload.len().saturating_sub(offset) < 7 {
            return Err(invalid_value(field_name, "payload is truncated"));
        }

        let downmix_and_drc = read_u16(payload, offset);
        offset += 2;
        let peak_levels = read_uint(payload, offset, 3) as u32;
        offset += 3;
        let measurement_system_and_reliability_for_tp = payload[offset];
        offset += 1;
        let measurement_count = usize::from(payload[offset]);
        offset += 1;

        let mut measurements = Vec::with_capacity(untrusted_prealloc_hint(measurement_count));
        for _ in 0..measurement_count {
            if payload.len().saturating_sub(offset) < 3 {
                return Err(invalid_value(field_name, "payload is truncated"));
            }
            let method_definition = payload[offset];
            let method_value = payload[offset + 1];
            let measurement_system_and_reliability = payload[offset + 2];
            offset += 3;

            measurements.push(LoudnessMeasurement {
                method_definition,
                method_value,
                measurement_system: measurement_system_and_reliability >> 4,
                reliability: measurement_system_and_reliability & 0x0f,
            });
        }

        entries.push(LoudnessEntry {
            eq_set_id,
            downmix_id: downmix_and_drc >> 6,
            drc_set_id: (downmix_and_drc & 0x3f) as u8,
            bs_sample_peak_level: ((peak_levels >> 12) & 0x0fff) as u16,
            bs_true_peak_level: (peak_levels & 0x0fff) as u16,
            measurement_system_for_tp: measurement_system_and_reliability_for_tp >> 4,
            reliability_for_tp: measurement_system_and_reliability_for_tp & 0x0f,
            measurements,
        });
    }

    if offset != payload.len() {
        return Err(invalid_value(field_name, "payload has trailing bytes"));
    }

    Ok(entries)
}

fn render_loudness_measurements(measurements: &[LoudnessMeasurement]) -> String {
    render_array(measurements.iter().map(|measurement| {
        format!(
            "{{MethodDefinition={} MethodValue={} MeasurementSystem={} Reliability={}}}",
            measurement.method_definition,
            measurement.method_value,
            measurement.measurement_system,
            measurement.reliability,
        )
    }))
}

fn render_loudness_entries(version: u8, entries: &[LoudnessEntry]) -> String {
    render_array(entries.iter().map(|entry| {
        let mut fields = Vec::new();
        if version >= 1 {
            fields.push(format!("EQSetID={}", entry.eq_set_id));
        }
        fields.push(format!("DownmixID={}", entry.downmix_id));
        fields.push(format!("DRCSetID={}", entry.drc_set_id));
        fields.push(format!("BsSamplePeakLevel={}", entry.bs_sample_peak_level));
        fields.push(format!("BsTruePeakLevel={}", entry.bs_true_peak_level));
        fields.push(format!(
            "MeasurementSystemForTP={}",
            entry.measurement_system_for_tp
        ));
        fields.push(format!("ReliabilityForTP={}", entry.reliability_for_tp));
        fields.push(format!(
            "Measurements={}",
            render_loudness_measurements(&entry.measurements)
        ));
        format!("{{{}}}", fields.join(" "))
    }))
}

fn quoted_fourcc(value: FourCc) -> String {
    format!("\"{value}\"")
}

fn quote_bytes(bytes: &[u8]) -> String {
    format!("\"{}\"", escape_bytes(bytes))
}

fn escape_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| escape_display_char(char::from(*byte)))
        .collect()
}

fn escape_display_char(value: char) -> char {
    if value.is_control() || !value.is_ascii_graphic() && value != ' ' {
        '.'
    } else {
        value
    }
}

fn format_fixed_16_16_signed(value: i32) -> String {
    if value & 0xffff == 0 {
        return (value >> 16).to_string();
    }
    format!("{:.5}", f64::from(value) / 65536.0)
}

fn format_fixed_16_16_unsigned(value: u32) -> String {
    if value & 0xffff == 0 {
        return (value >> 16).to_string();
    }
    format!("{:.5}", f64::from(value) / 65536.0)
}

fn format_fixed_8_8_signed(value: i16) -> String {
    if value & 0xff == 0 {
        return (value >> 8).to_string();
    }
    format!("{:.3}", f32::from(value) / 256.0)
}

fn read_i16(bytes: &[u8], offset: usize) -> i16 {
    i16::from_be_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_be_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn read_i64(bytes: &[u8], offset: usize) -> i64 {
    i64::from_be_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn read_uint(bytes: &[u8], offset: usize, width_bytes: usize) -> u64 {
    let mut value = 0_u64;
    for byte in &bytes[offset..offset + width_bytes] {
        value = (value << 8) | u64::from(*byte);
    }
    value
}

fn push_uint(
    field_name: &'static str,
    bytes: &mut Vec<u8>,
    width_bytes: usize,
    value: u64,
) -> Result<(), FieldValueError> {
    let max_value = if width_bytes == 8 {
        u64::MAX
    } else {
        (1_u64 << (width_bytes * 8)) - 1
    };
    if value > max_value {
        return Err(invalid_value(
            field_name,
            "value does not fit in the configured byte width",
        ));
    }

    for shift in (0..width_bytes).rev() {
        bytes.push((value >> (shift * 8)) as u8);
    }
    Ok(())
}

fn field_len_from_count(count: u32, bytes_per_entry: usize) -> Option<u32> {
    usize::try_from(count)
        .ok()
        .and_then(|count| field_len_bytes(count, bytes_per_entry))
}

fn encode_avc_parameter_sets(
    field_name: &'static str,
    parameter_sets: &[AVCParameterSet],
) -> Result<Vec<u8>, FieldValueError> {
    let mut bytes = Vec::new();
    for parameter_set in parameter_sets {
        let actual_len = u16::try_from(parameter_set.nal_unit.len())
            .map_err(|_| invalid_value(field_name, "parameter set length does not fit in u16"))?;
        if parameter_set.length != actual_len {
            return Err(invalid_value(
                field_name,
                "parameter set length does not match the NAL unit size",
            ));
        }
        bytes.extend_from_slice(&parameter_set.length.to_be_bytes());
        bytes.extend_from_slice(&parameter_set.nal_unit);
    }
    Ok(bytes)
}

fn encoded_avc_parameter_sets_len(
    field_name: &'static str,
    parameter_sets: &[AVCParameterSet],
) -> Result<u32, FieldValueError> {
    let bytes = encode_avc_parameter_sets(field_name, parameter_sets)?;
    u32::try_from(bytes.len()).map_err(|_| {
        invalid_value(
            field_name,
            "parameter-set payload length does not fit in u32",
        )
    })
}

fn parse_avc_parameter_sets(
    field_name: &'static str,
    bytes: &[u8],
    expected_count: u8,
) -> Result<Vec<AVCParameterSet>, FieldValueError> {
    let mut parameter_sets =
        Vec::with_capacity(untrusted_prealloc_hint(usize::from(expected_count)));
    let mut offset = 0_usize;
    for _ in 0..expected_count {
        if bytes.len().saturating_sub(offset) < 2 {
            return Err(invalid_value(
                field_name,
                "parameter-set payload length does not match the entry count",
            ));
        }

        let length = read_u16(bytes, offset);
        offset += 2;
        let end = offset + usize::from(length);
        if end > bytes.len() {
            return Err(invalid_value(
                field_name,
                "parameter-set payload length does not match the entry count",
            ));
        }

        parameter_sets.push(AVCParameterSet {
            length,
            nal_unit: bytes[offset..end].to_vec(),
        });
        offset = end;
    }

    if offset != bytes.len() {
        return Err(invalid_value(
            field_name,
            "parameter-set payload length does not match the entry count",
        ));
    }

    Ok(parameter_sets)
}

fn render_avc_parameter_sets(parameter_sets: &[AVCParameterSet]) -> String {
    render_array(parameter_sets.iter().map(|parameter_set| {
        format!(
            "{{Length={} NALUnit={}}}",
            parameter_set.length,
            render_hex_bytes(&parameter_set.nal_unit)
        )
    }))
}

fn pack_hevc_profile_compatibility(values: &[bool; 32]) -> [u8; 4] {
    let mut bytes = [0_u8; 4];
    for (index, value) in values.iter().copied().enumerate() {
        if value {
            bytes[index / 8] |= 1 << (7 - (index % 8));
        }
    }
    bytes
}

fn unpack_hevc_profile_compatibility(bytes: &[u8; 4]) -> [bool; 32] {
    let mut values = [false; 32];
    for (index, value) in values.iter_mut().enumerate() {
        *value = bytes[index / 8] & (1 << (7 - (index % 8))) != 0;
    }
    values
}

fn encode_hevc_nalus(
    field_name: &'static str,
    nalus: &[HEVCNalu],
) -> Result<Vec<u8>, FieldValueError> {
    let mut bytes = Vec::new();
    for nalu in nalus {
        let actual_len = u16::try_from(nalu.nal_unit.len())
            .map_err(|_| invalid_value(field_name, "NAL unit length does not fit in u16"))?;
        if nalu.length != actual_len {
            return Err(invalid_value(
                field_name,
                "NAL unit length does not match the NAL unit size",
            ));
        }
        bytes.extend_from_slice(&nalu.length.to_be_bytes());
        bytes.extend_from_slice(&nalu.nal_unit);
    }
    Ok(bytes)
}

fn render_hevc_nalus(nalus: &[HEVCNalu]) -> String {
    render_array(nalus.iter().map(|nalu| {
        format!(
            "{{Length={} NALUnit={}}}",
            nalu.length,
            render_hex_bytes(&nalu.nal_unit)
        )
    }))
}

fn encode_hevc_nalu_arrays(
    field_name: &'static str,
    arrays: &[HEVCNaluArray],
) -> Result<Vec<u8>, FieldValueError> {
    let mut bytes = Vec::new();
    for array in arrays {
        if array.nalu_type > 0x3f {
            return Err(invalid_value("NaluType", "value does not fit in 6 bits"));
        }
        require_count("NumNalus", u32::from(array.num_nalus), array.nalus.len())?;
        let nalus = encode_hevc_nalus("Nalus", &array.nalus)?;
        bytes.push(
            (u8::from(array.completeness) << 7) | (u8::from(array.reserved) << 6) | array.nalu_type,
        );
        bytes.extend_from_slice(&array.num_nalus.to_be_bytes());
        bytes.extend_from_slice(&nalus);
    }

    let _ = field_name;
    Ok(bytes)
}

fn encoded_hevc_nalu_arrays_len(
    field_name: &'static str,
    arrays: &[HEVCNaluArray],
) -> Result<u32, FieldValueError> {
    let bytes = encode_hevc_nalu_arrays(field_name, arrays)?;
    u32::try_from(bytes.len())
        .map_err(|_| invalid_value(field_name, "NAL-array payload length does not fit in u32"))
}

fn parse_hevc_nalu_arrays(
    field_name: &'static str,
    bytes: &[u8],
    expected_count: u8,
) -> Result<Vec<HEVCNaluArray>, FieldValueError> {
    let mut arrays = Vec::with_capacity(untrusted_prealloc_hint(usize::from(expected_count)));
    let mut offset = 0_usize;

    for _ in 0..expected_count {
        if bytes.len().saturating_sub(offset) < 3 {
            return Err(invalid_value(
                field_name,
                "NAL-array payload length does not match the entry count",
            ));
        }

        let header = bytes[offset];
        let completeness = header & 0x80 != 0;
        let reserved = header & 0x40 != 0;
        let nalu_type = header & 0x3f;
        offset += 1;

        let num_nalus = read_u16(bytes, offset);
        offset += 2;

        let mut nalus = Vec::with_capacity(untrusted_prealloc_hint(usize::from(num_nalus)));
        for _ in 0..num_nalus {
            if bytes.len().saturating_sub(offset) < 2 {
                return Err(invalid_value(
                    field_name,
                    "NAL-array payload length does not match the entry count",
                ));
            }

            let length = read_u16(bytes, offset);
            offset += 2;
            let end = offset + usize::from(length);
            if end > bytes.len() {
                return Err(invalid_value(
                    field_name,
                    "NAL-array payload length does not match the entry count",
                ));
            }

            nalus.push(HEVCNalu {
                length,
                nal_unit: bytes[offset..end].to_vec(),
            });
            offset = end;
        }

        arrays.push(HEVCNaluArray {
            completeness,
            reserved,
            nalu_type,
            num_nalus,
            nalus,
        });
    }

    if offset != bytes.len() {
        return Err(invalid_value(
            field_name,
            "NAL-array payload length does not match the entry count",
        ));
    }

    Ok(arrays)
}

fn render_hevc_nalu_arrays(arrays: &[HEVCNaluArray]) -> String {
    render_array(arrays.iter().map(|array| {
        format!(
            "{{Completeness={} Reserved={} NaluType=0x{:x} NumNalus={} Nalus={}}}",
            array.completeness,
            array.reserved,
            array.nalu_type,
            array.num_nalus,
            render_hevc_nalus(&array.nalus)
        )
    }))
}

fn avc_profile_supports_extensions(profile: u8) -> bool {
    matches!(profile, 100 | 110 | 122 | 144)
}

fn require_count(
    field_name: &'static str,
    expected_count: u32,
    actual_count: usize,
) -> Result<(), FieldValueError> {
    if usize::try_from(expected_count).ok() != Some(actual_count) {
        return Err(invalid_value(
            field_name,
            "entry count does not match the parsed payload",
        ));
    }
    Ok(())
}

macro_rules! impl_leaf_box {
    ($name:ident, $box_type:expr) => {
        impl FieldHooks for $name {}

        impl ImmutableBox for $name {
            fn box_type(&self) -> FourCc {
                FourCc::from_bytes($box_type)
            }
        }

        impl MutableBox for $name {}
    };
}

macro_rules! impl_full_box {
    ($name:ident, $box_type:expr) => {
        impl ImmutableBox for $name {
            fn box_type(&self) -> FourCc {
                FourCc::from_bytes($box_type)
            }

            fn version(&self) -> u8 {
                self.full_box.version
            }

            fn flags(&self) -> u32 {
                self.full_box.flags
            }
        }

        impl MutableBox for $name {
            fn set_version(&mut self, version: u8) {
                self.full_box.version = version;
            }

            fn set_flags(&mut self, flags: u32) {
                self.full_box.flags = flags;
            }
        }
    };
}

macro_rules! empty_hooks {
    ($($name:ident),* $(,)?) => {
        $(
            impl FieldHooks for $name {}
        )*
    };
}

macro_rules! empty_box_codec {
    ($name:ident) => {
        impl FieldValueRead for $name {
            fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
                Err(missing_field(field_name))
            }
        }

        impl FieldValueWrite for $name {
            fn set_field_value(
                &mut self,
                field_name: &'static str,
                value: FieldValue,
            ) -> Result<(), FieldValueError> {
                Err(unexpected_field(field_name, value))
            }
        }

        impl CodecBox for $name {
            const FIELD_TABLE: FieldTable = FieldTable::new(&[]);
        }
    };
}

macro_rules! empty_full_box_codec {
    ($name:ident) => {
        impl FieldValueRead for $name {
            fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
                Err(missing_field(field_name))
            }
        }

        impl FieldValueWrite for $name {
            fn set_field_value(
                &mut self,
                field_name: &'static str,
                value: FieldValue,
            ) -> Result<(), FieldValueError> {
                Err(unexpected_field(field_name, value))
            }
        }

        impl CodecBox for $name {
            const FIELD_TABLE: FieldTable = FieldTable::new(&[
                codec_field!("Version", 0, with_bit_width(8), as_version_field()),
                codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
            ]);
            const SUPPORTED_VERSIONS: &'static [u8] = &[0];
        }
    };
}

macro_rules! simple_container_box {
    ($name:ident, $box_type:expr) => {
        #[doc = "Container box with no direct payload fields."]
        #[derive(Clone, Debug, Default, PartialEq, Eq)]
        pub struct $name;

        impl_leaf_box!($name, $box_type);
        empty_box_codec!($name);
    };
}

macro_rules! raw_data_box {
    ($name:ident, $box_type:expr) => {
        #[doc = "Raw-data box that preserves its payload bytes verbatim."]
        #[derive(Clone, Debug, Default, PartialEq, Eq)]
        pub struct $name {
            pub data: Vec<u8>,
        }

        impl_leaf_box!($name, $box_type);

        impl FieldValueRead for $name {
            fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
                match field_name {
                    "Data" => Ok(FieldValue::Bytes(self.data.clone())),
                    _ => Err(missing_field(field_name)),
                }
            }
        }

        impl FieldValueWrite for $name {
            fn set_field_value(
                &mut self,
                field_name: &'static str,
                value: FieldValue,
            ) -> Result<(), FieldValueError> {
                match (field_name, value) {
                    ("Data", FieldValue::Bytes(data)) => {
                        self.data = data;
                        Ok(())
                    }
                    (field_name, value) => Err(unexpected_field(field_name, value)),
                }
            }
        }

        impl CodecBox for $name {
            const FIELD_TABLE: FieldTable =
                FieldTable::new(&[codec_field!("Data", 0, with_bit_width(8), as_bytes())]);
        }
    };
}

macro_rules! track_id_list_box {
    ($name:ident, $box_type:expr, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, Default, PartialEq, Eq)]
        pub struct $name {
            pub track_ids: Vec<u32>,
        }

        impl FieldHooks for $name {
            fn field_length(&self, name: &'static str) -> Option<u32> {
                match name {
                    "TrackIDs" => field_len_bytes(self.track_ids.len(), 4),
                    _ => None,
                }
            }

            fn display_field(&self, name: &'static str) -> Option<String> {
                match name {
                    "TrackIDs" => Some(render_array(
                        self.track_ids.iter().map(|track_id| track_id.to_string()),
                    )),
                    _ => None,
                }
            }
        }

        impl ImmutableBox for $name {
            fn box_type(&self) -> FourCc {
                FourCc::from_bytes($box_type)
            }
        }

        impl MutableBox for $name {}

        impl FieldValueRead for $name {
            fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
                match field_name {
                    "TrackIDs" => Ok(FieldValue::Bytes(track_id_vec_to_bytes(&self.track_ids))),
                    _ => Err(missing_field(field_name)),
                }
            }
        }

        impl FieldValueWrite for $name {
            fn set_field_value(
                &mut self,
                field_name: &'static str,
                value: FieldValue,
            ) -> Result<(), FieldValueError> {
                match (field_name, value) {
                    ("TrackIDs", FieldValue::Bytes(bytes)) => {
                        self.track_ids = bytes_to_track_id_vec(field_name, bytes)?;
                        Ok(())
                    }
                    (field_name, value) => Err(unexpected_field(field_name, value)),
                }
            }
        }

        impl CodecBox for $name {
            const FIELD_TABLE: FieldTable =
                FieldTable::new(&[codec_field!("TrackIDs", 0, with_bit_width(8), as_bytes())]);
        }
    };
}

simple_container_box!(Dinf, *b"dinf");
simple_container_box!(Edts, *b"edts");
simple_container_box!(Mdia, *b"mdia");
simple_container_box!(Minf, *b"minf");
simple_container_box!(Moof, *b"moof");
simple_container_box!(Moov, *b"moov");
simple_container_box!(Mvex, *b"mvex");
simple_container_box!(Mfra, *b"mfra");
simple_container_box!(Stbl, *b"stbl");
simple_container_box!(Traf, *b"traf");
simple_container_box!(Trak, *b"trak");
simple_container_box!(Tref, *b"tref");

raw_data_box!(Free, *b"free");
raw_data_box!(Skip, *b"skip");
raw_data_box!(Mdat, *b"mdat");

/// Closed-caption sample-data box that preserves its payload bytes verbatim.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cdat {
    pub data: Vec<u8>,
}

impl_leaf_box!(Cdat, *b"cdat");

impl FieldValueRead for Cdat {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Data" => Ok(FieldValue::Bytes(self.data.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Cdat {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Data", FieldValue::Bytes(data)) => {
                self.data = data;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Cdat {
    const FIELD_TABLE: FieldTable =
        FieldTable::new(&[codec_field!("Data", 0, with_bit_width(8), as_bytes())]);
}

/// User-data container carried by boxes such as `moov` and `trak`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Udta;

impl_leaf_box!(Udta, *b"udta");
empty_box_codec!(Udta);

/// User-data loudness container that groups track and album loudness boxes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ludt;

impl_leaf_box!(Ludt, *b"ludt");
empty_box_codec!(Ludt);

/// One loudness measurement record carried by `tlou` and `alou`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoudnessMeasurement {
    pub method_definition: u8,
    pub method_value: u8,
    pub measurement_system: u8,
    pub reliability: u8,
}

/// One loudness entry carried by `tlou` and `alou`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoudnessEntry {
    pub eq_set_id: u8,
    pub downmix_id: u16,
    pub drc_set_id: u8,
    pub bs_sample_peak_level: u16,
    pub bs_true_peak_level: u16,
    pub measurement_system_for_tp: u8,
    pub reliability_for_tp: u8,
    pub measurements: Vec<LoudnessMeasurement>,
}

macro_rules! define_loudness_info_box {
    ($(#[$doc:meta])* $name:ident, $box_type:expr) => {
        $(#[$doc])*
        #[derive(Clone, Debug, Default, PartialEq, Eq)]
        pub struct $name {
            full_box: FullBoxState,
            pub entries: Vec<LoudnessEntry>,
        }

        impl FieldHooks for $name {
            fn field_length(&self, name: &'static str) -> Option<u32> {
                match name {
                    "Entries" => encoded_loudness_entries_len(self.version(), &self.entries).ok(),
                    _ => None,
                }
            }

            fn display_field(&self, name: &'static str) -> Option<String> {
                match name {
                    "Entries" => Some(render_loudness_entries(self.version(), &self.entries)),
                    _ => None,
                }
            }
        }

        impl ImmutableBox for $name {
            fn box_type(&self) -> FourCc {
                FourCc::from_bytes($box_type)
            }

            fn version(&self) -> u8 {
                self.full_box.version
            }

            fn flags(&self) -> u32 {
                self.full_box.flags
            }
        }

        impl MutableBox for $name {
            fn set_version(&mut self, version: u8) {
                self.full_box.version = version;
            }

            fn set_flags(&mut self, flags: u32) {
                self.full_box.flags = flags;
            }
        }

        impl FieldValueRead for $name {
            fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
                match field_name {
                    "Entries" => Ok(FieldValue::Bytes(encode_loudness_entries(
                        field_name,
                        self.version(),
                        &self.entries,
                    )?)),
                    _ => Err(missing_field(field_name)),
                }
            }
        }

        impl FieldValueWrite for $name {
            fn set_field_value(
                &mut self,
                field_name: &'static str,
                value: FieldValue,
            ) -> Result<(), FieldValueError> {
                match (field_name, value) {
                    ("Entries", FieldValue::Bytes(value)) => {
                        self.entries = decode_loudness_entries(field_name, self.version(), &value)?;
                        Ok(())
                    }
                    (field_name, value) => Err(unexpected_field(field_name, value)),
                }
            }
        }

        impl CodecBox for $name {
            const FIELD_TABLE: FieldTable = FieldTable::new(&[
                codec_field!("Version", 0, with_bit_width(8), as_version_field()),
                codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
                codec_field!("Entries", 2, with_bit_width(8), with_dynamic_length(), as_bytes()),
            ]);
            const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];

            fn custom_marshal(
                &self,
                writer: &mut dyn Write,
            ) -> Result<Option<u64>, CodecError> {
                if self.version() > 1 {
                    return Err(CodecError::UnsupportedVersion {
                        box_type: self.box_type(),
                        version: self.version(),
                    });
                }
                if self.flags() != 0 {
                    return Err(invalid_value("Flags", "non-zero flags are not supported").into());
                }

                let entries = encode_loudness_entries("Entries", self.version(), &self.entries)?;
                let mut payload = Vec::with_capacity(4 + entries.len());
                payload.push(self.version());
                payload.extend_from_slice(&self.flags().to_be_bytes()[1..]);
                payload.extend_from_slice(&entries);
                writer.write_all(&payload)?;
                Ok(Some(payload.len() as u64))
            }

            fn custom_unmarshal(
                &mut self,
                reader: &mut dyn ReadSeek,
                payload_size: u64,
            ) -> Result<Option<u64>, CodecError> {
                let payload_len = usize::try_from(payload_size)
                    .map_err(|_| invalid_value("Payload", "payload is too large to decode"))?;
                if payload_len < 4 {
                    return Err(invalid_value("Payload", "payload is too short").into());
                }

                let payload = read_exact_vec_untrusted(reader, payload_len)?;
                let version = payload[0];
                if version > 1 {
                    return Err(CodecError::UnsupportedVersion {
                        box_type: self.box_type(),
                        version,
                    });
                }
                let flags = u32::from_be_bytes([0, payload[1], payload[2], payload[3]]);
                if flags != 0 {
                    return Err(invalid_value("Flags", "non-zero flags are not supported").into());
                }

                self.full_box = FullBoxState { version, flags };
                self.entries = decode_loudness_entries("Entries", version, &payload[4..])?;
                Ok(Some(payload_size))
            }
        }
    };
}

define_loudness_info_box!(
    /// Track loudness metadata box carried under `ludt`.
    TrackLoudnessInfo,
    *b"tlou"
);

define_loudness_info_box!(
    /// Album loudness metadata box carried under `ludt`.
    AlbumLoudnessInfo,
    *b"alou"
);

/// Spherical-video metadata payload stored inside one `uuid` box subtype.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SphericalVideoV1Metadata {
    pub xml_data: Vec<u8>,
}

/// Fragment-absolute-timing payload stored inside one `uuid` box subtype.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UuidFragmentAbsoluteTiming {
    pub version: u8,
    pub flags: u32,
    pub fragment_absolute_time: u64,
    pub fragment_absolute_duration: u64,
}

/// One fragment timing record carried by a fragment-run table `uuid` payload.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UuidFragmentRunEntry {
    pub fragment_absolute_time: u64,
    pub fragment_absolute_duration: u64,
}

/// Fragment-run table payload stored inside one `uuid` box subtype.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UuidFragmentRunTable {
    pub version: u8,
    pub flags: u32,
    pub fragment_count: u8,
    pub entries: Vec<UuidFragmentRunEntry>,
}

/// Typed payload variants for `uuid` boxes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UuidPayload {
    Raw(Vec<u8>),
    SphericalVideoV1(SphericalVideoV1Metadata),
    FragmentAbsoluteTiming(UuidFragmentAbsoluteTiming),
    FragmentRunTable(UuidFragmentRunTable),
    SampleEncryption(Senc),
}

impl Default for UuidPayload {
    fn default() -> Self {
        Self::Raw(Vec::new())
    }
}

/// User-type box that keeps unknown payloads opaque while modeling selected UUID subtypes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Uuid {
    pub user_type: [u8; 16],
    pub payload: UuidPayload,
}

impl FieldHooks for Uuid {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "RawPayload" => match &self.payload {
                UuidPayload::Raw(bytes) => u32::try_from(bytes.len()).ok(),
                _ => None,
            },
            "XMLData" => match &self.payload {
                UuidPayload::SphericalVideoV1(data) => u32::try_from(data.xml_data.len()).ok(),
                _ => None,
            },
            "Entries" => match &self.payload {
                UuidPayload::FragmentRunTable(data) => {
                    u32::try_from(encode_uuid_fragment_run_entries(name, data).ok()?.len()).ok()
                }
                _ => None,
            },
            "Samples" => match &self.payload {
                UuidPayload::SampleEncryption(data) => {
                    let payload = encode_senc_payload(data).ok()?;
                    u32::try_from(payload.len().saturating_sub(8)).ok()
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn field_enabled(&self, name: &'static str) -> Option<bool> {
        match name {
            "RawPayload" => Some(matches!(self.payload, UuidPayload::Raw(_))),
            "XMLData" => Some(matches!(self.payload, UuidPayload::SphericalVideoV1(_))),
            "Version" | "Flags" => Some(matches!(
                self.payload,
                UuidPayload::FragmentAbsoluteTiming(_)
                    | UuidPayload::FragmentRunTable(_)
                    | UuidPayload::SampleEncryption(_)
            )),
            "FragmentAbsoluteTime" | "FragmentAbsoluteDuration" => Some(matches!(
                self.payload,
                UuidPayload::FragmentAbsoluteTiming(_)
            )),
            "FragmentCount" | "Entries" => {
                Some(matches!(self.payload, UuidPayload::FragmentRunTable(_)))
            }
            "SampleCount" | "Samples" => {
                Some(matches!(self.payload, UuidPayload::SampleEncryption(_)))
            }
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match (name, &self.payload) {
            ("XMLData", UuidPayload::SphericalVideoV1(data)) => Some(quote_bytes(&data.xml_data)),
            ("Entries", UuidPayload::FragmentRunTable(data)) => {
                Some(render_uuid_fragment_run_entries(&data.entries))
            }
            ("Samples", UuidPayload::SampleEncryption(data)) => {
                Some(render_senc_samples_display(&data.samples))
            }
            _ => None,
        }
    }
}

impl ImmutableBox for Uuid {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"uuid")
    }

    fn version(&self) -> u8 {
        match &self.payload {
            UuidPayload::FragmentAbsoluteTiming(data) => data.version,
            UuidPayload::FragmentRunTable(data) => data.version,
            UuidPayload::SampleEncryption(data) => data.version(),
            _ => ANY_VERSION,
        }
    }

    fn flags(&self) -> u32 {
        match &self.payload {
            UuidPayload::FragmentAbsoluteTiming(data) => data.flags,
            UuidPayload::FragmentRunTable(data) => data.flags,
            UuidPayload::SampleEncryption(data) => data.flags(),
            _ => 0,
        }
    }
}

impl MutableBox for Uuid {
    fn set_version(&mut self, version: u8) {
        match &mut self.payload {
            UuidPayload::FragmentAbsoluteTiming(data) => data.version = version,
            UuidPayload::FragmentRunTable(data) => data.version = version,
            UuidPayload::SampleEncryption(data) => data.set_version(version),
            _ => {}
        }
    }

    fn set_flags(&mut self, flags: u32) {
        match &mut self.payload {
            UuidPayload::FragmentAbsoluteTiming(data) => data.flags = flags,
            UuidPayload::FragmentRunTable(data) => data.flags = flags,
            UuidPayload::SampleEncryption(data) => data.set_flags(flags),
            _ => {}
        }
    }
}

impl FieldValueRead for Uuid {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match (field_name, &self.payload) {
            ("UserType", _) => Ok(FieldValue::Bytes(self.user_type.to_vec())),
            ("RawPayload", UuidPayload::Raw(bytes)) => Ok(FieldValue::Bytes(bytes.clone())),
            ("XMLData", UuidPayload::SphericalVideoV1(data)) => {
                Ok(FieldValue::Bytes(data.xml_data.clone()))
            }
            ("FragmentAbsoluteTime", UuidPayload::FragmentAbsoluteTiming(data)) => {
                Ok(FieldValue::Unsigned(data.fragment_absolute_time))
            }
            ("FragmentAbsoluteDuration", UuidPayload::FragmentAbsoluteTiming(data)) => {
                Ok(FieldValue::Unsigned(data.fragment_absolute_duration))
            }
            ("FragmentCount", UuidPayload::FragmentRunTable(data)) => {
                Ok(FieldValue::Unsigned(u64::from(data.fragment_count)))
            }
            ("Entries", UuidPayload::FragmentRunTable(data)) => Ok(FieldValue::Bytes(
                encode_uuid_fragment_run_entries(field_name, data)?,
            )),
            ("SampleCount", UuidPayload::SampleEncryption(data)) => {
                Ok(FieldValue::Unsigned(u64::from(data.sample_count)))
            }
            ("Samples", UuidPayload::SampleEncryption(data)) => Ok(FieldValue::Bytes(
                encode_senc_payload(data).map_err(|_| {
                    invalid_value(field_name, "sample encryption payload is invalid")
                })?[8..]
                    .to_vec(),
            )),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Uuid {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("UserType", FieldValue::Bytes(value)) => {
                let payload_bytes = encode_uuid_payload(self.user_type, &self.payload)?;
                self.user_type = bytes_to_uuid(field_name, value)?;
                self.payload = if payload_bytes.is_empty() {
                    match self.user_type {
                        UUID_SPHERICAL_VIDEO_V1 => {
                            UuidPayload::SphericalVideoV1(SphericalVideoV1Metadata::default())
                        }
                        UUID_FRAGMENT_ABSOLUTE_TIMING => UuidPayload::FragmentAbsoluteTiming(
                            UuidFragmentAbsoluteTiming::default(),
                        ),
                        UUID_FRAGMENT_RUN_TABLE => {
                            UuidPayload::FragmentRunTable(UuidFragmentRunTable::default())
                        }
                        UUID_SAMPLE_ENCRYPTION => UuidPayload::SampleEncryption(Senc::default()),
                        _ => UuidPayload::Raw(Vec::new()),
                    }
                } else {
                    decode_uuid_payload(self.user_type, &payload_bytes)?
                };
                Ok(())
            }
            ("RawPayload", FieldValue::Bytes(value)) => {
                self.payload = UuidPayload::Raw(value);
                Ok(())
            }
            ("XMLData", FieldValue::Bytes(value)) => {
                if self.user_type != UUID_SPHERICAL_VIDEO_V1 {
                    return Err(invalid_value(
                        field_name,
                        "field requires the spherical UUID user type",
                    ));
                }
                self.payload =
                    UuidPayload::SphericalVideoV1(SphericalVideoV1Metadata { xml_data: value });
                Ok(())
            }
            ("FragmentAbsoluteTime", FieldValue::Unsigned(value)) => {
                let UuidPayload::FragmentAbsoluteTiming(data) = &mut self.payload else {
                    return Err(missing_field(field_name));
                };
                data.fragment_absolute_time = value;
                Ok(())
            }
            ("FragmentAbsoluteDuration", FieldValue::Unsigned(value)) => {
                let UuidPayload::FragmentAbsoluteTiming(data) = &mut self.payload else {
                    return Err(missing_field(field_name));
                };
                data.fragment_absolute_duration = value;
                Ok(())
            }
            ("FragmentCount", FieldValue::Unsigned(value)) => {
                let UuidPayload::FragmentRunTable(data) = &mut self.payload else {
                    return Err(missing_field(field_name));
                };
                data.fragment_count = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Entries", FieldValue::Bytes(value)) => {
                let UuidPayload::FragmentRunTable(data) = &mut self.payload else {
                    return Err(missing_field(field_name));
                };
                data.entries = decode_uuid_fragment_run_entries(
                    field_name,
                    data.version,
                    data.fragment_count,
                    &value,
                )?;
                Ok(())
            }
            ("SampleCount", FieldValue::Unsigned(value)) => {
                let UuidPayload::SampleEncryption(data) = &mut self.payload else {
                    return Err(missing_field(field_name));
                };
                data.sample_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Samples", FieldValue::Bytes(value)) => {
                let UuidPayload::SampleEncryption(data) = &mut self.payload else {
                    return Err(missing_field(field_name));
                };
                let mut payload = Vec::with_capacity(8 + value.len());
                payload.push(data.version());
                payload.extend_from_slice(&(data.flags() & 0x00ff_ffff).to_be_bytes()[1..]);
                payload.extend_from_slice(&data.sample_count.to_be_bytes());
                payload.extend_from_slice(&value);
                *data = decode_senc_payload(&payload).map_err(|_| {
                    invalid_value(field_name, "sample encryption payload is invalid")
                })?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Uuid {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!(
            "UserType",
            0,
            with_bit_width(8),
            with_length(16),
            as_bytes(),
            as_uuid()
        ),
        codec_field!(
            "RawPayload",
            1,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes(),
            with_dynamic_presence()
        ),
        codec_field!(
            "XMLData",
            2,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes(),
            with_dynamic_presence()
        ),
        codec_field!(
            "Version",
            3,
            with_bit_width(8),
            as_version_field(),
            with_dynamic_presence()
        ),
        codec_field!(
            "Flags",
            4,
            with_bit_width(24),
            as_flags_field(),
            with_dynamic_presence()
        ),
        codec_field!(
            "FragmentAbsoluteTime",
            5,
            with_bit_width(64),
            with_dynamic_presence()
        ),
        codec_field!(
            "FragmentAbsoluteDuration",
            6,
            with_bit_width(64),
            with_dynamic_presence()
        ),
        codec_field!(
            "FragmentCount",
            7,
            with_bit_width(8),
            with_dynamic_presence()
        ),
        codec_field!(
            "Entries",
            8,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes(),
            with_dynamic_presence()
        ),
        codec_field!(
            "SampleCount",
            9,
            with_bit_width(32),
            with_dynamic_presence()
        ),
        codec_field!(
            "Samples",
            10,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes(),
            with_dynamic_presence()
        ),
    ]);

    fn custom_marshal(&self, writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        let payload_bytes = encode_uuid_payload(self.user_type, &self.payload)?;
        let mut payload = Vec::with_capacity(16 + payload_bytes.len());
        payload.extend_from_slice(&self.user_type);
        payload.extend_from_slice(&payload_bytes);
        writer.write_all(&payload)?;
        Ok(Some(payload.len() as u64))
    }

    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, CodecError> {
        let payload_len = usize::try_from(payload_size)
            .map_err(|_| invalid_value("Payload", "payload is too large to decode"))?;
        if payload_len < 16 {
            return Err(invalid_value("Payload", "payload is too short").into());
        }

        let payload = read_exact_vec_untrusted(reader, payload_len)?;
        self.user_type = payload[..16].try_into().unwrap();
        self.payload = decode_uuid_payload(self.user_type, &payload[16..])?;
        Ok(Some(payload_size))
    }
}

/// File type and compatibility declaration box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ftyp {
    pub major_brand: FourCc,
    pub minor_version: u32,
    pub compatible_brands: Vec<FourCc>,
}

impl Default for Ftyp {
    fn default() -> Self {
        Self {
            major_brand: FourCc::ANY,
            minor_version: 0,
            compatible_brands: Vec::new(),
        }
    }
}

impl Ftyp {
    /// Adds `brand` if it is not already listed as compatible.
    pub fn add_compatible_brand(&mut self, brand: FourCc) {
        if !self.has_compatible_brand(brand) {
            self.compatible_brands.push(brand);
        }
    }

    /// Removes `brand` from the compatibility list.
    pub fn remove_compatible_brand(&mut self, brand: FourCc) {
        self.compatible_brands
            .retain(|candidate| *candidate != brand);
    }

    /// Returns `true` when `brand` is present in the compatibility list.
    pub fn has_compatible_brand(&self, brand: FourCc) -> bool {
        self.compatible_brands.contains(&brand)
    }
}

impl FieldHooks for Ftyp {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "CompatibleBrands" => field_len_bytes(self.compatible_brands.len(), 4),
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "MajorBrand" => Some(quoted_fourcc(self.major_brand)),
            "CompatibleBrands" => {
                Some(render_array(self.compatible_brands.iter().map(|brand| {
                    format!("{{CompatibleBrand={}}}", quoted_fourcc(*brand))
                })))
            }
            _ => None,
        }
    }
}

impl ImmutableBox for Ftyp {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"ftyp")
    }
}

impl MutableBox for Ftyp {}

impl FieldValueRead for Ftyp {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "MajorBrand" => Ok(FieldValue::Bytes(self.major_brand.as_bytes().to_vec())),
            "MinorVersion" => Ok(FieldValue::Unsigned(u64::from(self.minor_version))),
            "CompatibleBrands" => Ok(FieldValue::Bytes(fourcc_vec_to_bytes(
                &self.compatible_brands,
            ))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Ftyp {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("MajorBrand", FieldValue::Bytes(bytes)) => {
                self.major_brand = bytes_to_fourcc(field_name, bytes)?;
                Ok(())
            }
            ("MinorVersion", FieldValue::Unsigned(value)) => {
                self.minor_version = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("CompatibleBrands", FieldValue::Bytes(bytes)) => {
                self.compatible_brands = bytes_to_fourcc_vec(field_name, bytes)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Ftyp {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!(
            "MajorBrand",
            0,
            with_bit_width(8),
            with_length(4),
            as_bytes()
        ),
        codec_field!("MinorVersion", 1, with_bit_width(32)),
        codec_field!("CompatibleBrands", 2, with_bit_width(8), as_bytes()),
    ]);
}

/// Segment type and compatibility declaration box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Styp {
    pub major_brand: FourCc,
    pub minor_version: u32,
    pub compatible_brands: Vec<FourCc>,
}

impl Default for Styp {
    fn default() -> Self {
        Self {
            major_brand: FourCc::ANY,
            minor_version: 0,
            compatible_brands: Vec::new(),
        }
    }
}

impl FieldHooks for Styp {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "CompatibleBrands" => field_len_bytes(self.compatible_brands.len(), 4),
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "MajorBrand" => Some(quoted_fourcc(self.major_brand)),
            "CompatibleBrands" => {
                Some(render_array(self.compatible_brands.iter().map(|brand| {
                    format!("{{CompatibleBrand={}}}", quoted_fourcc(*brand))
                })))
            }
            _ => None,
        }
    }
}

impl ImmutableBox for Styp {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"styp")
    }
}

impl MutableBox for Styp {}

impl FieldValueRead for Styp {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "MajorBrand" => Ok(FieldValue::Bytes(self.major_brand.as_bytes().to_vec())),
            "MinorVersion" => Ok(FieldValue::Unsigned(u64::from(self.minor_version))),
            "CompatibleBrands" => Ok(FieldValue::Bytes(fourcc_vec_to_bytes(
                &self.compatible_brands,
            ))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Styp {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("MajorBrand", FieldValue::Bytes(bytes)) => {
                self.major_brand = bytes_to_fourcc(field_name, bytes)?;
                Ok(())
            }
            ("MinorVersion", FieldValue::Unsigned(value)) => {
                self.minor_version = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("CompatibleBrands", FieldValue::Bytes(bytes)) => {
                self.compatible_brands = bytes_to_fourcc_vec(field_name, bytes)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Styp {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!(
            "MajorBrand",
            0,
            with_bit_width(8),
            with_length(4),
            as_bytes()
        ),
        codec_field!("MinorVersion", 1, with_bit_width(32)),
        codec_field!("CompatibleBrands", 2, with_bit_width(8), as_bytes()),
    ]);
}

empty_hooks!(
    Dref, Url, Urn, Mfhd, Mfro, Prft, Mehd, Mdhd, Tfdt, Tfhd, Trep, Trex, Vmhd, Stsd, Cslg
);

/// Data reference box that counts child data-entry boxes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dref {
    full_box: FullBoxState,
    pub entry_count: u32,
}

impl_full_box!(Dref, *b"dref");

impl FieldValueRead for Dref {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "EntryCount" => Ok(FieldValue::Unsigned(u64::from(self.entry_count))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Dref {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("EntryCount", FieldValue::Unsigned(value)) => {
                self.entry_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Dref {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("EntryCount", 2, with_bit_width(32)),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// URL data-entry box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Url {
    full_box: FullBoxState,
    pub location: String,
}

impl_full_box!(Url, *b"url ");

impl FieldValueRead for Url {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Location" => Ok(FieldValue::String(self.location.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Url {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Location", FieldValue::String(value)) => {
                self.location = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Url {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!(
            "Location",
            2,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated),
            with_forbidden_flags(URL_SELF_CONTAINED)
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// URN data-entry box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Urn {
    full_box: FullBoxState,
    pub name: String,
    pub location: String,
}

impl_full_box!(Urn, *b"urn ");

impl FieldValueRead for Urn {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Name" => Ok(FieldValue::String(self.name.clone())),
            "Location" => Ok(FieldValue::String(self.location.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Urn {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Name", FieldValue::String(value)) => {
                self.name = value;
                Ok(())
            }
            ("Location", FieldValue::String(value)) => {
                self.location = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Urn {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!(
            "Name",
            2,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated),
            with_forbidden_flags(URN_SELF_CONTAINED)
        ),
        codec_field!(
            "Location",
            3,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated),
            with_forbidden_flags(URN_SELF_CONTAINED)
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Movie fragment header box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mfhd {
    full_box: FullBoxState,
    pub sequence_number: u32,
}

impl_full_box!(Mfhd, *b"mfhd");

impl FieldValueRead for Mfhd {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "SequenceNumber" => Ok(FieldValue::Unsigned(u64::from(self.sequence_number))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Mfhd {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("SequenceNumber", FieldValue::Unsigned(value)) => {
                self.sequence_number = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Mfhd {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("SequenceNumber", 2, with_bit_width(32)),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Movie fragment random access offset box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mfro {
    full_box: FullBoxState,
    pub size: u32,
}

impl_full_box!(Mfro, *b"mfro");

impl FieldValueRead for Mfro {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Size" => Ok(FieldValue::Unsigned(u64::from(self.size))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Mfro {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Size", FieldValue::Unsigned(value)) => {
                self.size = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Mfro {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("Size", 2, with_bit_width(32)),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Producer reference time box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Prft {
    full_box: FullBoxState,
    pub reference_track_id: u32,
    pub ntp_timestamp: u64,
    pub media_time_v0: u32,
    pub media_time_v1: u64,
}

impl_full_box!(Prft, *b"prft");

impl Prft {
    /// Returns the active media time for the current box version.
    pub fn media_time(&self) -> u64 {
        match self.version() {
            0 => u64::from(self.media_time_v0),
            1 => self.media_time_v1,
            _ => 0,
        }
    }

    /// Returns the whole-second component of the stored NTP timestamp.
    pub fn ntp_seconds(&self) -> u32 {
        (self.ntp_timestamp >> 32) as u32
    }

    /// Returns the fractional component of the stored NTP timestamp.
    pub fn ntp_fraction(&self) -> u32 {
        self.ntp_timestamp as u32
    }

    /// Returns the fractional NTP component converted to nanoseconds.
    pub fn ntp_fraction_nanos(&self) -> u32 {
        ((u128::from(self.ntp_fraction()) * NANOS_PER_SECOND) / PRFT_NTP_FRACTION_SCALE) as u32
    }

    /// Returns the whole-second UNIX-epoch component represented by the NTP timestamp.
    ///
    /// Returns `None` when the stored timestamp predates `1970-01-01T00:00:00Z`.
    pub fn unix_seconds(&self) -> Option<u64> {
        u64::from(self.ntp_seconds()).checked_sub(PRFT_NTP_UNIX_EPOCH_OFFSET_SECONDS)
    }

    /// Returns the stored NTP timestamp as a UNIX `SystemTime`.
    ///
    /// Returns `None` when the stored timestamp predates `1970-01-01T00:00:00Z`.
    pub fn unix_time(&self) -> Option<SystemTime> {
        let seconds = self.unix_seconds()?;
        UNIX_EPOCH.checked_add(Duration::new(seconds, self.ntp_fraction_nanos()))
    }

    /// Returns the known capture-point name for the stored `prft` flags value.
    pub fn flag_meaning(&self) -> Option<&'static str> {
        Self::known_flag_meaning(self.flags())
    }

    /// Returns the known capture-point name for one `prft` flags value.
    pub fn known_flag_meaning(flags: u32) -> Option<&'static str> {
        match flags {
            PRFT_TIME_ENCODER_INPUT => Some("time_encoder_input"),
            PRFT_TIME_ENCODER_OUTPUT => Some("time_encoder_output"),
            PRFT_TIME_MOOF_FINALIZED => Some("time_moof_finalized"),
            PRFT_TIME_MOOF_WRITTEN => Some("time_moof_written"),
            PRFT_TIME_ARBITRARY_CONSISTENT => Some("time_arbitrary_consistent"),
            PRFT_TIME_CAPTURED => Some("time_captured"),
            _ => None,
        }
    }
}

impl FieldValueRead for Prft {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "ReferenceTrackID" => Ok(FieldValue::Unsigned(u64::from(self.reference_track_id))),
            "NTPTimestamp" => Ok(FieldValue::Unsigned(self.ntp_timestamp)),
            "MediaTimeV0" => Ok(FieldValue::Unsigned(u64::from(self.media_time_v0))),
            "MediaTimeV1" => Ok(FieldValue::Unsigned(self.media_time_v1)),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Prft {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("ReferenceTrackID", FieldValue::Unsigned(value)) => {
                self.reference_track_id = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("NTPTimestamp", FieldValue::Unsigned(value)) => {
                self.ntp_timestamp = value;
                Ok(())
            }
            ("MediaTimeV0", FieldValue::Unsigned(value)) => {
                self.media_time_v0 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("MediaTimeV1", FieldValue::Unsigned(value)) => {
                self.media_time_v1 = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Prft {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("ReferenceTrackID", 2, with_bit_width(32)),
        codec_field!("NTPTimestamp", 3, with_bit_width(64)),
        codec_field!("MediaTimeV0", 4, with_bit_width(32), with_version(0)),
        codec_field!("MediaTimeV1", 5, with_bit_width(64), with_version(1)),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// Movie extends header box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mehd {
    full_box: FullBoxState,
    pub fragment_duration_v0: u32,
    pub fragment_duration_v1: u64,
}

impl_full_box!(Mehd, *b"mehd");

impl Mehd {
    /// Returns the active fragment duration for the current box version.
    pub fn fragment_duration(&self) -> u64 {
        match self.version() {
            0 => u64::from(self.fragment_duration_v0),
            1 => self.fragment_duration_v1,
            _ => 0,
        }
    }
}

impl FieldValueRead for Mehd {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "FragmentDurationV0" => Ok(FieldValue::Unsigned(u64::from(self.fragment_duration_v0))),
            "FragmentDurationV1" => Ok(FieldValue::Unsigned(self.fragment_duration_v1)),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Mehd {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("FragmentDurationV0", FieldValue::Unsigned(value)) => {
                self.fragment_duration_v0 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("FragmentDurationV1", FieldValue::Unsigned(value)) => {
                self.fragment_duration_v1 = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Mehd {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("FragmentDurationV0", 2, with_bit_width(32), with_version(0)),
        codec_field!("FragmentDurationV1", 3, with_bit_width(64), with_version(1)),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// Media header box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mdhd {
    full_box: FullBoxState,
    pub creation_time_v0: u32,
    pub modification_time_v0: u32,
    pub creation_time_v1: u64,
    pub modification_time_v1: u64,
    pub timescale: u32,
    pub duration_v0: u32,
    pub duration_v1: u64,
    pub pad: bool,
    pub language: [u8; 3],
    pub pre_defined: u16,
}

impl_full_box!(Mdhd, *b"mdhd");

impl Mdhd {
    /// Returns the active media creation time for the current box version.
    pub fn creation_time(&self) -> u64 {
        match self.version() {
            0 => u64::from(self.creation_time_v0),
            1 => self.creation_time_v1,
            _ => 0,
        }
    }

    /// Returns the active media modification time for the current box version.
    pub fn modification_time(&self) -> u64 {
        match self.version() {
            0 => u64::from(self.modification_time_v0),
            1 => self.modification_time_v1,
            _ => 0,
        }
    }

    /// Returns the active media duration for the current box version.
    pub fn duration(&self) -> u64 {
        match self.version() {
            0 => u64::from(self.duration_v0),
            1 => self.duration_v1,
            _ => 0,
        }
    }
}

impl FieldValueRead for Mdhd {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "CreationTimeV0" => Ok(FieldValue::Unsigned(u64::from(self.creation_time_v0))),
            "ModificationTimeV0" => Ok(FieldValue::Unsigned(u64::from(self.modification_time_v0))),
            "CreationTimeV1" => Ok(FieldValue::Unsigned(self.creation_time_v1)),
            "ModificationTimeV1" => Ok(FieldValue::Unsigned(self.modification_time_v1)),
            "Timescale" => Ok(FieldValue::Unsigned(u64::from(self.timescale))),
            "DurationV0" => Ok(FieldValue::Unsigned(u64::from(self.duration_v0))),
            "DurationV1" => Ok(FieldValue::Unsigned(self.duration_v1)),
            "Pad" => Ok(FieldValue::Boolean(self.pad)),
            "Language" => Ok(FieldValue::UnsignedArray(
                self.language.iter().copied().map(u64::from).collect(),
            )),
            "PreDefined" => Ok(FieldValue::Unsigned(u64::from(self.pre_defined))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Mdhd {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("CreationTimeV0", FieldValue::Unsigned(value)) => {
                self.creation_time_v0 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ModificationTimeV0", FieldValue::Unsigned(value)) => {
                self.modification_time_v0 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("CreationTimeV1", FieldValue::Unsigned(value)) => {
                self.creation_time_v1 = value;
                Ok(())
            }
            ("ModificationTimeV1", FieldValue::Unsigned(value)) => {
                self.modification_time_v1 = value;
                Ok(())
            }
            ("Timescale", FieldValue::Unsigned(value)) => {
                self.timescale = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DurationV0", FieldValue::Unsigned(value)) => {
                self.duration_v0 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DurationV1", FieldValue::Unsigned(value)) => {
                self.duration_v1 = value;
                Ok(())
            }
            ("Pad", FieldValue::Boolean(value)) => {
                self.pad = value;
                Ok(())
            }
            ("Language", FieldValue::UnsignedArray(values)) => {
                if values.len() != 3 {
                    return Err(invalid_value(
                        field_name,
                        "value must contain exactly 3 elements",
                    ));
                }
                self.language = [
                    u8_from_unsigned(field_name, values[0])?,
                    u8_from_unsigned(field_name, values[1])?,
                    u8_from_unsigned(field_name, values[2])?,
                ];
                Ok(())
            }
            ("PreDefined", FieldValue::Unsigned(value)) => {
                self.pre_defined = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Mdhd {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("CreationTimeV0", 2, with_bit_width(32), with_version(0)),
        codec_field!("ModificationTimeV0", 3, with_bit_width(32), with_version(0)),
        codec_field!("CreationTimeV1", 4, with_bit_width(64), with_version(1)),
        codec_field!("ModificationTimeV1", 5, with_bit_width(64), with_version(1)),
        codec_field!("Timescale", 6, with_bit_width(32)),
        codec_field!("DurationV0", 7, with_bit_width(32), with_version(0)),
        codec_field!("DurationV1", 8, with_bit_width(64), with_version(1)),
        codec_field!("Pad", 9, with_bit_width(1), as_boolean(), as_hidden()),
        codec_field!(
            "Language",
            10,
            with_bit_width(5),
            with_length(3),
            as_iso639_2()
        ),
        codec_field!("PreDefined", 11, with_bit_width(16)),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

fn validate_c_string_value(field_name: &'static str, value: &str) -> Result<(), FieldValueError> {
    if value.as_bytes().contains(&0) {
        return Err(invalid_value(
            field_name,
            "value must not contain NUL bytes",
        ));
    }
    Ok(())
}

fn decode_c_string(field_name: &'static str, bytes: &[u8]) -> Result<String, CodecError> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8(bytes[..end].to_vec()).map_err(|_| CodecError::InvalidUtf8 { field_name })
}

fn parse_required_c_string(
    field_name: &'static str,
    bytes: &[u8],
) -> Result<(String, usize), FieldValueError> {
    let Some(end) = bytes.iter().position(|byte| *byte == 0) else {
        return Err(invalid_value(field_name, "string is not NUL-terminated"));
    };

    let value = String::from_utf8(bytes[..end].to_vec())
        .map_err(|_| invalid_value(field_name, "string is not valid UTF-8"))?;
    Ok((value, end + 1))
}

fn decode_required_c_string(
    field_name: &'static str,
    bytes: &[u8],
) -> Result<(String, usize), CodecError> {
    let Some(end) = bytes.iter().position(|byte| *byte == 0) else {
        return Err(invalid_value(field_name, "string is not NUL-terminated").into());
    };

    let value = String::from_utf8(bytes[..end].to_vec())
        .map_err(|_| CodecError::InvalidUtf8 { field_name })?;
    Ok((value, end + 1))
}

fn looks_like_missing_elng_full_box_header(bytes: &[u8]) -> bool {
    let Some(end) = bytes.iter().position(|byte| *byte == 0) else {
        return false;
    };

    end > 0
        && bytes[end..].iter().all(|byte| *byte == 0)
        && bytes[..end]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
}

/// Extended-language box carried alongside `mdhd` when a track uses a language tag that does not
/// fit the compact ISO-639-2 code stored in the media header.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Elng {
    full_box: FullBoxState,
    pub extended_language: String,
    missing_full_box_header: bool,
}

impl FieldHooks for Elng {}

impl ImmutableBox for Elng {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"elng")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for Elng {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
        self.missing_full_box_header = false;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
        self.missing_full_box_header = false;
    }
}

impl FieldValueRead for Elng {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "ExtendedLanguage" => Ok(FieldValue::String(self.extended_language.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Elng {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("ExtendedLanguage", FieldValue::String(value)) => {
                validate_c_string_value(field_name, &value)?;
                self.extended_language = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Elng {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!(
            "ExtendedLanguage",
            2,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated)
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];

    fn custom_marshal(&self, writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        validate_c_string_value("ExtendedLanguage", &self.extended_language)?;
        if self.version() != 0 {
            return Err(CodecError::UnsupportedVersion {
                box_type: self.box_type(),
                version: self.version(),
            });
        }
        if self.flags() != 0 {
            return Err(invalid_value("Flags", "non-zero flags are not supported").into());
        }

        let mut payload = Vec::with_capacity(4 + self.extended_language.len() + 1);
        if !self.missing_full_box_header {
            payload.push(self.version());
            push_uint("Flags", &mut payload, 3, u64::from(self.flags()))?;
        }
        payload.extend_from_slice(self.extended_language.as_bytes());
        payload.push(0);
        writer.write_all(&payload)?;
        Ok(Some(payload.len() as u64))
    }

    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, CodecError> {
        let payload_len = usize::try_from(payload_size)
            .map_err(|_| invalid_value("Payload", "payload is too large to decode"))?;
        let payload = read_exact_vec_untrusted(reader, payload_len).map_err(CodecError::Io)?;

        if (payload.len() < 4 || !payload.starts_with(&[0, 0, 0, 0]))
            && looks_like_missing_elng_full_box_header(&payload)
        {
            self.full_box = FullBoxState::default();
            self.extended_language = decode_c_string("ExtendedLanguage", &payload)?;
            self.missing_full_box_header = true;
            return Ok(Some(payload_size));
        }

        if payload.len() < 4 {
            return Err(invalid_value("Payload", "payload is too short").into());
        }

        let version = payload[0];
        if version != 0 {
            return Err(CodecError::UnsupportedVersion {
                box_type: self.box_type(),
                version,
            });
        }
        let flags = read_uint(&payload, 1, 3) as u32;
        if flags != 0 {
            return Err(invalid_value("Flags", "non-zero flags are not supported").into());
        }

        self.full_box = FullBoxState { version, flags };
        self.extended_language = decode_c_string("ExtendedLanguage", &payload[4..])?;
        self.missing_full_box_header = false;
        Ok(Some(payload_size))
    }
}

/// Movie header box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mvhd {
    full_box: FullBoxState,
    pub creation_time_v0: u32,
    pub modification_time_v0: u32,
    pub creation_time_v1: u64,
    pub modification_time_v1: u64,
    pub timescale: u32,
    pub duration_v0: u32,
    pub duration_v1: u64,
    pub rate: i32,
    pub volume: i16,
    pub matrix: [i32; 9],
    pub pre_defined: [i32; 6],
    pub next_track_id: u32,
}

impl_full_box!(Mvhd, *b"mvhd");

impl FieldHooks for Mvhd {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Rate" => Some(format_fixed_16_16_signed(self.rate)),
            _ => None,
        }
    }
}

impl Mvhd {
    /// Returns the active movie creation time for the current box version.
    pub fn creation_time(&self) -> u64 {
        match self.version() {
            0 => u64::from(self.creation_time_v0),
            1 => self.creation_time_v1,
            _ => 0,
        }
    }

    /// Returns the active movie modification time for the current box version.
    pub fn modification_time(&self) -> u64 {
        match self.version() {
            0 => u64::from(self.modification_time_v0),
            1 => self.modification_time_v1,
            _ => 0,
        }
    }

    /// Returns the active movie duration for the current box version.
    pub fn duration(&self) -> u64 {
        match self.version() {
            0 => u64::from(self.duration_v0),
            1 => self.duration_v1,
            _ => 0,
        }
    }

    /// Returns the playback rate as a signed 16.16 fixed-point value.
    pub fn rate_value(&self) -> f64 {
        f64::from(self.rate) / 65536.0
    }

    /// Returns the integer component of the playback rate.
    pub fn rate_int(&self) -> i16 {
        (self.rate >> 16) as i16
    }
}

impl FieldValueRead for Mvhd {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "CreationTimeV0" => Ok(FieldValue::Unsigned(u64::from(self.creation_time_v0))),
            "ModificationTimeV0" => Ok(FieldValue::Unsigned(u64::from(self.modification_time_v0))),
            "CreationTimeV1" => Ok(FieldValue::Unsigned(self.creation_time_v1)),
            "ModificationTimeV1" => Ok(FieldValue::Unsigned(self.modification_time_v1)),
            "Timescale" => Ok(FieldValue::Unsigned(u64::from(self.timescale))),
            "DurationV0" => Ok(FieldValue::Unsigned(u64::from(self.duration_v0))),
            "DurationV1" => Ok(FieldValue::Unsigned(self.duration_v1)),
            "Rate" => Ok(FieldValue::Signed(i64::from(self.rate))),
            "Volume" => Ok(FieldValue::Signed(i64::from(self.volume))),
            "Reserved2" => Ok(FieldValue::Bytes(vec![0; 8])),
            "Matrix" => Ok(FieldValue::SignedArray(
                self.matrix.iter().copied().map(i64::from).collect(),
            )),
            "PreDefined" => Ok(FieldValue::SignedArray(
                self.pre_defined.iter().copied().map(i64::from).collect(),
            )),
            "NextTrackID" => Ok(FieldValue::Unsigned(u64::from(self.next_track_id))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Mvhd {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("CreationTimeV0", FieldValue::Unsigned(value)) => {
                self.creation_time_v0 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ModificationTimeV0", FieldValue::Unsigned(value)) => {
                self.modification_time_v0 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("CreationTimeV1", FieldValue::Unsigned(value)) => {
                self.creation_time_v1 = value;
                Ok(())
            }
            ("ModificationTimeV1", FieldValue::Unsigned(value)) => {
                self.modification_time_v1 = value;
                Ok(())
            }
            ("Timescale", FieldValue::Unsigned(value)) => {
                self.timescale = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DurationV0", FieldValue::Unsigned(value)) => {
                self.duration_v0 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DurationV1", FieldValue::Unsigned(value)) => {
                self.duration_v1 = value;
                Ok(())
            }
            ("Rate", FieldValue::Signed(value)) => {
                self.rate = i32_from_signed(field_name, value)?;
                Ok(())
            }
            ("Volume", FieldValue::Signed(value)) => {
                self.volume = i16_from_signed(field_name, value)?;
                Ok(())
            }
            ("Reserved2", FieldValue::Bytes(bytes)) => bytes_to_zeroes(field_name, &bytes, 8),
            ("Matrix", FieldValue::SignedArray(values)) => {
                if values.len() != 9 {
                    return Err(invalid_value(
                        field_name,
                        "value must contain exactly 9 elements",
                    ));
                }
                for (slot, value) in self.matrix.iter_mut().zip(values) {
                    *slot = i32_from_signed(field_name, value)?;
                }
                Ok(())
            }
            ("PreDefined", FieldValue::SignedArray(values)) => {
                if values.len() != 6 {
                    return Err(invalid_value(
                        field_name,
                        "value must contain exactly 6 elements",
                    ));
                }
                for (slot, value) in self.pre_defined.iter_mut().zip(values) {
                    *slot = i32_from_signed(field_name, value)?;
                }
                Ok(())
            }
            ("NextTrackID", FieldValue::Unsigned(value)) => {
                self.next_track_id = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Mvhd {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("CreationTimeV0", 2, with_bit_width(32), with_version(0)),
        codec_field!("ModificationTimeV0", 3, with_bit_width(32), with_version(0)),
        codec_field!("CreationTimeV1", 4, with_bit_width(64), with_version(1)),
        codec_field!("ModificationTimeV1", 5, with_bit_width(64), with_version(1)),
        codec_field!("Timescale", 6, with_bit_width(32)),
        codec_field!("DurationV0", 7, with_bit_width(32), with_version(0)),
        codec_field!("DurationV1", 8, with_bit_width(64), with_version(1)),
        codec_field!("Rate", 9, with_bit_width(32), as_signed()),
        codec_field!("Volume", 10, with_bit_width(16), as_signed()),
        codec_field!("Reserved", 11, with_bit_width(16), with_constant("0")),
        codec_field!(
            "Reserved2",
            12,
            with_bit_width(8),
            with_length(8),
            as_bytes(),
            as_hidden()
        ),
        codec_field!(
            "Matrix",
            13,
            with_bit_width(32),
            with_length(9),
            as_signed(),
            as_hex()
        ),
        codec_field!(
            "PreDefined",
            14,
            with_bit_width(32),
            with_length(6),
            as_signed()
        ),
        codec_field!("NextTrackID", 15, with_bit_width(32)),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// Track fragment decode time box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tfdt {
    full_box: FullBoxState,
    pub base_media_decode_time_v0: u32,
    pub base_media_decode_time_v1: u64,
}

impl_full_box!(Tfdt, *b"tfdt");

impl Tfdt {
    /// Returns the active base media decode time for the current box version.
    pub fn base_media_decode_time(&self) -> u64 {
        match self.version() {
            0 => u64::from(self.base_media_decode_time_v0),
            1 => self.base_media_decode_time_v1,
            _ => 0,
        }
    }
}

impl FieldValueRead for Tfdt {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "BaseMediaDecodeTimeV0" => Ok(FieldValue::Unsigned(u64::from(
                self.base_media_decode_time_v0,
            ))),
            "BaseMediaDecodeTimeV1" => Ok(FieldValue::Unsigned(self.base_media_decode_time_v1)),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Tfdt {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("BaseMediaDecodeTimeV0", FieldValue::Unsigned(value)) => {
                self.base_media_decode_time_v0 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("BaseMediaDecodeTimeV1", FieldValue::Unsigned(value)) => {
                self.base_media_decode_time_v1 = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Tfdt {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!(
            "BaseMediaDecodeTimeV0",
            2,
            with_bit_width(32),
            with_version(0)
        ),
        codec_field!(
            "BaseMediaDecodeTimeV1",
            3,
            with_bit_width(64),
            with_version(1)
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// Track fragment header box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tfhd {
    full_box: FullBoxState,
    pub track_id: u32,
    pub base_data_offset: u64,
    pub sample_description_index: u32,
    pub default_sample_duration: u32,
    pub default_sample_size: u32,
    pub default_sample_flags: u32,
}

impl_full_box!(Tfhd, *b"tfhd");

impl FieldValueRead for Tfhd {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "TrackID" => Ok(FieldValue::Unsigned(u64::from(self.track_id))),
            "BaseDataOffset" => Ok(FieldValue::Unsigned(self.base_data_offset)),
            "SampleDescriptionIndex" => Ok(FieldValue::Unsigned(u64::from(
                self.sample_description_index,
            ))),
            "DefaultSampleDuration" => Ok(FieldValue::Unsigned(u64::from(
                self.default_sample_duration,
            ))),
            "DefaultSampleSize" => Ok(FieldValue::Unsigned(u64::from(self.default_sample_size))),
            "DefaultSampleFlags" => Ok(FieldValue::Unsigned(u64::from(self.default_sample_flags))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Tfhd {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("TrackID", FieldValue::Unsigned(value)) => {
                self.track_id = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("BaseDataOffset", FieldValue::Unsigned(value)) => {
                self.base_data_offset = value;
                Ok(())
            }
            ("SampleDescriptionIndex", FieldValue::Unsigned(value)) => {
                self.sample_description_index = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DefaultSampleDuration", FieldValue::Unsigned(value)) => {
                self.default_sample_duration = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DefaultSampleSize", FieldValue::Unsigned(value)) => {
                self.default_sample_size = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DefaultSampleFlags", FieldValue::Unsigned(value)) => {
                self.default_sample_flags = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Tfhd {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("TrackID", 2, with_bit_width(32)),
        codec_field!(
            "BaseDataOffset",
            3,
            with_bit_width(64),
            with_required_flags(TFHD_BASE_DATA_OFFSET_PRESENT)
        ),
        codec_field!(
            "SampleDescriptionIndex",
            4,
            with_bit_width(32),
            with_required_flags(TFHD_SAMPLE_DESCRIPTION_INDEX_PRESENT)
        ),
        codec_field!(
            "DefaultSampleDuration",
            5,
            with_bit_width(32),
            with_required_flags(TFHD_DEFAULT_SAMPLE_DURATION_PRESENT)
        ),
        codec_field!(
            "DefaultSampleSize",
            6,
            with_bit_width(32),
            with_required_flags(TFHD_DEFAULT_SAMPLE_SIZE_PRESENT)
        ),
        codec_field!(
            "DefaultSampleFlags",
            7,
            with_bit_width(32),
            with_required_flags(TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT),
            as_hex()
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Track header box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tkhd {
    full_box: FullBoxState,
    pub creation_time_v0: u32,
    pub modification_time_v0: u32,
    pub creation_time_v1: u64,
    pub modification_time_v1: u64,
    pub track_id: u32,
    pub duration_v0: u32,
    pub duration_v1: u64,
    pub layer: i16,
    pub alternate_group: i16,
    pub volume: i16,
    pub matrix: [i32; 9],
    pub width: u32,
    pub height: u32,
}

impl FieldHooks for Tkhd {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Width" => Some(format_fixed_16_16_unsigned(self.width)),
            "Height" => Some(format_fixed_16_16_unsigned(self.height)),
            _ => None,
        }
    }
}

impl ImmutableBox for Tkhd {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"tkhd")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for Tkhd {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }
}

impl Tkhd {
    /// Returns the active track creation time for the current box version.
    pub fn creation_time(&self) -> u64 {
        match self.version() {
            0 => u64::from(self.creation_time_v0),
            1 => self.creation_time_v1,
            _ => 0,
        }
    }

    /// Returns the active track modification time for the current box version.
    pub fn modification_time(&self) -> u64 {
        match self.version() {
            0 => u64::from(self.modification_time_v0),
            1 => self.modification_time_v1,
            _ => 0,
        }
    }

    /// Returns the active track duration for the current box version.
    pub fn duration(&self) -> u64 {
        match self.version() {
            0 => u64::from(self.duration_v0),
            1 => self.duration_v1,
            _ => 0,
        }
    }

    /// Returns the track width as an unsigned 16.16 fixed-point value.
    pub fn width_value(&self) -> f64 {
        f64::from(self.width) / 65536.0
    }

    /// Returns the integer component of the track width.
    pub fn width_int(&self) -> u16 {
        (self.width >> 16) as u16
    }

    /// Returns the track height as an unsigned 16.16 fixed-point value.
    pub fn height_value(&self) -> f64 {
        f64::from(self.height) / 65536.0
    }

    /// Returns the integer component of the track height.
    pub fn height_int(&self) -> u16 {
        (self.height >> 16) as u16
    }
}

impl FieldValueRead for Tkhd {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "CreationTimeV0" => Ok(FieldValue::Unsigned(u64::from(self.creation_time_v0))),
            "ModificationTimeV0" => Ok(FieldValue::Unsigned(u64::from(self.modification_time_v0))),
            "CreationTimeV1" => Ok(FieldValue::Unsigned(self.creation_time_v1)),
            "ModificationTimeV1" => Ok(FieldValue::Unsigned(self.modification_time_v1)),
            "TrackID" => Ok(FieldValue::Unsigned(u64::from(self.track_id))),
            "DurationV0" => Ok(FieldValue::Unsigned(u64::from(self.duration_v0))),
            "DurationV1" => Ok(FieldValue::Unsigned(self.duration_v1)),
            "Reserved1" => Ok(FieldValue::Bytes(vec![0; 8])),
            "Layer" => Ok(FieldValue::Signed(i64::from(self.layer))),
            "AlternateGroup" => Ok(FieldValue::Signed(i64::from(self.alternate_group))),
            "Volume" => Ok(FieldValue::Signed(i64::from(self.volume))),
            "Matrix" => Ok(FieldValue::SignedArray(
                self.matrix.iter().copied().map(i64::from).collect(),
            )),
            "Width" => Ok(FieldValue::Unsigned(u64::from(self.width))),
            "Height" => Ok(FieldValue::Unsigned(u64::from(self.height))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Tkhd {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("CreationTimeV0", FieldValue::Unsigned(value)) => {
                self.creation_time_v0 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ModificationTimeV0", FieldValue::Unsigned(value)) => {
                self.modification_time_v0 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("CreationTimeV1", FieldValue::Unsigned(value)) => {
                self.creation_time_v1 = value;
                Ok(())
            }
            ("ModificationTimeV1", FieldValue::Unsigned(value)) => {
                self.modification_time_v1 = value;
                Ok(())
            }
            ("TrackID", FieldValue::Unsigned(value)) => {
                self.track_id = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DurationV0", FieldValue::Unsigned(value)) => {
                self.duration_v0 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DurationV1", FieldValue::Unsigned(value)) => {
                self.duration_v1 = value;
                Ok(())
            }
            ("Reserved1", FieldValue::Bytes(bytes)) => bytes_to_zeroes(field_name, &bytes, 8),
            ("Layer", FieldValue::Signed(value)) => {
                self.layer = i16_from_signed(field_name, value)?;
                Ok(())
            }
            ("AlternateGroup", FieldValue::Signed(value)) => {
                self.alternate_group = i16_from_signed(field_name, value)?;
                Ok(())
            }
            ("Volume", FieldValue::Signed(value)) => {
                self.volume = i16_from_signed(field_name, value)?;
                Ok(())
            }
            ("Matrix", FieldValue::SignedArray(values)) => {
                if values.len() != 9 {
                    return Err(invalid_value(
                        field_name,
                        "value must contain exactly 9 elements",
                    ));
                }
                for (slot, value) in self.matrix.iter_mut().zip(values) {
                    *slot = i32_from_signed(field_name, value)?;
                }
                Ok(())
            }
            ("Width", FieldValue::Unsigned(value)) => {
                self.width = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Height", FieldValue::Unsigned(value)) => {
                self.height = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Tkhd {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("CreationTimeV0", 2, with_bit_width(32), with_version(0)),
        codec_field!("ModificationTimeV0", 3, with_bit_width(32), with_version(0)),
        codec_field!("CreationTimeV1", 4, with_bit_width(64), with_version(1)),
        codec_field!("ModificationTimeV1", 5, with_bit_width(64), with_version(1)),
        codec_field!("TrackID", 6, with_bit_width(32)),
        codec_field!("Reserved0", 7, with_bit_width(32), with_constant("0")),
        codec_field!("DurationV0", 8, with_bit_width(32), with_version(0)),
        codec_field!("DurationV1", 9, with_bit_width(64), with_version(1)),
        codec_field!(
            "Reserved1",
            10,
            with_bit_width(8),
            with_length(8),
            as_bytes(),
            as_hidden()
        ),
        codec_field!("Layer", 11, with_bit_width(16), as_signed()),
        codec_field!("AlternateGroup", 12, with_bit_width(16), as_signed()),
        codec_field!("Volume", 13, with_bit_width(16), as_signed()),
        codec_field!("Reserved2", 14, with_bit_width(16), with_constant("0")),
        codec_field!(
            "Matrix",
            15,
            with_bit_width(32),
            with_length(9),
            as_signed(),
            as_hex()
        ),
        codec_field!("Width", 16, with_bit_width(32)),
        codec_field!("Height", 17, with_bit_width(32)),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// One level-assignment record carried by [`Leva`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LevaLevel {
    pub track_id: u32,
    pub padding_flag: bool,
    pub assignment_type: u8,
    pub grouping_type: u32,
    pub grouping_type_parameter: u32,
    pub sub_track_id: u32,
}

/// Level-assignment box used by track-extension property paths.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Leva {
    full_box: FullBoxState,
    pub level_count: u8,
    pub levels: Vec<LevaLevel>,
}

fn format_leva_levels(levels: &[LevaLevel]) -> String {
    render_array(levels.iter().map(|level| match level.assignment_type {
        0 => format!(
            "{{TrackID={} PaddingFlag={} AssignmentType={} GroupingType=0x{:08x}}}",
            level.track_id, level.padding_flag, level.assignment_type, level.grouping_type
        ),
        1 => format!(
            "{{TrackID={} PaddingFlag={} AssignmentType={} GroupingType=0x{:08x} GroupingTypeParameter={}}}",
            level.track_id,
            level.padding_flag,
            level.assignment_type,
            level.grouping_type,
            level.grouping_type_parameter
        ),
        4 => format!(
            "{{TrackID={} PaddingFlag={} AssignmentType={} SubTrackID={}}}",
            level.track_id, level.padding_flag, level.assignment_type, level.sub_track_id
        ),
        _ => format!(
            "{{TrackID={} PaddingFlag={} AssignmentType={}}}",
            level.track_id, level.padding_flag, level.assignment_type
        ),
    }))
}

fn encode_leva_levels(
    field_name: &'static str,
    levels: &[LevaLevel],
) -> Result<Vec<u8>, FieldValueError> {
    let mut bytes = Vec::new();
    for level in levels {
        if level.assignment_type > 4 {
            return Err(invalid_value(
                field_name,
                "assignment type uses a reserved layout",
            ));
        }

        bytes.extend_from_slice(&level.track_id.to_be_bytes());
        bytes.push((u8::from(level.padding_flag) << 7) | level.assignment_type);
        match level.assignment_type {
            0 => bytes.extend_from_slice(&level.grouping_type.to_be_bytes()),
            1 => {
                bytes.extend_from_slice(&level.grouping_type.to_be_bytes());
                bytes.extend_from_slice(&level.grouping_type_parameter.to_be_bytes());
            }
            2 | 3 => {}
            4 => bytes.extend_from_slice(&level.sub_track_id.to_be_bytes()),
            _ => unreachable!(),
        }
    }
    Ok(bytes)
}

fn parse_leva_levels(
    field_name: &'static str,
    level_count: u8,
    bytes: &[u8],
) -> Result<Vec<LevaLevel>, FieldValueError> {
    let mut levels = Vec::with_capacity(untrusted_prealloc_hint(usize::from(level_count)));
    let mut offset = 0usize;

    for _ in 0..level_count {
        if bytes.len().saturating_sub(offset) < 5 {
            return Err(invalid_value(field_name, "level payload is truncated"));
        }

        let track_id = read_u32(bytes, offset);
        let assignment_header = bytes[offset + 4];
        offset += 5;

        let padding_flag = assignment_header & 0x80 != 0;
        let assignment_type = assignment_header & 0x7f;
        let mut level = LevaLevel {
            track_id,
            padding_flag,
            assignment_type,
            ..LevaLevel::default()
        };

        match assignment_type {
            0 => {
                if bytes.len().saturating_sub(offset) < 4 {
                    return Err(invalid_value(
                        field_name,
                        "grouping type payload is truncated",
                    ));
                }
                level.grouping_type = read_u32(bytes, offset);
                offset += 4;
            }
            1 => {
                if bytes.len().saturating_sub(offset) < 8 {
                    return Err(invalid_value(
                        field_name,
                        "grouping type parameter payload is truncated",
                    ));
                }
                level.grouping_type = read_u32(bytes, offset);
                level.grouping_type_parameter = read_u32(bytes, offset + 4);
                offset += 8;
            }
            2 | 3 => {}
            4 => {
                if bytes.len().saturating_sub(offset) < 4 {
                    return Err(invalid_value(field_name, "sub-track payload is truncated"));
                }
                level.sub_track_id = read_u32(bytes, offset);
                offset += 4;
            }
            _ => {
                return Err(invalid_value(
                    field_name,
                    "assignment type uses a reserved layout",
                ));
            }
        }

        levels.push(level);
    }

    if offset != bytes.len() {
        return Err(invalid_value(
            field_name,
            "level payload length does not match the level count",
        ));
    }

    Ok(levels)
}

impl FieldHooks for Leva {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Levels" => Some(format_leva_levels(&self.levels)),
            _ => None,
        }
    }
}

impl_full_box!(Leva, *b"leva");

impl FieldValueRead for Leva {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "LevelCount" => Ok(FieldValue::Unsigned(u64::from(self.level_count))),
            "Levels" => {
                require_count(field_name, u32::from(self.level_count), self.levels.len())?;
                Ok(FieldValue::Bytes(encode_leva_levels(
                    field_name,
                    &self.levels,
                )?))
            }
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Leva {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("LevelCount", FieldValue::Unsigned(value)) => {
                self.level_count = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Levels", FieldValue::Bytes(bytes)) => {
                self.levels = parse_leva_levels(field_name, self.level_count, &bytes)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Leva {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("LevelCount", 2, with_bit_width(8)),
        codec_field!("Levels", 3, with_bit_width(8), as_bytes()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

track_id_list_box!(
    Cdsc,
    *b"cdsc",
    "Track-link type box for content-description links."
);
track_id_list_box!(
    Dpnd,
    *b"dpnd",
    "Track-link type box for track-dependency links."
);
track_id_list_box!(Font, *b"font", "Track-link type box for font links.");
track_id_list_box!(Hind, *b"hind", "Track-link type box for hint dependencies.");
track_id_list_box!(Hint, *b"hint", "Track-link type box for hint-track links.");
track_id_list_box!(
    Ipir,
    *b"ipir",
    "Track-link type box for auxiliary-picture links."
);
track_id_list_box!(
    Mpod,
    *b"mpod",
    "Track-link type box for metadata-pod links."
);
track_id_list_box!(Subt, *b"subt", "Track-link type box for subtitle links.");
track_id_list_box!(
    Sync,
    *b"sync",
    "Track-link type box for synchronized-track links."
);
track_id_list_box!(
    Vdep,
    *b"vdep",
    "Track-link type box for video-dependency links."
);
track_id_list_box!(
    Vplx,
    *b"vplx",
    "Track-link type box for video-multiplex links."
);

/// Track extension properties box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Trep {
    full_box: FullBoxState,
    pub track_id: u32,
}

impl_full_box!(Trep, *b"trep");

impl FieldValueRead for Trep {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "TrackID" => Ok(FieldValue::Unsigned(u64::from(self.track_id))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Trep {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("TrackID", FieldValue::Unsigned(value)) => {
                self.track_id = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Trep {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("TrackID", 2, with_bit_width(32)),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Track extends defaults box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Trex {
    full_box: FullBoxState,
    pub track_id: u32,
    pub default_sample_description_index: u32,
    pub default_sample_duration: u32,
    pub default_sample_size: u32,
    pub default_sample_flags: u32,
}

impl_full_box!(Trex, *b"trex");

impl FieldValueRead for Trex {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "TrackID" => Ok(FieldValue::Unsigned(u64::from(self.track_id))),
            "DefaultSampleDescriptionIndex" => Ok(FieldValue::Unsigned(u64::from(
                self.default_sample_description_index,
            ))),
            "DefaultSampleDuration" => Ok(FieldValue::Unsigned(u64::from(
                self.default_sample_duration,
            ))),
            "DefaultSampleSize" => Ok(FieldValue::Unsigned(u64::from(self.default_sample_size))),
            "DefaultSampleFlags" => Ok(FieldValue::Unsigned(u64::from(self.default_sample_flags))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Trex {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("TrackID", FieldValue::Unsigned(value)) => {
                self.track_id = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DefaultSampleDescriptionIndex", FieldValue::Unsigned(value)) => {
                self.default_sample_description_index = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DefaultSampleDuration", FieldValue::Unsigned(value)) => {
                self.default_sample_duration = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DefaultSampleSize", FieldValue::Unsigned(value)) => {
                self.default_sample_size = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DefaultSampleFlags", FieldValue::Unsigned(value)) => {
                self.default_sample_flags = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Trex {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("TrackID", 2, with_bit_width(32)),
        codec_field!("DefaultSampleDescriptionIndex", 3, with_bit_width(32)),
        codec_field!("DefaultSampleDuration", 4, with_bit_width(32)),
        codec_field!("DefaultSampleSize", 5, with_bit_width(32)),
        codec_field!("DefaultSampleFlags", 6, with_bit_width(32), as_hex()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Video media header box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Vmhd {
    full_box: FullBoxState,
    pub graphicsmode: u16,
    pub opcolor: [u16; 3],
}

impl_full_box!(Vmhd, *b"vmhd");

impl FieldValueRead for Vmhd {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Graphicsmode" => Ok(FieldValue::Unsigned(u64::from(self.graphicsmode))),
            "Opcolor" => Ok(FieldValue::UnsignedArray(
                self.opcolor.iter().copied().map(u64::from).collect(),
            )),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Vmhd {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Graphicsmode", FieldValue::Unsigned(value)) => {
                self.graphicsmode = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Opcolor", FieldValue::UnsignedArray(values)) => {
                if values.len() != 3 {
                    return Err(invalid_value(
                        field_name,
                        "value must contain exactly 3 elements",
                    ));
                }
                self.opcolor = [
                    u16_from_unsigned(field_name, values[0])?,
                    u16_from_unsigned(field_name, values[1])?,
                    u16_from_unsigned(field_name, values[2])?,
                ];
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Vmhd {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("Graphicsmode", 2, with_bit_width(16)),
        codec_field!("Opcolor", 3, with_bit_width(16), with_length(3)),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Sound media header box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Smhd {
    full_box: FullBoxState,
    pub balance: i16,
}

impl FieldHooks for Smhd {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Balance" => Some(format_fixed_8_8_signed(self.balance)),
            _ => None,
        }
    }
}

impl ImmutableBox for Smhd {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"smhd")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for Smhd {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }
}

impl Smhd {
    /// Returns the balance as a signed 8.8 fixed-point value.
    pub fn balance_value(&self) -> f32 {
        f32::from(self.balance) / 256.0
    }

    /// Returns the integer component of the balance.
    pub fn balance_int(&self) -> i8 {
        (self.balance >> 8) as i8
    }
}

impl FieldValueRead for Smhd {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Balance" => Ok(FieldValue::Signed(i64::from(self.balance))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Smhd {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Balance", FieldValue::Signed(value)) => {
                self.balance = i16_from_signed(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Smhd {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("Balance", 2, with_bit_width(16), as_signed()),
        codec_field!("Reserved", 3, with_bit_width(16), with_constant("0")),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Subtitle media header box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sthd {
    full_box: FullBoxState,
}

impl_full_box!(Sthd, *b"sthd");
empty_hooks!(Sthd);
empty_full_box_codec!(Sthd);

/// Null media header box that carries no additional media-header fields.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Nmhd {
    full_box: FullBoxState,
}

impl_full_box!(Nmhd, *b"nmhd");
empty_hooks!(Nmhd);
empty_full_box_codec!(Nmhd);

/// Sample description box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stsd {
    full_box: FullBoxState,
    pub entry_count: u32,
}

impl_full_box!(Stsd, *b"stsd");

impl FieldValueRead for Stsd {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "EntryCount" => Ok(FieldValue::Unsigned(u64::from(self.entry_count))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Stsd {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("EntryCount", FieldValue::Unsigned(value)) => {
                self.entry_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Stsd {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("EntryCount", 2, with_bit_width(32)),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Composition-to-decode timeline shift box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cslg {
    full_box: FullBoxState,
    pub composition_to_dts_shift_v0: i32,
    pub least_decode_to_display_delta_v0: i32,
    pub greatest_decode_to_display_delta_v0: i32,
    pub composition_start_time_v0: i32,
    pub composition_end_time_v0: i32,
    pub composition_to_dts_shift_v1: i64,
    pub least_decode_to_display_delta_v1: i64,
    pub greatest_decode_to_display_delta_v1: i64,
    pub composition_start_time_v1: i64,
    pub composition_end_time_v1: i64,
}

impl_full_box!(Cslg, *b"cslg");

impl Cslg {
    /// Returns the active composition-to-decode shift for the current box version.
    pub fn composition_to_dts_shift(&self) -> i64 {
        match self.version() {
            0 => i64::from(self.composition_to_dts_shift_v0),
            1 => self.composition_to_dts_shift_v1,
            _ => 0,
        }
    }

    /// Returns the active least decode-to-display delta for the current box version.
    pub fn least_decode_to_display_delta(&self) -> i64 {
        match self.version() {
            0 => i64::from(self.least_decode_to_display_delta_v0),
            1 => self.least_decode_to_display_delta_v1,
            _ => 0,
        }
    }

    /// Returns the active greatest decode-to-display delta for the current box version.
    pub fn greatest_decode_to_display_delta(&self) -> i64 {
        match self.version() {
            0 => i64::from(self.greatest_decode_to_display_delta_v0),
            1 => self.greatest_decode_to_display_delta_v1,
            _ => 0,
        }
    }

    /// Returns the active composition start time for the current box version.
    pub fn composition_start_time(&self) -> i64 {
        match self.version() {
            0 => i64::from(self.composition_start_time_v0),
            1 => self.composition_start_time_v1,
            _ => 0,
        }
    }

    /// Returns the active composition end time for the current box version.
    pub fn composition_end_time(&self) -> i64 {
        match self.version() {
            0 => i64::from(self.composition_end_time_v0),
            1 => self.composition_end_time_v1,
            _ => 0,
        }
    }
}

impl FieldValueRead for Cslg {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "CompositionToDTSShiftV0" => Ok(FieldValue::Signed(i64::from(
                self.composition_to_dts_shift_v0,
            ))),
            "LeastDecodeToDisplayDeltaV0" => Ok(FieldValue::Signed(i64::from(
                self.least_decode_to_display_delta_v0,
            ))),
            "GreatestDecodeToDisplayDeltaV0" => Ok(FieldValue::Signed(i64::from(
                self.greatest_decode_to_display_delta_v0,
            ))),
            "CompositionStartTimeV0" => Ok(FieldValue::Signed(i64::from(
                self.composition_start_time_v0,
            ))),
            "CompositionEndTimeV0" => {
                Ok(FieldValue::Signed(i64::from(self.composition_end_time_v0)))
            }
            "CompositionToDTSShiftV1" => Ok(FieldValue::Signed(self.composition_to_dts_shift_v1)),
            "LeastDecodeToDisplayDeltaV1" => {
                Ok(FieldValue::Signed(self.least_decode_to_display_delta_v1))
            }
            "GreatestDecodeToDisplayDeltaV1" => {
                Ok(FieldValue::Signed(self.greatest_decode_to_display_delta_v1))
            }
            "CompositionStartTimeV1" => Ok(FieldValue::Signed(self.composition_start_time_v1)),
            "CompositionEndTimeV1" => Ok(FieldValue::Signed(self.composition_end_time_v1)),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Cslg {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("CompositionToDTSShiftV0", FieldValue::Signed(value)) => {
                self.composition_to_dts_shift_v0 = i32_from_signed(field_name, value)?;
                Ok(())
            }
            ("LeastDecodeToDisplayDeltaV0", FieldValue::Signed(value)) => {
                self.least_decode_to_display_delta_v0 = i32_from_signed(field_name, value)?;
                Ok(())
            }
            ("GreatestDecodeToDisplayDeltaV0", FieldValue::Signed(value)) => {
                self.greatest_decode_to_display_delta_v0 = i32_from_signed(field_name, value)?;
                Ok(())
            }
            ("CompositionStartTimeV0", FieldValue::Signed(value)) => {
                self.composition_start_time_v0 = i32_from_signed(field_name, value)?;
                Ok(())
            }
            ("CompositionEndTimeV0", FieldValue::Signed(value)) => {
                self.composition_end_time_v0 = i32_from_signed(field_name, value)?;
                Ok(())
            }
            ("CompositionToDTSShiftV1", FieldValue::Signed(value)) => {
                self.composition_to_dts_shift_v1 = i64_from_signed(field_name, value)?;
                Ok(())
            }
            ("LeastDecodeToDisplayDeltaV1", FieldValue::Signed(value)) => {
                self.least_decode_to_display_delta_v1 = i64_from_signed(field_name, value)?;
                Ok(())
            }
            ("GreatestDecodeToDisplayDeltaV1", FieldValue::Signed(value)) => {
                self.greatest_decode_to_display_delta_v1 = i64_from_signed(field_name, value)?;
                Ok(())
            }
            ("CompositionStartTimeV1", FieldValue::Signed(value)) => {
                self.composition_start_time_v1 = i64_from_signed(field_name, value)?;
                Ok(())
            }
            ("CompositionEndTimeV1", FieldValue::Signed(value)) => {
                self.composition_end_time_v1 = i64_from_signed(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Cslg {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!(
            "CompositionToDTSShiftV0",
            2,
            with_bit_width(32),
            with_version(0),
            as_signed()
        ),
        codec_field!(
            "LeastDecodeToDisplayDeltaV0",
            3,
            with_bit_width(32),
            with_version(0),
            as_signed()
        ),
        codec_field!(
            "GreatestDecodeToDisplayDeltaV0",
            4,
            with_bit_width(32),
            with_version(0),
            as_signed()
        ),
        codec_field!(
            "CompositionStartTimeV0",
            5,
            with_bit_width(32),
            with_version(0),
            as_signed()
        ),
        codec_field!(
            "CompositionEndTimeV0",
            6,
            with_bit_width(32),
            with_version(0),
            as_signed()
        ),
        codec_field!(
            "CompositionToDTSShiftV1",
            7,
            with_bit_width(64),
            with_version(1),
            as_signed()
        ),
        codec_field!(
            "LeastDecodeToDisplayDeltaV1",
            8,
            with_bit_width(64),
            with_version(1),
            as_signed()
        ),
        codec_field!(
            "GreatestDecodeToDisplayDeltaV1",
            9,
            with_bit_width(64),
            with_version(1),
            as_signed()
        ),
        codec_field!(
            "CompositionStartTimeV1",
            10,
            with_bit_width(64),
            with_version(1),
            as_signed()
        ),
        codec_field!(
            "CompositionEndTimeV1",
            11,
            with_bit_width(64),
            with_version(1),
            as_signed()
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// One composition-offset run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CttsEntry {
    pub sample_count: u32,
    pub sample_offset_v0: u32,
    pub sample_offset_v1: i32,
}

/// Composition time to sample box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ctts {
    full_box: FullBoxState,
    pub entry_count: u32,
    pub entries: Vec<CttsEntry>,
}

impl FieldHooks for Ctts {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "Entries" => usize::try_from(self.entry_count)
                .ok()
                .and_then(|count| field_len_bytes(count, 8)),
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Entries" => Some(render_array(self.entries.iter().map(
                |entry| match self.version() {
                    0 => format!(
                        "{{SampleCount={} SampleOffsetV0={}}}",
                        entry.sample_count, entry.sample_offset_v0
                    ),
                    1 => format!(
                        "{{SampleCount={} SampleOffsetV1={}}}",
                        entry.sample_count, entry.sample_offset_v1
                    ),
                    _ => String::from("{}"),
                },
            ))),
            _ => None,
        }
    }
}

impl ImmutableBox for Ctts {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"ctts")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for Ctts {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }
}

impl Ctts {
    /// Returns the active sample offset for `index`.
    pub fn sample_offset(&self, index: usize) -> i64 {
        match self.version() {
            0 => i64::from(self.entries[index].sample_offset_v0),
            1 => i64::from(self.entries[index].sample_offset_v1),
            _ => 0,
        }
    }
}

impl FieldValueRead for Ctts {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "EntryCount" => Ok(FieldValue::Unsigned(u64::from(self.entry_count))),
            "Entries" => {
                let mut bytes = Vec::with_capacity(self.entries.len() * 8);
                for entry in &self.entries {
                    bytes.extend_from_slice(&entry.sample_count.to_be_bytes());
                    match self.version() {
                        0 => bytes.extend_from_slice(&entry.sample_offset_v0.to_be_bytes()),
                        1 => bytes.extend_from_slice(&entry.sample_offset_v1.to_be_bytes()),
                        _ => {}
                    }
                }
                Ok(FieldValue::Bytes(bytes))
            }
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Ctts {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("EntryCount", FieldValue::Unsigned(value)) => {
                self.entry_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Entries", FieldValue::Bytes(bytes)) => {
                self.entries =
                    parse_fixed_chunks(field_name, &bytes, 8, |chunk| match self.version() {
                        0 => CttsEntry {
                            sample_count: read_u32(chunk, 0),
                            sample_offset_v0: read_u32(chunk, 4),
                            ..CttsEntry::default()
                        },
                        1 => CttsEntry {
                            sample_count: read_u32(chunk, 0),
                            sample_offset_v1: read_i32(chunk, 4),
                            ..CttsEntry::default()
                        },
                        _ => CttsEntry::default(),
                    })?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Ctts {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("EntryCount", 2, with_bit_width(32)),
        codec_field!(
            "Entries",
            3,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes()
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// One edit-list entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ElstEntry {
    pub segment_duration_v0: u32,
    pub media_time_v0: i32,
    pub segment_duration_v1: u64,
    pub media_time_v1: i64,
    pub media_rate_integer: i16,
}

/// Edit list box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Elst {
    full_box: FullBoxState,
    pub entry_count: u32,
    pub entries: Vec<ElstEntry>,
}

impl FieldHooks for Elst {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "Entries" => match self.version() {
                0 => usize::try_from(self.entry_count)
                    .ok()
                    .and_then(|count| field_len_bytes(count, 12)),
                1 => usize::try_from(self.entry_count)
                    .ok()
                    .and_then(|count| field_len_bytes(count, 20)),
                _ => Some(0),
            },
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Entries" => Some(render_array(self.entries.iter().map(
                |entry| match self.version() {
                    0 => format!(
                        "{{SegmentDurationV0={} MediaTimeV0={} MediaRateInteger={}}}",
                        entry.segment_duration_v0, entry.media_time_v0, entry.media_rate_integer
                    ),
                    1 => format!(
                        "{{SegmentDurationV1={} MediaTimeV1={} MediaRateInteger={}}}",
                        entry.segment_duration_v1, entry.media_time_v1, entry.media_rate_integer
                    ),
                    _ => String::from("{}"),
                },
            ))),
            _ => None,
        }
    }
}

impl ImmutableBox for Elst {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"elst")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for Elst {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }
}

impl Elst {
    /// Returns the active segment duration for `index`.
    pub fn segment_duration(&self, index: usize) -> u64 {
        match self.version() {
            0 => u64::from(self.entries[index].segment_duration_v0),
            1 => self.entries[index].segment_duration_v1,
            _ => 0,
        }
    }

    /// Returns the active media time for `index`.
    pub fn media_time(&self, index: usize) -> i64 {
        match self.version() {
            0 => i64::from(self.entries[index].media_time_v0),
            1 => self.entries[index].media_time_v1,
            _ => 0,
        }
    }
}

impl FieldValueRead for Elst {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "EntryCount" => Ok(FieldValue::Unsigned(u64::from(self.entry_count))),
            "Entries" => {
                let mut bytes = Vec::new();
                for entry in &self.entries {
                    match self.version() {
                        0 => {
                            bytes.extend_from_slice(&entry.segment_duration_v0.to_be_bytes());
                            bytes.extend_from_slice(&entry.media_time_v0.to_be_bytes());
                        }
                        1 => {
                            bytes.extend_from_slice(&entry.segment_duration_v1.to_be_bytes());
                            bytes.extend_from_slice(&entry.media_time_v1.to_be_bytes());
                        }
                        _ => {}
                    }
                    bytes.extend_from_slice(&entry.media_rate_integer.to_be_bytes());
                    bytes.extend_from_slice(&0_i16.to_be_bytes());
                }
                Ok(FieldValue::Bytes(bytes))
            }
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Elst {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("EntryCount", FieldValue::Unsigned(value)) => {
                self.entry_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Entries", FieldValue::Bytes(bytes)) => {
                self.entries = match self.version() {
                    0 => parse_fixed_chunks(field_name, &bytes, 12, |chunk| ElstEntry {
                        segment_duration_v0: read_u32(chunk, 0),
                        media_time_v0: read_i32(chunk, 4),
                        media_rate_integer: read_i16(chunk, 8),
                        ..ElstEntry::default()
                    })?,
                    1 => parse_fixed_chunks(field_name, &bytes, 20, |chunk| ElstEntry {
                        segment_duration_v1: read_u64(chunk, 0),
                        media_time_v1: read_i64(chunk, 8),
                        media_rate_integer: read_i16(chunk, 16),
                        ..ElstEntry::default()
                    })?,
                    _ => Vec::new(),
                };
                for chunk in bytes.chunks_exact(match self.version() {
                    0 => 12,
                    1 => 20,
                    _ => 1,
                }) {
                    let offset = chunk.len() - 2;
                    if read_i16(chunk, offset) != 0 {
                        return Err(invalid_value(
                            field_name,
                            "media rate fraction must be zero",
                        ));
                    }
                }
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Elst {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("EntryCount", 2, with_bit_width(32)),
        codec_field!(
            "Entries",
            3,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes()
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// 64-bit chunk offset box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Co64 {
    full_box: FullBoxState,
    pub entry_count: u32,
    pub chunk_offset: Vec<u64>,
}

impl_full_box!(Co64, *b"co64");

impl FieldHooks for Co64 {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "ChunkOffset" => Some(self.entry_count),
            _ => None,
        }
    }
}

impl FieldValueRead for Co64 {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "EntryCount" => Ok(FieldValue::Unsigned(u64::from(self.entry_count))),
            "ChunkOffset" => Ok(FieldValue::UnsignedArray(self.chunk_offset.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Co64 {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("EntryCount", FieldValue::Unsigned(value)) => {
                self.entry_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ChunkOffset", FieldValue::UnsignedArray(values)) => {
                self.chunk_offset = values;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Co64 {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("EntryCount", 2, with_bit_width(32)),
        codec_field!("ChunkOffset", 3, with_bit_width(64), with_dynamic_length()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// 32-bit chunk offset box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stco {
    full_box: FullBoxState,
    pub entry_count: u32,
    pub chunk_offset: Vec<u64>,
}

impl_full_box!(Stco, *b"stco");

impl FieldHooks for Stco {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "ChunkOffset" => Some(self.entry_count),
            _ => None,
        }
    }
}

impl FieldValueRead for Stco {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "EntryCount" => Ok(FieldValue::Unsigned(u64::from(self.entry_count))),
            "ChunkOffset" => Ok(FieldValue::UnsignedArray(self.chunk_offset.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Stco {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("EntryCount", FieldValue::Unsigned(value)) => {
                self.entry_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ChunkOffset", FieldValue::UnsignedArray(values)) => {
                let mut offsets = Vec::with_capacity(values.len());
                for value in values {
                    offsets.push(u64::from(u32_from_unsigned(field_name, value)?));
                }
                self.chunk_offset = offsets;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Stco {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("EntryCount", 2, with_bit_width(32)),
        codec_field!("ChunkOffset", 3, with_bit_width(32), with_dynamic_length()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// One sample-to-chunk entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StscEntry {
    pub first_chunk: u32,
    pub samples_per_chunk: u32,
    pub sample_description_index: u32,
}

/// Sample-to-chunk box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stsc {
    full_box: FullBoxState,
    pub entry_count: u32,
    pub entries: Vec<StscEntry>,
}

impl FieldHooks for Stsc {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "Entries" => usize::try_from(self.entry_count)
                .ok()
                .and_then(|count| field_len_bytes(count, 12)),
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Entries" => Some(render_array(self.entries.iter().map(|entry| {
                format!(
                    "{{FirstChunk={} SamplesPerChunk={} SampleDescriptionIndex={}}}",
                    entry.first_chunk, entry.samples_per_chunk, entry.sample_description_index
                )
            }))),
            _ => None,
        }
    }
}

impl ImmutableBox for Stsc {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"stsc")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for Stsc {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }
}

impl FieldValueRead for Stsc {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "EntryCount" => Ok(FieldValue::Unsigned(u64::from(self.entry_count))),
            "Entries" => {
                let mut bytes = Vec::with_capacity(self.entries.len() * 12);
                for entry in &self.entries {
                    bytes.extend_from_slice(&entry.first_chunk.to_be_bytes());
                    bytes.extend_from_slice(&entry.samples_per_chunk.to_be_bytes());
                    bytes.extend_from_slice(&entry.sample_description_index.to_be_bytes());
                }
                Ok(FieldValue::Bytes(bytes))
            }
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Stsc {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("EntryCount", FieldValue::Unsigned(value)) => {
                self.entry_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Entries", FieldValue::Bytes(bytes)) => {
                self.entries = parse_fixed_chunks(field_name, &bytes, 12, |chunk| StscEntry {
                    first_chunk: read_u32(chunk, 0),
                    samples_per_chunk: read_u32(chunk, 4),
                    sample_description_index: read_u32(chunk, 8),
                })?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Stsc {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("EntryCount", 2, with_bit_width(32)),
        codec_field!(
            "Entries",
            3,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes()
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Sync sample box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stss {
    full_box: FullBoxState,
    pub entry_count: u32,
    pub sample_number: Vec<u64>,
}

impl_full_box!(Stss, *b"stss");

impl FieldHooks for Stss {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "SampleNumber" => Some(self.entry_count),
            _ => None,
        }
    }
}

impl FieldValueRead for Stss {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "EntryCount" => Ok(FieldValue::Unsigned(u64::from(self.entry_count))),
            "SampleNumber" => Ok(FieldValue::UnsignedArray(self.sample_number.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Stss {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("EntryCount", FieldValue::Unsigned(value)) => {
                self.entry_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("SampleNumber", FieldValue::UnsignedArray(values)) => {
                let mut numbers = Vec::with_capacity(values.len());
                for value in values {
                    numbers.push(u64::from(u32_from_unsigned(field_name, value)?));
                }
                self.sample_number = numbers;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Stss {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("EntryCount", 2, with_bit_width(32)),
        codec_field!("SampleNumber", 3, with_bit_width(32), with_dynamic_length()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Sample size box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stsz {
    full_box: FullBoxState,
    pub sample_size: u32,
    pub sample_count: u32,
    pub entry_size: Vec<u64>,
}

impl FieldHooks for Stsz {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "EntrySize" => {
                if self.sample_size == 0 {
                    Some(self.sample_count)
                } else {
                    Some(0)
                }
            }
            _ => None,
        }
    }

    fn display_field(&self, _name: &'static str) -> Option<String> {
        None
    }
}

impl ImmutableBox for Stsz {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"stsz")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for Stsz {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }
}

impl FieldValueRead for Stsz {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "SampleSize" => Ok(FieldValue::Unsigned(u64::from(self.sample_size))),
            "SampleCount" => Ok(FieldValue::Unsigned(u64::from(self.sample_count))),
            "EntrySize" => Ok(FieldValue::UnsignedArray(self.entry_size.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Stsz {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("SampleSize", FieldValue::Unsigned(value)) => {
                self.sample_size = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("SampleCount", FieldValue::Unsigned(value)) => {
                self.sample_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("EntrySize", FieldValue::UnsignedArray(values)) => {
                let mut entry_size = Vec::with_capacity(values.len());
                for value in values {
                    entry_size.push(u64::from(u32_from_unsigned(field_name, value)?));
                }
                self.entry_size = entry_size;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Stsz {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("SampleSize", 2, with_bit_width(32)),
        codec_field!("SampleCount", 3, with_bit_width(32)),
        codec_field!("EntrySize", 4, with_bit_width(32), with_dynamic_length()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// One time-to-sample entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SttsEntry {
    pub sample_count: u32,
    pub sample_delta: u32,
}

/// Time to sample box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stts {
    full_box: FullBoxState,
    pub entry_count: u32,
    pub entries: Vec<SttsEntry>,
}

impl FieldHooks for Stts {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "Entries" => usize::try_from(self.entry_count)
                .ok()
                .and_then(|count| field_len_bytes(count, 8)),
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Entries" => Some(render_array(self.entries.iter().map(|entry| {
                format!(
                    "{{SampleCount={} SampleDelta={}}}",
                    entry.sample_count, entry.sample_delta
                )
            }))),
            _ => None,
        }
    }
}

impl ImmutableBox for Stts {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"stts")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for Stts {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }
}

impl FieldValueRead for Stts {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "EntryCount" => Ok(FieldValue::Unsigned(u64::from(self.entry_count))),
            "Entries" => {
                let mut bytes = Vec::with_capacity(self.entries.len() * 8);
                for entry in &self.entries {
                    bytes.extend_from_slice(&entry.sample_count.to_be_bytes());
                    bytes.extend_from_slice(&entry.sample_delta.to_be_bytes());
                }
                Ok(FieldValue::Bytes(bytes))
            }
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Stts {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("EntryCount", FieldValue::Unsigned(value)) => {
                self.entry_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Entries", FieldValue::Bytes(bytes)) => {
                self.entries = parse_fixed_chunks(field_name, &bytes, 8, |chunk| SttsEntry {
                    sample_count: read_u32(chunk, 0),
                    sample_delta: read_u32(chunk, 4),
                })?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Stts {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("EntryCount", 2, with_bit_width(32)),
        codec_field!(
            "Entries",
            3,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes()
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// One track-run sample entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrunEntry {
    pub sample_duration: u32,
    pub sample_size: u32,
    pub sample_flags: u32,
    pub sample_composition_time_offset_v0: u32,
    pub sample_composition_time_offset_v1: i32,
}

/// Track run box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Trun {
    full_box: FullBoxState,
    pub sample_count: u32,
    pub data_offset: i32,
    pub first_sample_flags: u32,
    pub entries: Vec<TrunEntry>,
}

impl FieldHooks for Trun {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "Entries" => {
                let mut bytes_per_entry = 0usize;
                if self.flags() & TRUN_SAMPLE_DURATION_PRESENT != 0 {
                    bytes_per_entry += 4;
                }
                if self.flags() & TRUN_SAMPLE_SIZE_PRESENT != 0 {
                    bytes_per_entry += 4;
                }
                if self.flags() & TRUN_SAMPLE_FLAGS_PRESENT != 0 {
                    bytes_per_entry += 4;
                }
                if self.flags() & TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT != 0 {
                    bytes_per_entry += 4;
                }
                usize::try_from(self.sample_count)
                    .ok()
                    .and_then(|count| field_len_bytes(count, bytes_per_entry))
            }
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Entries" => Some(render_array(self.entries.iter().map(|entry| {
                let mut fields = Vec::new();
                if self.flags() & TRUN_SAMPLE_DURATION_PRESENT != 0 {
                    fields.push(format!("SampleDuration={}", entry.sample_duration));
                }
                if self.flags() & TRUN_SAMPLE_SIZE_PRESENT != 0 {
                    fields.push(format!("SampleSize={}", entry.sample_size));
                }
                if self.flags() & TRUN_SAMPLE_FLAGS_PRESENT != 0 {
                    fields.push(format!("SampleFlags=0x{:x}", entry.sample_flags));
                }
                if self.flags() & TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT != 0 {
                    if self.version() == 0 {
                        fields.push(format!(
                            "SampleCompositionTimeOffsetV0={}",
                            entry.sample_composition_time_offset_v0
                        ));
                    } else {
                        fields.push(format!(
                            "SampleCompositionTimeOffsetV1={}",
                            entry.sample_composition_time_offset_v1
                        ));
                    }
                }
                format!("{{{}}}", fields.join(" "))
            }))),
            _ => None,
        }
    }
}

impl ImmutableBox for Trun {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"trun")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for Trun {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }
}

impl Trun {
    /// Returns the active composition time offset for `index`.
    pub fn sample_composition_time_offset(&self, index: usize) -> i64 {
        match self.version() {
            0 => i64::from(self.entries[index].sample_composition_time_offset_v0),
            1 => i64::from(self.entries[index].sample_composition_time_offset_v1),
            _ => 0,
        }
    }
}

impl FieldValueRead for Trun {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "SampleCount" => Ok(FieldValue::Unsigned(u64::from(self.sample_count))),
            "DataOffset" => Ok(FieldValue::Signed(i64::from(self.data_offset))),
            "FirstSampleFlags" => Ok(FieldValue::Unsigned(u64::from(self.first_sample_flags))),
            "Entries" => {
                let mut bytes = Vec::new();
                for entry in &self.entries {
                    if self.flags() & TRUN_SAMPLE_DURATION_PRESENT != 0 {
                        bytes.extend_from_slice(&entry.sample_duration.to_be_bytes());
                    }
                    if self.flags() & TRUN_SAMPLE_SIZE_PRESENT != 0 {
                        bytes.extend_from_slice(&entry.sample_size.to_be_bytes());
                    }
                    if self.flags() & TRUN_SAMPLE_FLAGS_PRESENT != 0 {
                        bytes.extend_from_slice(&entry.sample_flags.to_be_bytes());
                    }
                    if self.flags() & TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT != 0 {
                        if self.version() == 0 {
                            bytes.extend_from_slice(
                                &entry.sample_composition_time_offset_v0.to_be_bytes(),
                            );
                        } else {
                            bytes.extend_from_slice(
                                &entry.sample_composition_time_offset_v1.to_be_bytes(),
                            );
                        }
                    }
                }
                Ok(FieldValue::Bytes(bytes))
            }
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Trun {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("SampleCount", FieldValue::Unsigned(value)) => {
                self.sample_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DataOffset", FieldValue::Signed(value)) => {
                self.data_offset = i32_from_signed(field_name, value)?;
                Ok(())
            }
            ("FirstSampleFlags", FieldValue::Unsigned(value)) => {
                self.first_sample_flags = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Entries", FieldValue::Bytes(bytes)) => {
                let mut bytes_per_entry = 0usize;
                if self.flags() & TRUN_SAMPLE_DURATION_PRESENT != 0 {
                    bytes_per_entry += 4;
                }
                if self.flags() & TRUN_SAMPLE_SIZE_PRESENT != 0 {
                    bytes_per_entry += 4;
                }
                if self.flags() & TRUN_SAMPLE_FLAGS_PRESENT != 0 {
                    bytes_per_entry += 4;
                }
                if self.flags() & TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT != 0 {
                    bytes_per_entry += 4;
                }

                self.entries = if bytes_per_entry == 0 {
                    Vec::new()
                } else {
                    parse_fixed_chunks(field_name, &bytes, bytes_per_entry, |chunk| {
                        let mut offset = 0;
                        let mut entry = TrunEntry::default();
                        if self.flags() & TRUN_SAMPLE_DURATION_PRESENT != 0 {
                            entry.sample_duration = read_u32(chunk, offset);
                            offset += 4;
                        }
                        if self.flags() & TRUN_SAMPLE_SIZE_PRESENT != 0 {
                            entry.sample_size = read_u32(chunk, offset);
                            offset += 4;
                        }
                        if self.flags() & TRUN_SAMPLE_FLAGS_PRESENT != 0 {
                            entry.sample_flags = read_u32(chunk, offset);
                            offset += 4;
                        }
                        if self.flags() & TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT != 0 {
                            if self.version() == 0 {
                                entry.sample_composition_time_offset_v0 = read_u32(chunk, offset);
                            } else {
                                entry.sample_composition_time_offset_v1 = read_i32(chunk, offset);
                            }
                        }
                        entry
                    })?
                };
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Trun {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("SampleCount", 2, with_bit_width(32)),
        codec_field!(
            "DataOffset",
            3,
            with_bit_width(32),
            as_signed(),
            with_required_flags(TRUN_DATA_OFFSET_PRESENT)
        ),
        codec_field!(
            "FirstSampleFlags",
            4,
            with_bit_width(32),
            with_required_flags(TRUN_FIRST_SAMPLE_FLAGS_PRESENT),
            as_hex()
        ),
        codec_field!(
            "Entries",
            5,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes()
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

simple_container_box!(Schi, *b"schi");
simple_container_box!(Sinf, *b"sinf");
simple_container_box!(Wave, *b"wave");

/// Metadata box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    full_box: FullBoxState,
    quicktime_headerless: bool,
}

impl FieldHooks for Meta {
    fn field_enabled(&self, name: &'static str) -> Option<bool> {
        match name {
            "Version" | "Flags" => Some(!self.quicktime_headerless),
            _ => None,
        }
    }
}

impl ImmutableBox for Meta {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"meta")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for Meta {
    fn set_version(&mut self, version: u8) {
        self.quicktime_headerless = false;
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.quicktime_headerless = false;
        self.full_box.flags = flags;
    }

    fn before_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<(), CodecError> {
        self.quicktime_headerless = false;
        if payload_size < 4 {
            return Ok(());
        }

        // Headerless metadata starts directly with the first child box type instead of the full-box prefix.
        let start = reader.stream_position()?;
        let mut prefix = [0_u8; 4];
        reader.read_exact(&mut prefix)?;
        reader.seek(SeekFrom::Start(start))?;

        if prefix.iter().any(|byte| *byte != 0) {
            self.quicktime_headerless = true;
            self.full_box.version = 0;
            self.full_box.flags = 0;
        }

        Ok(())
    }
}

impl Meta {
    /// Returns `true` when the payload omits the normal full-box header bytes.
    pub fn is_quicktime_headerless(&self) -> bool {
        self.quicktime_headerless
    }
}

impl FieldValueRead for Meta {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        Err(missing_field(field_name))
    }
}

impl FieldValueWrite for Meta {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        Err(unexpected_field(field_name, value))
    }
}

impl CodecBox for Meta {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!(
            "Version",
            0,
            with_bit_width(8),
            as_version_field(),
            with_dynamic_presence()
        ),
        codec_field!(
            "Flags",
            1,
            with_bit_width(24),
            as_flags_field(),
            with_dynamic_presence()
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Track-kind metadata box that stores a scheme URI and value string.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Kind {
    full_box: FullBoxState,
    pub scheme_uri: String,
    pub value: String,
}

impl FieldHooks for Kind {}

impl_full_box!(Kind, *b"kind");

impl FieldValueRead for Kind {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "SchemeURI" => Ok(FieldValue::String(self.scheme_uri.clone())),
            "Value" => Ok(FieldValue::String(self.value.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Kind {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("SchemeURI", FieldValue::String(value)) => {
                validate_c_string_value(field_name, &value)?;
                self.scheme_uri = value;
                Ok(())
            }
            ("Value", FieldValue::String(value)) => {
                validate_c_string_value(field_name, &value)?;
                self.value = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Kind {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!(
            "SchemeURI",
            2,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated)
        ),
        codec_field!(
            "Value",
            3,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated)
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];

    fn custom_marshal(&self, writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        validate_c_string_value("SchemeURI", &self.scheme_uri)?;
        validate_c_string_value("Value", &self.value)?;
        if self.version() != 0 {
            return Err(CodecError::UnsupportedVersion {
                box_type: self.box_type(),
                version: self.version(),
            });
        }

        let mut payload = Vec::with_capacity(6 + self.scheme_uri.len() + self.value.len());
        payload.push(self.version());
        push_uint("Flags", &mut payload, 3, u64::from(self.flags()))?;
        payload.extend_from_slice(self.scheme_uri.as_bytes());
        payload.push(0);
        payload.extend_from_slice(self.value.as_bytes());
        payload.push(0);
        writer.write_all(&payload)?;
        Ok(Some(payload.len() as u64))
    }

    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, CodecError> {
        let payload_len = usize::try_from(payload_size)
            .map_err(|_| invalid_value("Payload", "payload is too large to decode"))?;
        let payload = read_exact_vec_untrusted(reader, payload_len).map_err(CodecError::Io)?;

        if payload.len() < 6 {
            return Err(invalid_value("Payload", "payload is too short").into());
        }

        let version = payload[0];
        if version != 0 {
            return Err(CodecError::UnsupportedVersion {
                box_type: self.box_type(),
                version,
            });
        }

        let (scheme_uri, scheme_len) = decode_required_c_string("SchemeURI", &payload[4..])?;
        let value_offset = 4 + scheme_len;
        let (value, value_len) = decode_required_c_string("Value", &payload[value_offset..])?;
        if value_offset + value_len != payload.len() {
            return Err(invalid_value("Payload", "payload has trailing bytes").into());
        }

        self.full_box = FullBoxState {
            version,
            flags: read_uint(&payload, 1, 3) as u32,
        };
        self.scheme_uri = scheme_uri;
        self.value = value;
        Ok(Some(payload_size))
    }
}

/// MIME metadata box that preserves whether the payload omitted the trailing NUL byte.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mime {
    full_box: FullBoxState,
    pub content_type: String,
    pub lacks_zero_termination: bool,
}

impl FieldHooks for Mime {
    fn field_enabled(&self, name: &'static str) -> Option<bool> {
        match name {
            "LacksZeroTermination" => Some(self.lacks_zero_termination),
            _ => None,
        }
    }
}

impl_full_box!(Mime, *b"mime");

impl FieldValueRead for Mime {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "ContentType" => Ok(FieldValue::String(self.content_type.clone())),
            "LacksZeroTermination" => Ok(FieldValue::Boolean(self.lacks_zero_termination)),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Mime {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("ContentType", FieldValue::String(value)) => {
                validate_c_string_value(field_name, &value)?;
                self.content_type = value;
                Ok(())
            }
            ("LacksZeroTermination", FieldValue::Boolean(value)) => {
                self.lacks_zero_termination = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Mime {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!(
            "ContentType",
            2,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated)
        ),
        codec_field!(
            "LacksZeroTermination",
            3,
            with_bit_width(1),
            as_boolean(),
            with_dynamic_presence()
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];

    fn custom_marshal(&self, writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        validate_c_string_value("ContentType", &self.content_type)?;
        if self.version() != 0 {
            return Err(CodecError::UnsupportedVersion {
                box_type: self.box_type(),
                version: self.version(),
            });
        }
        if self.lacks_zero_termination && self.content_type.is_empty() {
            return Err(
                invalid_value("ContentType", "non-terminated payload must not be empty").into(),
            );
        }

        let mut payload = Vec::with_capacity(
            4 + self.content_type.len() + usize::from(!self.lacks_zero_termination),
        );
        payload.push(self.version());
        push_uint("Flags", &mut payload, 3, u64::from(self.flags()))?;
        payload.extend_from_slice(self.content_type.as_bytes());
        if !self.lacks_zero_termination {
            payload.push(0);
        }
        writer.write_all(&payload)?;
        Ok(Some(payload.len() as u64))
    }

    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, CodecError> {
        let payload_len = usize::try_from(payload_size)
            .map_err(|_| invalid_value("Payload", "payload is too large to decode"))?;
        let payload = read_exact_vec_untrusted(reader, payload_len).map_err(CodecError::Io)?;

        if payload.len() < 5 {
            return Err(invalid_value("Payload", "payload is too short").into());
        }

        let version = payload[0];
        if version != 0 {
            return Err(CodecError::UnsupportedVersion {
                box_type: self.box_type(),
                version,
            });
        }

        let content_bytes = if payload.last() == Some(&0) {
            self.lacks_zero_termination = false;
            &payload[4..payload.len() - 1]
        } else {
            self.lacks_zero_termination = true;
            &payload[4..]
        };

        if content_bytes.contains(&0) {
            return Err(invalid_value("ContentType", "value must not contain NUL bytes").into());
        }

        self.full_box = FullBoxState {
            version,
            flags: read_uint(&payload, 1, 3) as u32,
        };
        self.content_type =
            String::from_utf8(content_bytes.to_vec()).map_err(|_| CodecError::InvalidUtf8 {
                field_name: "ContentType",
            })?;
        Ok(Some(payload_size))
    }
}

/// Handler reference box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hdlr {
    full_box: FullBoxState,
    pub pre_defined: u32,
    pub handler_type: FourCc,
    pub reserved: [u8; 12],
    pub name: String,
}

impl Default for Hdlr {
    fn default() -> Self {
        Self {
            full_box: FullBoxState::default(),
            pre_defined: 0,
            handler_type: FourCc::ANY,
            reserved: [0; 12],
            name: String::new(),
        }
    }
}

impl FieldHooks for Hdlr {
    fn is_pascal_string(
        &self,
        name: &'static str,
        _data: &[u8],
        remaining_bytes: u64,
    ) -> Option<bool> {
        match name {
            // Some files store the handler name as a Pascal string and consume the last payload byte with the length prefix.
            "Name" => Some(self.pre_defined != 0 && remaining_bytes == 0),
            _ => None,
        }
    }

    fn consume_remaining_bytes_after_string(&self, name: &'static str) -> Option<bool> {
        match name {
            // Handler names may be padded after the visible terminator, so keep consuming the declared field payload.
            "Name" => Some(true),
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "HandlerType" => Some(quoted_fourcc(self.handler_type)),
            _ => None,
        }
    }
}

impl_full_box!(Hdlr, *b"hdlr");

impl FieldValueRead for Hdlr {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "PreDefined" => Ok(FieldValue::Unsigned(u64::from(self.pre_defined))),
            "HandlerType" => Ok(FieldValue::Bytes(self.handler_type.as_bytes().to_vec())),
            "Reserved" => Ok(FieldValue::Bytes(self.reserved.to_vec())),
            "Name" => Ok(FieldValue::String(self.name.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Hdlr {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("PreDefined", FieldValue::Unsigned(value)) => {
                self.pre_defined = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("HandlerType", FieldValue::Bytes(bytes)) => {
                self.handler_type = bytes_to_fourcc(field_name, bytes)?;
                Ok(())
            }
            ("Reserved", FieldValue::Bytes(bytes)) => {
                if bytes.len() != 12 {
                    return Err(invalid_value(
                        field_name,
                        "value must contain exactly 12 bytes",
                    ));
                }
                self.reserved.copy_from_slice(&bytes);
                Ok(())
            }
            ("Name", FieldValue::String(value)) => {
                self.name = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Hdlr {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("PreDefined", 2, with_bit_width(32)),
        codec_field!(
            "HandlerType",
            3,
            with_bit_width(8),
            with_length(4),
            as_bytes()
        ),
        codec_field!(
            "Reserved",
            4,
            with_bit_width(8),
            with_length(12),
            as_bytes(),
            as_hidden()
        ),
        codec_field!(
            "Name",
            5,
            with_bit_width(8),
            as_string(StringFieldMode::PascalCompatible)
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Auxiliary information offsets box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saio {
    full_box: FullBoxState,
    pub aux_info_type: FourCc,
    pub aux_info_type_parameter: u32,
    pub entry_count: u32,
    pub offset_v0: Vec<u64>,
    pub offset_v1: Vec<u64>,
}

impl Default for Saio {
    fn default() -> Self {
        Self {
            full_box: FullBoxState::default(),
            aux_info_type: FourCc::ANY,
            aux_info_type_parameter: 0,
            entry_count: 0,
            offset_v0: Vec::new(),
            offset_v1: Vec::new(),
        }
    }
}

impl FieldHooks for Saio {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "OffsetV0" | "OffsetV1" => Some(self.entry_count),
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "AuxInfoType" => Some(quoted_fourcc(self.aux_info_type)),
            _ => None,
        }
    }
}

impl_full_box!(Saio, *b"saio");

impl Saio {
    /// Returns the active auxiliary information offset at `index`.
    pub fn offset(&self, index: usize) -> u64 {
        match self.version() {
            0 => self.offset_v0[index],
            1 => self.offset_v1[index],
            _ => 0,
        }
    }
}

impl FieldValueRead for Saio {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "AuxInfoType" => Ok(FieldValue::Bytes(self.aux_info_type.as_bytes().to_vec())),
            "AuxInfoTypeParameter" => Ok(FieldValue::Unsigned(u64::from(
                self.aux_info_type_parameter,
            ))),
            "EntryCount" => Ok(FieldValue::Unsigned(u64::from(self.entry_count))),
            "OffsetV0" => Ok(FieldValue::UnsignedArray(self.offset_v0.clone())),
            "OffsetV1" => Ok(FieldValue::UnsignedArray(self.offset_v1.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Saio {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("AuxInfoType", FieldValue::Bytes(bytes)) => {
                self.aux_info_type = bytes_to_fourcc(field_name, bytes)?;
                Ok(())
            }
            ("AuxInfoTypeParameter", FieldValue::Unsigned(value)) => {
                self.aux_info_type_parameter = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("EntryCount", FieldValue::Unsigned(value)) => {
                self.entry_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("OffsetV0", FieldValue::UnsignedArray(values)) => {
                let mut offsets = Vec::with_capacity(values.len());
                for value in values {
                    offsets.push(u64::from(u32_from_unsigned(field_name, value)?));
                }
                self.offset_v0 = offsets;
                Ok(())
            }
            ("OffsetV1", FieldValue::UnsignedArray(values)) => {
                self.offset_v1 = values;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Saio {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!(
            "AuxInfoType",
            2,
            with_bit_width(8),
            with_length(4),
            as_bytes(),
            with_required_flags(AUX_INFO_TYPE_PRESENT)
        ),
        codec_field!(
            "AuxInfoTypeParameter",
            3,
            with_bit_width(32),
            as_hex(),
            with_required_flags(AUX_INFO_TYPE_PRESENT)
        ),
        codec_field!("EntryCount", 4, with_bit_width(32)),
        codec_field!(
            "OffsetV0",
            5,
            with_bit_width(32),
            with_dynamic_length(),
            with_version(0)
        ),
        codec_field!(
            "OffsetV1",
            6,
            with_bit_width(64),
            with_dynamic_length(),
            with_version(1)
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// Auxiliary information sizes box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saiz {
    full_box: FullBoxState,
    pub aux_info_type: FourCc,
    pub aux_info_type_parameter: u32,
    pub default_sample_info_size: u8,
    pub sample_count: u32,
    pub sample_info_size: Vec<u8>,
}

impl Default for Saiz {
    fn default() -> Self {
        Self {
            full_box: FullBoxState::default(),
            aux_info_type: FourCc::ANY,
            aux_info_type_parameter: 0,
            default_sample_info_size: 0,
            sample_count: 0,
            sample_info_size: Vec::new(),
        }
    }
}

impl FieldHooks for Saiz {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "SampleInfoSize" => Some(self.sample_count),
            _ => None,
        }
    }

    fn field_enabled(&self, name: &'static str) -> Option<bool> {
        match name {
            "SampleInfoSize" => Some(self.default_sample_info_size == 0),
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "AuxInfoType" => Some(quoted_fourcc(self.aux_info_type)),
            _ => None,
        }
    }
}

impl_full_box!(Saiz, *b"saiz");

impl FieldValueRead for Saiz {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "AuxInfoType" => Ok(FieldValue::Bytes(self.aux_info_type.as_bytes().to_vec())),
            "AuxInfoTypeParameter" => Ok(FieldValue::Unsigned(u64::from(
                self.aux_info_type_parameter,
            ))),
            "DefaultSampleInfoSize" => Ok(FieldValue::Unsigned(u64::from(
                self.default_sample_info_size,
            ))),
            "SampleCount" => Ok(FieldValue::Unsigned(u64::from(self.sample_count))),
            "SampleInfoSize" => Ok(FieldValue::UnsignedArray(
                self.sample_info_size
                    .iter()
                    .copied()
                    .map(u64::from)
                    .collect(),
            )),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Saiz {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("AuxInfoType", FieldValue::Bytes(bytes)) => {
                self.aux_info_type = bytes_to_fourcc(field_name, bytes)?;
                Ok(())
            }
            ("AuxInfoTypeParameter", FieldValue::Unsigned(value)) => {
                self.aux_info_type_parameter = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DefaultSampleInfoSize", FieldValue::Unsigned(value)) => {
                self.default_sample_info_size = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("SampleCount", FieldValue::Unsigned(value)) => {
                self.sample_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("SampleInfoSize", FieldValue::UnsignedArray(values)) => {
                let mut sizes = Vec::with_capacity(values.len());
                for value in values {
                    sizes.push(u8_from_unsigned(field_name, value)?);
                }
                self.sample_info_size = sizes;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Saiz {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!(
            "AuxInfoType",
            2,
            with_bit_width(8),
            with_length(4),
            as_bytes(),
            with_required_flags(AUX_INFO_TYPE_PRESENT)
        ),
        codec_field!(
            "AuxInfoTypeParameter",
            3,
            with_bit_width(32),
            as_hex(),
            with_required_flags(AUX_INFO_TYPE_PRESENT)
        ),
        codec_field!("DefaultSampleInfoSize", 4, with_bit_width(8)),
        codec_field!("SampleCount", 5, with_bit_width(32)),
        codec_field!(
            "SampleInfoSize",
            6,
            with_bit_width(8),
            with_dynamic_length(),
            with_dynamic_presence()
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// One sample-to-group entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SbgpEntry {
    pub sample_count: u32,
    pub group_description_index: u32,
}

/// Sample-to-group box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sbgp {
    full_box: FullBoxState,
    pub grouping_type: u32,
    pub grouping_type_parameter: u32,
    pub entry_count: u32,
    pub entries: Vec<SbgpEntry>,
}

impl FieldHooks for Sbgp {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Entries" => Some(render_array(self.entries.iter().map(|entry| {
                format!(
                    "{{SampleCount={} GroupDescriptionIndex={}}}",
                    entry.sample_count, entry.group_description_index
                )
            }))),
            _ => None,
        }
    }
}

impl_full_box!(Sbgp, *b"sbgp");

impl FieldValueRead for Sbgp {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "GroupingType" => Ok(FieldValue::Unsigned(u64::from(self.grouping_type))),
            "GroupingTypeParameter" => Ok(FieldValue::Unsigned(u64::from(
                self.grouping_type_parameter,
            ))),
            "EntryCount" => Ok(FieldValue::Unsigned(u64::from(self.entry_count))),
            "Entries" => {
                let mut bytes = Vec::with_capacity(self.entries.len() * 8);
                for entry in &self.entries {
                    bytes.extend_from_slice(&entry.sample_count.to_be_bytes());
                    bytes.extend_from_slice(&entry.group_description_index.to_be_bytes());
                }
                Ok(FieldValue::Bytes(bytes))
            }
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Sbgp {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("GroupingType", FieldValue::Unsigned(value)) => {
                self.grouping_type = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("GroupingTypeParameter", FieldValue::Unsigned(value)) => {
                self.grouping_type_parameter = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("EntryCount", FieldValue::Unsigned(value)) => {
                self.entry_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Entries", FieldValue::Bytes(bytes)) => {
                let expected_len = field_len_from_count(self.entry_count, 8)
                    .map(|len| len as usize)
                    .unwrap_or(0);
                if bytes.len() != expected_len {
                    return Err(invalid_value(
                        field_name,
                        "entry payload length does not match the entry count",
                    ));
                }

                self.entries = parse_fixed_chunks(field_name, &bytes, 8, |chunk| SbgpEntry {
                    sample_count: read_u32(chunk, 0),
                    group_description_index: read_u32(chunk, 4),
                })?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Sbgp {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("GroupingType", 2, with_bit_width(32)),
        codec_field!(
            "GroupingTypeParameter",
            3,
            with_bit_width(32),
            with_version(1)
        ),
        codec_field!("EntryCount", 4, with_bit_width(32)),
        codec_field!("Entries", 5, with_bit_width(8), as_bytes()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// One subsample record carried by [`Subs`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SubsSample {
    pub subsample_size: u32,
    pub subsample_priority: u8,
    pub discardable: u8,
    pub codec_specific_parameters: u32,
}

/// One sample entry inside [`Subs`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SubsEntry {
    pub sample_delta: u32,
    pub subsample_count: u16,
    pub subsamples: Vec<SubsSample>,
}

/// Subsample-information box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Subs {
    full_box: FullBoxState,
    pub entry_count: u32,
    pub entries: Vec<SubsEntry>,
}

fn format_subs_entries(entries: &[SubsEntry]) -> String {
    render_array(entries.iter().map(|entry| {
        format!(
            "{{SampleDelta={} SubsampleCount={} Subsamples={}}}",
            entry.sample_delta,
            entry.subsample_count,
            render_array(entry.subsamples.iter().map(|subsample| {
                format!(
                    "{{SubsampleSize={} SubsamplePriority={} Discardable={} CodecSpecificParameters={}}}",
                    subsample.subsample_size,
                    subsample.subsample_priority,
                    subsample.discardable,
                    subsample.codec_specific_parameters
                )
            }))
        )
    }))
}

fn encode_subs_entries(
    field_name: &'static str,
    version: u8,
    entries: &[SubsEntry],
) -> Result<Vec<u8>, FieldValueError> {
    let mut bytes = Vec::new();

    for entry in entries {
        require_count(
            field_name,
            u32::from(entry.subsample_count),
            entry.subsamples.len(),
        )?;
        bytes.extend_from_slice(&entry.sample_delta.to_be_bytes());
        bytes.extend_from_slice(&entry.subsample_count.to_be_bytes());

        for subsample in &entry.subsamples {
            if version == 0 {
                let subsample_size = u16::try_from(subsample.subsample_size).map_err(|_| {
                    invalid_value(field_name, "version 0 subsample size does not fit in u16")
                })?;
                bytes.extend_from_slice(&subsample_size.to_be_bytes());
            } else {
                bytes.extend_from_slice(&subsample.subsample_size.to_be_bytes());
            }

            bytes.push(subsample.subsample_priority);
            bytes.push(subsample.discardable);
            bytes.extend_from_slice(&subsample.codec_specific_parameters.to_be_bytes());
        }
    }

    Ok(bytes)
}

fn parse_subs_entries(
    field_name: &'static str,
    version: u8,
    entry_count: u32,
    bytes: &[u8],
) -> Result<Vec<SubsEntry>, FieldValueError> {
    let mut entries = Vec::with_capacity(untrusted_prealloc_hint(
        usize::try_from(entry_count).unwrap_or(0),
    ));
    let mut offset = 0usize;

    for _ in 0..entry_count {
        if bytes.len().saturating_sub(offset) < 6 {
            return Err(invalid_value(field_name, "entry payload is truncated"));
        }

        let sample_delta = read_u32(bytes, offset);
        let subsample_count = read_u16(bytes, offset + 4);
        offset += 6;

        let mut subsamples =
            Vec::with_capacity(untrusted_prealloc_hint(usize::from(subsample_count)));
        for _ in 0..subsample_count {
            let subsample_header_len = if version == 1 { 10 } else { 8 };
            if bytes.len().saturating_sub(offset) < subsample_header_len {
                return Err(invalid_value(field_name, "subsample payload is truncated"));
            }

            let subsample_size = if version == 1 {
                let value = read_u32(bytes, offset);
                offset += 4;
                value
            } else {
                let value = u32::from(read_u16(bytes, offset));
                offset += 2;
                value
            };

            let subsample_priority = bytes[offset];
            let discardable = bytes[offset + 1];
            let codec_specific_parameters = read_u32(bytes, offset + 2);
            offset += 6;

            subsamples.push(SubsSample {
                subsample_size,
                subsample_priority,
                discardable,
                codec_specific_parameters,
            });
        }

        entries.push(SubsEntry {
            sample_delta,
            subsample_count,
            subsamples,
        });
    }

    if offset != bytes.len() {
        return Err(invalid_value(
            field_name,
            "entry payload length does not match the entry count",
        ));
    }

    Ok(entries)
}

impl FieldHooks for Subs {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Entries" => Some(format_subs_entries(&self.entries)),
            _ => None,
        }
    }
}

impl_full_box!(Subs, *b"subs");

impl FieldValueRead for Subs {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "EntryCount" => Ok(FieldValue::Unsigned(u64::from(self.entry_count))),
            "Entries" => {
                require_count(field_name, self.entry_count, self.entries.len())?;
                Ok(FieldValue::Bytes(encode_subs_entries(
                    field_name,
                    self.version(),
                    &self.entries,
                )?))
            }
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Subs {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("EntryCount", FieldValue::Unsigned(value)) => {
                self.entry_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Entries", FieldValue::Bytes(bytes)) => {
                self.entries =
                    parse_subs_entries(field_name, self.version(), self.entry_count, &bytes)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Subs {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("EntryCount", 2, with_bit_width(32)),
        codec_field!("Entries", 3, with_bit_width(8), as_bytes()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// One packed sample dependency entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SdtpSampleElem {
    pub is_leading: u8,
    pub sample_depends_on: u8,
    pub sample_is_depended_on: u8,
    pub sample_has_redundancy: u8,
}

fn encode_sdtp_sample(
    field_name: &'static str,
    sample: &SdtpSampleElem,
) -> Result<u8, FieldValueError> {
    if sample.is_leading > 0x03
        || sample.sample_depends_on > 0x03
        || sample.sample_is_depended_on > 0x03
        || sample.sample_has_redundancy > 0x03
    {
        return Err(invalid_value(
            field_name,
            "sample dependency fields must fit in 2 bits",
        ));
    }

    Ok((sample.is_leading << 6)
        | (sample.sample_depends_on << 4)
        | (sample.sample_is_depended_on << 2)
        | sample.sample_has_redundancy)
}

/// Sample dependency type box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sdtp {
    full_box: FullBoxState,
    pub samples: Vec<SdtpSampleElem>,
}

impl FieldHooks for Sdtp {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Samples" => Some(render_array(self.samples.iter().map(|sample| {
                format!(
                    "{{IsLeading=0x{:x} SampleDependsOn=0x{:x} SampleIsDependedOn=0x{:x} SampleHasRedundancy=0x{:x}}}",
                    sample.is_leading,
                    sample.sample_depends_on,
                    sample.sample_is_depended_on,
                    sample.sample_has_redundancy
                )
            }))),
            _ => None,
        }
    }
}

impl_full_box!(Sdtp, *b"sdtp");

impl FieldValueRead for Sdtp {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Samples" => {
                let mut bytes = Vec::with_capacity(self.samples.len());
                for sample in &self.samples {
                    bytes.push(encode_sdtp_sample(field_name, sample)?);
                }
                Ok(FieldValue::Bytes(bytes))
            }
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Sdtp {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Samples", FieldValue::Bytes(bytes)) => {
                self.samples = bytes
                    .into_iter()
                    .map(|sample| SdtpSampleElem {
                        is_leading: (sample >> 6) & 0x03,
                        sample_depends_on: (sample >> 4) & 0x03,
                        sample_is_depended_on: (sample >> 2) & 0x03,
                        sample_has_redundancy: sample & 0x03,
                    })
                    .collect();
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Sdtp {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("Samples", 2, with_bit_width(8), as_bytes()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Length-prefixed roll-distance description.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RollDistanceWithLength {
    pub description_length: u32,
    pub roll_distance: i16,
}

/// Optional alternative-startup sample counts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AlternativeStartupEntryOpt {
    pub num_output_samples: u16,
    pub num_total_samples: u16,
}

/// Alternative-startup group description payload.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AlternativeStartupEntry {
    pub roll_count: u16,
    pub first_output_sample: u16,
    pub sample_offset: Vec<u32>,
    pub opts: Vec<AlternativeStartupEntryOpt>,
}

/// Length-prefixed alternative-startup description.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AlternativeStartupEntryL {
    pub description_length: u32,
    pub alternative_startup_entry: AlternativeStartupEntry,
}

/// Visual random-access group description payload.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VisualRandomAccessEntry {
    pub num_leading_samples_known: bool,
    pub num_leading_samples: u8,
}

/// Length-prefixed visual random-access description.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VisualRandomAccessEntryL {
    pub description_length: u32,
    pub visual_random_access_entry: VisualRandomAccessEntry,
}

/// Temporal-level group description payload.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TemporalLevelEntry {
    pub level_independently_decodable: bool,
}

/// Length-prefixed temporal-level description.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TemporalLevelEntryL {
    pub description_length: u32,
    pub temporal_level_entry: TemporalLevelEntry,
}

/// Sample-encryption information group description payload for the `seig` grouping type.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SeigEntry {
    pub reserved: u8,
    pub crypt_byte_block: u8,
    pub skip_byte_block: u8,
    pub is_protected: u8,
    pub per_sample_iv_size: u8,
    pub kid: [u8; 16],
    pub constant_iv_size: u8,
    pub constant_iv: Vec<u8>,
}

/// Length-prefixed sample-encryption information group description payload.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SeigEntryL {
    pub description_length: u32,
    pub seig_entry: SeigEntry,
}

fn format_alternative_startup_opts(opts: &[AlternativeStartupEntryOpt]) -> String {
    render_array(opts.iter().map(|opt| {
        format!(
            "{{NumOutputSamples={} NumTotalSamples={}}}",
            opt.num_output_samples, opt.num_total_samples
        )
    }))
}

fn format_alternative_startup_entry(entry: &AlternativeStartupEntry) -> String {
    format!(
        "{{RollCount={} FirstOutputSample={} SampleOffset={} Opts={}}}",
        entry.roll_count,
        entry.first_output_sample,
        render_array(entry.sample_offset.iter().map(|offset| offset.to_string())),
        format_alternative_startup_opts(&entry.opts)
    )
}

fn encode_alternative_startup_entry(
    field_name: &'static str,
    entry: &AlternativeStartupEntry,
) -> Result<Vec<u8>, FieldValueError> {
    require_count(
        field_name,
        u32::from(entry.roll_count),
        entry.sample_offset.len(),
    )?;

    let mut bytes = Vec::with_capacity(4 + (entry.sample_offset.len() + entry.opts.len()) * 4);
    bytes.extend_from_slice(&entry.roll_count.to_be_bytes());
    bytes.extend_from_slice(&entry.first_output_sample.to_be_bytes());
    for sample_offset in &entry.sample_offset {
        bytes.extend_from_slice(&sample_offset.to_be_bytes());
    }
    for opt in &entry.opts {
        bytes.extend_from_slice(&opt.num_output_samples.to_be_bytes());
        bytes.extend_from_slice(&opt.num_total_samples.to_be_bytes());
    }
    Ok(bytes)
}

fn parse_alternative_startup_entry(
    field_name: &'static str,
    bytes: &[u8],
) -> Result<AlternativeStartupEntry, FieldValueError> {
    if bytes.len() < 4 {
        return Err(invalid_value(
            field_name,
            "alternative startup entry is too short",
        ));
    }

    let roll_count = read_u16(bytes, 0);
    let sample_offset_count = usize::from(roll_count);
    let sample_offset_bytes = sample_offset_count
        .checked_mul(4)
        .ok_or_else(|| invalid_value(field_name, "alternative startup entry is too large"))?;
    let minimum_len = 4_usize
        .checked_add(sample_offset_bytes)
        .ok_or_else(|| invalid_value(field_name, "alternative startup entry is too large"))?;
    if bytes.len() < minimum_len {
        return Err(invalid_value(
            field_name,
            "alternative startup entry is shorter than its roll count requires",
        ));
    }

    let trailing_len = bytes.len() - minimum_len;
    if !trailing_len.is_multiple_of(4) {
        return Err(invalid_value(
            field_name,
            "alternative startup entry options do not align to 4 bytes",
        ));
    }

    let mut sample_offset = Vec::with_capacity(untrusted_prealloc_hint(sample_offset_count));
    let mut offset = 4;
    for _ in 0..sample_offset_count {
        sample_offset.push(read_u32(bytes, offset));
        offset += 4;
    }

    let mut opts = Vec::with_capacity(untrusted_prealloc_hint(trailing_len / 4));
    while offset < bytes.len() {
        opts.push(AlternativeStartupEntryOpt {
            num_output_samples: read_u16(bytes, offset),
            num_total_samples: read_u16(bytes, offset + 2),
        });
        offset += 4;
    }

    Ok(AlternativeStartupEntry {
        roll_count,
        first_output_sample: read_u16(bytes, 2),
        sample_offset,
        opts,
    })
}

fn encode_visual_random_access_entry(
    field_name: &'static str,
    entry: &VisualRandomAccessEntry,
) -> Result<u8, FieldValueError> {
    if entry.num_leading_samples > 0x7f {
        return Err(invalid_value(
            field_name,
            "num leading samples does not fit in 7 bits",
        ));
    }

    Ok((u8::from(entry.num_leading_samples_known) << 7) | entry.num_leading_samples)
}

fn parse_visual_random_access_entry(byte: u8) -> VisualRandomAccessEntry {
    VisualRandomAccessEntry {
        num_leading_samples_known: byte & 0x80 != 0,
        num_leading_samples: byte & 0x7f,
    }
}

fn encode_temporal_level_entry(entry: &TemporalLevelEntry) -> u8 {
    u8::from(entry.level_independently_decodable) << 7
}

fn parse_temporal_level_entry(
    field_name: &'static str,
    byte: u8,
) -> Result<TemporalLevelEntry, FieldValueError> {
    if byte & 0x7f != 0 {
        return Err(invalid_value(
            field_name,
            "temporal level entry reserved bits must be zero",
        ));
    }

    Ok(TemporalLevelEntry {
        level_independently_decodable: byte & 0x80 != 0,
    })
}

fn format_seig_entry(entry: &SeigEntry) -> String {
    let mut rendered = format!(
        "{{Reserved={} CryptByteBlock={} SkipByteBlock={} IsProtected={} PerSampleIVSize={} KID={}",
        entry.reserved,
        entry.crypt_byte_block,
        entry.skip_byte_block,
        entry.is_protected,
        entry.per_sample_iv_size,
        render_uuid(&entry.kid)
    );
    if entry.is_protected == 1 && entry.per_sample_iv_size == 0 {
        rendered.push_str(&format!(
            " ConstantIVSize={} ConstantIV={}",
            entry.constant_iv_size,
            render_hex_bytes(&entry.constant_iv)
        ));
    }
    rendered.push('}');
    rendered
}

fn encode_seig_entry(
    field_name: &'static str,
    entry: &SeigEntry,
) -> Result<Vec<u8>, FieldValueError> {
    if entry.crypt_byte_block > 0x0f {
        return Err(invalid_value(
            field_name,
            "crypt byte block does not fit in 4 bits",
        ));
    }
    if entry.skip_byte_block > 0x0f {
        return Err(invalid_value(
            field_name,
            "skip byte block does not fit in 4 bits",
        ));
    }

    let mut bytes = Vec::with_capacity(20 + entry.constant_iv.len());
    bytes.push(entry.reserved);
    bytes.push((entry.crypt_byte_block << 4) | entry.skip_byte_block);
    bytes.push(entry.is_protected);
    bytes.push(entry.per_sample_iv_size);
    bytes.extend_from_slice(&entry.kid);

    if entry.is_protected == 1 && entry.per_sample_iv_size == 0 {
        if entry.constant_iv.len() != usize::from(entry.constant_iv_size) {
            return Err(invalid_value(
                field_name,
                "constant IV length does not match the constant IV size",
            ));
        }
        bytes.push(entry.constant_iv_size);
        bytes.extend_from_slice(&entry.constant_iv);
    }

    Ok(bytes)
}

fn parse_seig_entry(
    field_name: &'static str,
    bytes: &[u8],
) -> Result<(SeigEntry, usize), FieldValueError> {
    if bytes.len() < 20 {
        return Err(invalid_value(field_name, "seig entry is too short"));
    }

    let reserved = bytes[0];
    let crypt_and_skip = bytes[1];
    let is_protected = bytes[2];
    let per_sample_iv_size = bytes[3];
    let kid = bytes[4..20].try_into().unwrap();
    let mut consumed = 20;
    let mut constant_iv_size = 0;
    let mut constant_iv = Vec::new();

    if is_protected == 1 && per_sample_iv_size == 0 {
        if bytes.len() < consumed + 1 {
            return Err(invalid_value(
                field_name,
                "seig constant IV size is truncated",
            ));
        }
        constant_iv_size = bytes[consumed];
        consumed += 1;
        let constant_iv_len = usize::from(constant_iv_size);
        if bytes.len() < consumed + constant_iv_len {
            return Err(invalid_value(
                field_name,
                "seig constant IV exceeds the remaining payload",
            ));
        }
        constant_iv = bytes[consumed..consumed + constant_iv_len].to_vec();
        consumed += constant_iv_len;
    }

    Ok((
        SeigEntry {
            reserved,
            crypt_byte_block: crypt_and_skip >> 4,
            skip_byte_block: crypt_and_skip & 0x0f,
            is_protected,
            per_sample_iv_size,
            kid,
            constant_iv_size,
            constant_iv,
        },
        consumed,
    ))
}

fn parse_seig_entry_exact(
    field_name: &'static str,
    bytes: &[u8],
) -> Result<SeigEntry, FieldValueError> {
    let (entry, consumed) = parse_seig_entry(field_name, bytes)?;
    if consumed != bytes.len() {
        return Err(invalid_value(field_name, "seig entry has trailing bytes"));
    }
    Ok(entry)
}

/// Sample group description box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sgpd {
    full_box: FullBoxState,
    pub grouping_type: FourCc,
    pub default_length: u32,
    pub default_sample_description_index: u32,
    pub entry_count: u32,
    pub roll_distances: Vec<i16>,
    pub roll_distances_l: Vec<RollDistanceWithLength>,
    pub alternative_startup_entries: Vec<AlternativeStartupEntry>,
    pub alternative_startup_entries_l: Vec<AlternativeStartupEntryL>,
    pub visual_random_access_entries: Vec<VisualRandomAccessEntry>,
    pub visual_random_access_entries_l: Vec<VisualRandomAccessEntryL>,
    pub temporal_level_entries: Vec<TemporalLevelEntry>,
    pub temporal_level_entries_l: Vec<TemporalLevelEntryL>,
    pub seig_entries: Vec<SeigEntry>,
    pub seig_entries_l: Vec<SeigEntryL>,
    pub unsupported: Vec<u8>,
}

impl Default for Sgpd {
    fn default() -> Self {
        Self {
            full_box: FullBoxState::default(),
            grouping_type: FourCc::ANY,
            default_length: 0,
            default_sample_description_index: 0,
            entry_count: 0,
            roll_distances: Vec::new(),
            roll_distances_l: Vec::new(),
            alternative_startup_entries: Vec::new(),
            alternative_startup_entries_l: Vec::new(),
            visual_random_access_entries: Vec::new(),
            visual_random_access_entries_l: Vec::new(),
            temporal_level_entries: Vec::new(),
            temporal_level_entries_l: Vec::new(),
            seig_entries: Vec::new(),
            seig_entries_l: Vec::new(),
            unsupported: Vec::new(),
        }
    }
}

impl Sgpd {
    fn no_default_length(&self) -> bool {
        self.version() == 1 && self.default_length == 0
    }

    fn is_roll_grouping_type(&self) -> bool {
        *self.grouping_type.as_bytes() == *b"roll" || *self.grouping_type.as_bytes() == *b"prol"
    }

    fn is_alternative_startup_grouping_type(&self) -> bool {
        *self.grouping_type.as_bytes() == *b"alst"
    }

    fn is_visual_random_access_grouping_type(&self) -> bool {
        *self.grouping_type.as_bytes() == *b"rap "
    }

    fn is_temporal_level_grouping_type(&self) -> bool {
        *self.grouping_type.as_bytes() == *b"tele"
    }

    fn is_seig_grouping_type(&self) -> bool {
        *self.grouping_type.as_bytes() == *b"seig"
    }
}

impl FieldHooks for Sgpd {
    fn field_enabled(&self, name: &'static str) -> Option<bool> {
        // The active payload shape depends on both the grouping type and whether version 1 uses per-entry lengths.
        let no_default_length = self.no_default_length();
        let roll_distances = self.is_roll_grouping_type();
        let alternative_startup_entries = self.is_alternative_startup_grouping_type();
        let visual_random_access_entries = self.is_visual_random_access_grouping_type();
        let temporal_level_entries = self.is_temporal_level_grouping_type();
        let seig_entries = self.is_seig_grouping_type();

        match name {
            "RollDistances" => Some(roll_distances && !no_default_length),
            "RollDistancesL" => Some(roll_distances && no_default_length),
            "AlternativeStartupEntries" => Some(alternative_startup_entries && !no_default_length),
            "AlternativeStartupEntriesL" => Some(alternative_startup_entries && no_default_length),
            "VisualRandomAccessEntries" => Some(visual_random_access_entries && !no_default_length),
            "VisualRandomAccessEntriesL" => Some(visual_random_access_entries && no_default_length),
            "TemporalLevelEntries" => Some(temporal_level_entries && !no_default_length),
            "TemporalLevelEntriesL" => Some(temporal_level_entries && no_default_length),
            "SeigEntries" => Some(seig_entries && !no_default_length),
            "SeigEntriesL" => Some(seig_entries && no_default_length),
            "Unsupported" => Some(
                !roll_distances
                    && !alternative_startup_entries
                    && !visual_random_access_entries
                    && !temporal_level_entries
                    && !seig_entries,
            ),
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "GroupingType" => Some(quoted_fourcc(self.grouping_type)),
            "RollDistances" => Some(render_array(
                self.roll_distances.iter().map(|distance| distance.to_string()),
            )),
            "RollDistancesL" => Some(render_array(self.roll_distances_l.iter().map(|entry| {
                format!(
                    "{{DescriptionLength={} RollDistance={}}}",
                    entry.description_length, entry.roll_distance
                )
            }))),
            "AlternativeStartupEntries" => Some(render_array(
                self.alternative_startup_entries
                    .iter()
                    .map(format_alternative_startup_entry),
            )),
            "AlternativeStartupEntriesL" => Some(render_array(
                self.alternative_startup_entries_l.iter().map(|entry| {
                    format!(
                        "{{DescriptionLength={} {}}}",
                        entry.description_length,
                        format_alternative_startup_entry(&entry.alternative_startup_entry)
                            .trim_start_matches('{')
                            .trim_end_matches('}')
                    )
                }),
            )),
            "VisualRandomAccessEntries" => Some(render_array(
                self.visual_random_access_entries.iter().map(|entry| {
                    format!(
                        "{{NumLeadingSamplesKnown={} NumLeadingSamples=0x{:x}}}",
                        entry.num_leading_samples_known, entry.num_leading_samples
                    )
                }),
            )),
            "VisualRandomAccessEntriesL" => Some(render_array(
                self.visual_random_access_entries_l.iter().map(|entry| {
                    format!(
                        "{{DescriptionLength={} NumLeadingSamplesKnown={} NumLeadingSamples=0x{:x}}}",
                        entry.description_length,
                        entry.visual_random_access_entry.num_leading_samples_known,
                        entry.visual_random_access_entry.num_leading_samples
                    )
                }),
            )),
            "TemporalLevelEntries" => Some(render_array(
                self.temporal_level_entries.iter().map(|entry| {
                    format!(
                        "{{LevelIndependentlyDecodable={}}}",
                        entry.level_independently_decodable
                    )
                }),
            )),
            "TemporalLevelEntriesL" => Some(render_array(
                self.temporal_level_entries_l.iter().map(|entry| {
                    format!(
                        "{{DescriptionLength={} LevelIndependentlyDecodable={}}}",
                        entry.description_length,
                        entry.temporal_level_entry.level_independently_decodable
                    )
                }),
            )),
            "SeigEntries" => Some(render_array(
                self.seig_entries.iter().map(format_seig_entry),
            )),
            "SeigEntriesL" => Some(render_array(self.seig_entries_l.iter().map(|entry| {
                format!(
                    "{{DescriptionLength={} {}}}",
                    entry.description_length,
                    format_seig_entry(&entry.seig_entry)
                        .trim_start_matches('{')
                        .trim_end_matches('}')
                )
            }))),
            _ => None,
        }
    }
}

impl_full_box!(Sgpd, *b"sgpd");

impl FieldValueRead for Sgpd {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "GroupingType" => Ok(FieldValue::Bytes(self.grouping_type.as_bytes().to_vec())),
            "DefaultLength" => Ok(FieldValue::Unsigned(u64::from(self.default_length))),
            "DefaultSampleDescriptionIndex" => Ok(FieldValue::Unsigned(u64::from(
                self.default_sample_description_index,
            ))),
            "EntryCount" => Ok(FieldValue::Unsigned(u64::from(self.entry_count))),
            "RollDistances" => {
                require_count(field_name, self.entry_count, self.roll_distances.len())?;
                let mut bytes = Vec::with_capacity(self.roll_distances.len() * 2);
                for roll_distance in &self.roll_distances {
                    bytes.extend_from_slice(&roll_distance.to_be_bytes());
                }
                Ok(FieldValue::Bytes(bytes))
            }
            "RollDistancesL" => {
                require_count(field_name, self.entry_count, self.roll_distances_l.len())?;
                let mut bytes = Vec::with_capacity(self.roll_distances_l.len() * 6);
                for entry in &self.roll_distances_l {
                    bytes.extend_from_slice(&entry.description_length.to_be_bytes());
                    bytes.extend_from_slice(&entry.roll_distance.to_be_bytes());
                }
                Ok(FieldValue::Bytes(bytes))
            }
            "AlternativeStartupEntries" => {
                require_count(
                    field_name,
                    self.entry_count,
                    self.alternative_startup_entries.len(),
                )?;
                let mut bytes = Vec::new();
                for entry in &self.alternative_startup_entries {
                    let encoded = encode_alternative_startup_entry(field_name, entry)?;
                    if self.default_length != 0 && encoded.len() != self.default_length as usize {
                        return Err(invalid_value(
                            field_name,
                            "alternative startup entry does not match the default length",
                        ));
                    }
                    bytes.extend_from_slice(&encoded);
                }
                Ok(FieldValue::Bytes(bytes))
            }
            "AlternativeStartupEntriesL" => {
                require_count(
                    field_name,
                    self.entry_count,
                    self.alternative_startup_entries_l.len(),
                )?;
                let mut bytes = Vec::new();
                for entry in &self.alternative_startup_entries_l {
                    let encoded = encode_alternative_startup_entry(
                        field_name,
                        &entry.alternative_startup_entry,
                    )?;
                    if encoded.len() != entry.description_length as usize {
                        return Err(invalid_value(
                            field_name,
                            "alternative startup entry length does not match the description length",
                        ));
                    }
                    bytes.extend_from_slice(&entry.description_length.to_be_bytes());
                    bytes.extend_from_slice(&encoded);
                }
                Ok(FieldValue::Bytes(bytes))
            }
            "VisualRandomAccessEntries" => {
                require_count(
                    field_name,
                    self.entry_count,
                    self.visual_random_access_entries.len(),
                )?;
                let mut bytes = Vec::with_capacity(self.visual_random_access_entries.len());
                for entry in &self.visual_random_access_entries {
                    bytes.push(encode_visual_random_access_entry(field_name, entry)?);
                }
                Ok(FieldValue::Bytes(bytes))
            }
            "VisualRandomAccessEntriesL" => {
                require_count(
                    field_name,
                    self.entry_count,
                    self.visual_random_access_entries_l.len(),
                )?;
                let mut bytes = Vec::new();
                for entry in &self.visual_random_access_entries_l {
                    if entry.description_length != 1 {
                        return Err(invalid_value(
                            field_name,
                            "visual random access entries with explicit lengths must be 1 byte",
                        ));
                    }
                    bytes.extend_from_slice(&entry.description_length.to_be_bytes());
                    bytes.push(encode_visual_random_access_entry(
                        field_name,
                        &entry.visual_random_access_entry,
                    )?);
                }
                Ok(FieldValue::Bytes(bytes))
            }
            "TemporalLevelEntries" => {
                require_count(
                    field_name,
                    self.entry_count,
                    self.temporal_level_entries.len(),
                )?;
                Ok(FieldValue::Bytes(
                    self.temporal_level_entries
                        .iter()
                        .map(encode_temporal_level_entry)
                        .collect(),
                ))
            }
            "TemporalLevelEntriesL" => {
                require_count(
                    field_name,
                    self.entry_count,
                    self.temporal_level_entries_l.len(),
                )?;
                let mut bytes = Vec::new();
                for entry in &self.temporal_level_entries_l {
                    if entry.description_length != 1 {
                        return Err(invalid_value(
                            field_name,
                            "temporal level entries with explicit lengths must be 1 byte",
                        ));
                    }
                    bytes.extend_from_slice(&entry.description_length.to_be_bytes());
                    bytes.push(encode_temporal_level_entry(&entry.temporal_level_entry));
                }
                Ok(FieldValue::Bytes(bytes))
            }
            "SeigEntries" => {
                require_count(field_name, self.entry_count, self.seig_entries.len())?;
                let mut bytes = Vec::new();
                for entry in &self.seig_entries {
                    let encoded = encode_seig_entry(field_name, entry)?;
                    if self.version() == 1
                        && self.default_length != 0
                        && encoded.len() != self.default_length as usize
                    {
                        return Err(invalid_value(
                            field_name,
                            "seig entry does not match the default length",
                        ));
                    }
                    bytes.extend_from_slice(&encoded);
                }
                Ok(FieldValue::Bytes(bytes))
            }
            "SeigEntriesL" => {
                require_count(field_name, self.entry_count, self.seig_entries_l.len())?;
                let mut bytes = Vec::new();
                for entry in &self.seig_entries_l {
                    let encoded = encode_seig_entry(field_name, &entry.seig_entry)?;
                    if encoded.len() != entry.description_length as usize {
                        return Err(invalid_value(
                            field_name,
                            "seig entry length does not match the description length",
                        ));
                    }
                    bytes.extend_from_slice(&entry.description_length.to_be_bytes());
                    bytes.extend_from_slice(&encoded);
                }
                Ok(FieldValue::Bytes(bytes))
            }
            "Unsupported" => Ok(FieldValue::Bytes(self.unsupported.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Sgpd {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("GroupingType", FieldValue::Bytes(bytes)) => {
                self.grouping_type = bytes_to_fourcc(field_name, bytes)?;
                Ok(())
            }
            ("DefaultLength", FieldValue::Unsigned(value)) => {
                self.default_length = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DefaultSampleDescriptionIndex", FieldValue::Unsigned(value)) => {
                self.default_sample_description_index = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("EntryCount", FieldValue::Unsigned(value)) => {
                self.entry_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("RollDistances", FieldValue::Bytes(bytes)) => {
                let expected_len = field_len_from_count(self.entry_count, 2)
                    .map(|len| len as usize)
                    .unwrap_or(0);
                if bytes.len() != expected_len {
                    return Err(invalid_value(
                        field_name,
                        "roll distance payload length does not match the entry count",
                    ));
                }
                self.roll_distances =
                    parse_fixed_chunks(field_name, &bytes, 2, |chunk| read_i16(chunk, 0))?;
                Ok(())
            }
            ("RollDistancesL", FieldValue::Bytes(bytes)) => {
                let expected_len = field_len_from_count(self.entry_count, 6)
                    .map(|len| len as usize)
                    .unwrap_or(0);
                if bytes.len() != expected_len {
                    return Err(invalid_value(
                        field_name,
                        "roll distance payload length does not match the entry count",
                    ));
                }
                self.roll_distances_l =
                    parse_fixed_chunks(field_name, &bytes, 6, |chunk| RollDistanceWithLength {
                        description_length: read_u32(chunk, 0),
                        roll_distance: read_i16(chunk, 4),
                    })?;
                Ok(())
            }
            ("AlternativeStartupEntries", FieldValue::Bytes(bytes)) => {
                let entry_len = usize::try_from(self.default_length)
                    .map_err(|_| invalid_value(field_name, "default length is too large"))?;
                if entry_len == 0 {
                    return Err(invalid_value(
                        field_name,
                        "default length must be non-zero for alternative startup entries",
                    ));
                }
                let expected_len = field_len_from_count(self.entry_count, entry_len)
                    .map(|len| len as usize)
                    .unwrap_or(0);
                if bytes.len() != expected_len {
                    return Err(invalid_value(
                        field_name,
                        "alternative startup payload length does not match the entry count",
                    ));
                }
                self.alternative_startup_entries = bytes
                    .chunks_exact(entry_len)
                    .map(|chunk| parse_alternative_startup_entry(field_name, chunk))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(())
            }
            ("AlternativeStartupEntriesL", FieldValue::Bytes(bytes)) => {
                let mut cursor = 0;
                let mut entries = Vec::new();
                while cursor < bytes.len() {
                    if bytes.len() - cursor < 4 {
                        return Err(invalid_value(
                            field_name,
                            "alternative startup entry length prefix is truncated",
                        ));
                    }
                    let description_length = read_u32(&bytes, cursor);
                    cursor += 4;
                    let description_len = usize::try_from(description_length).map_err(|_| {
                        invalid_value(field_name, "alternative startup description is too large")
                    })?;
                    if bytes.len() - cursor < description_len {
                        return Err(invalid_value(
                            field_name,
                            "alternative startup entry exceeds the remaining payload",
                        ));
                    }
                    let payload = &bytes[cursor..cursor + description_len];
                    cursor += description_len;
                    entries.push(AlternativeStartupEntryL {
                        description_length,
                        alternative_startup_entry: parse_alternative_startup_entry(
                            field_name, payload,
                        )?,
                    });
                }
                require_count(field_name, self.entry_count, entries.len())?;
                self.alternative_startup_entries_l = entries;
                Ok(())
            }
            ("VisualRandomAccessEntries", FieldValue::Bytes(bytes)) => {
                require_count(field_name, self.entry_count, bytes.len())?;
                self.visual_random_access_entries = bytes
                    .into_iter()
                    .map(parse_visual_random_access_entry)
                    .collect();
                Ok(())
            }
            ("VisualRandomAccessEntriesL", FieldValue::Bytes(bytes)) => {
                let mut cursor = 0;
                let mut entries = Vec::new();
                while cursor < bytes.len() {
                    if bytes.len() - cursor < 5 {
                        return Err(invalid_value(
                            field_name,
                            "visual random access entry is truncated",
                        ));
                    }
                    let description_length = read_u32(&bytes, cursor);
                    cursor += 4;
                    if description_length != 1 {
                        return Err(invalid_value(
                            field_name,
                            "visual random access entries with explicit lengths must be 1 byte",
                        ));
                    }
                    entries.push(VisualRandomAccessEntryL {
                        description_length,
                        visual_random_access_entry: parse_visual_random_access_entry(bytes[cursor]),
                    });
                    cursor += 1;
                }
                require_count(field_name, self.entry_count, entries.len())?;
                self.visual_random_access_entries_l = entries;
                Ok(())
            }
            ("TemporalLevelEntries", FieldValue::Bytes(bytes)) => {
                require_count(field_name, self.entry_count, bytes.len())?;
                self.temporal_level_entries = bytes
                    .into_iter()
                    .map(|byte| parse_temporal_level_entry(field_name, byte))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(())
            }
            ("TemporalLevelEntriesL", FieldValue::Bytes(bytes)) => {
                let mut cursor = 0;
                let mut entries = Vec::new();
                while cursor < bytes.len() {
                    if bytes.len() - cursor < 5 {
                        return Err(invalid_value(
                            field_name,
                            "temporal level entry is truncated",
                        ));
                    }
                    let description_length = read_u32(&bytes, cursor);
                    cursor += 4;
                    if description_length != 1 {
                        return Err(invalid_value(
                            field_name,
                            "temporal level entries with explicit lengths must be 1 byte",
                        ));
                    }
                    entries.push(TemporalLevelEntryL {
                        description_length,
                        temporal_level_entry: parse_temporal_level_entry(
                            field_name,
                            bytes[cursor],
                        )?,
                    });
                    cursor += 1;
                }
                require_count(field_name, self.entry_count, entries.len())?;
                self.temporal_level_entries_l = entries;
                Ok(())
            }
            ("SeigEntries", FieldValue::Bytes(bytes)) => {
                if self.version() == 1 {
                    let entry_len = usize::try_from(self.default_length)
                        .map_err(|_| invalid_value(field_name, "default length is too large"))?;
                    if entry_len == 0 {
                        return Err(invalid_value(
                            field_name,
                            "default length must be non-zero for seig entries",
                        ));
                    }
                    let expected_len = field_len_from_count(self.entry_count, entry_len)
                        .map(|len| len as usize)
                        .unwrap_or(0);
                    if bytes.len() != expected_len {
                        return Err(invalid_value(
                            field_name,
                            "seig payload length does not match the entry count",
                        ));
                    }
                    self.seig_entries = bytes
                        .chunks_exact(entry_len)
                        .map(|chunk| parse_seig_entry_exact(field_name, chunk))
                        .collect::<Result<Vec<_>, _>>()?;
                    return Ok(());
                }

                let mut cursor = 0;
                let mut entries = Vec::new();
                while cursor < bytes.len() {
                    let (entry, consumed) = parse_seig_entry(field_name, &bytes[cursor..])?;
                    cursor += consumed;
                    entries.push(entry);
                }
                require_count(field_name, self.entry_count, entries.len())?;
                self.seig_entries = entries;
                Ok(())
            }
            ("SeigEntriesL", FieldValue::Bytes(bytes)) => {
                let mut cursor = 0;
                let mut entries = Vec::new();
                while cursor < bytes.len() {
                    if bytes.len() - cursor < 4 {
                        return Err(invalid_value(
                            field_name,
                            "seig entry length prefix is truncated",
                        ));
                    }
                    let description_length = read_u32(&bytes, cursor);
                    cursor += 4;
                    let description_len = usize::try_from(description_length)
                        .map_err(|_| invalid_value(field_name, "seig description is too large"))?;
                    if bytes.len() - cursor < description_len {
                        return Err(invalid_value(
                            field_name,
                            "seig entry exceeds the remaining payload",
                        ));
                    }
                    let payload = &bytes[cursor..cursor + description_len];
                    cursor += description_len;
                    entries.push(SeigEntryL {
                        description_length,
                        seig_entry: parse_seig_entry_exact(field_name, payload)?,
                    });
                }
                require_count(field_name, self.entry_count, entries.len())?;
                self.seig_entries_l = entries;
                Ok(())
            }
            ("Unsupported", FieldValue::Bytes(bytes)) => {
                self.unsupported = bytes;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Sgpd {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!(
            "GroupingType",
            2,
            with_bit_width(8),
            with_length(4),
            as_bytes()
        ),
        codec_field!("DefaultLength", 3, with_bit_width(32), with_version(1)),
        codec_field!(
            "DefaultSampleDescriptionIndex",
            4,
            with_bit_width(32),
            with_version(2)
        ),
        codec_field!("EntryCount", 5, with_bit_width(32)),
        codec_field!(
            "RollDistances",
            6,
            with_bit_width(8),
            as_bytes(),
            with_dynamic_presence()
        ),
        codec_field!(
            "RollDistancesL",
            7,
            with_bit_width(8),
            as_bytes(),
            with_dynamic_presence()
        ),
        codec_field!(
            "AlternativeStartupEntries",
            8,
            with_bit_width(8),
            as_bytes(),
            with_dynamic_presence()
        ),
        codec_field!(
            "AlternativeStartupEntriesL",
            9,
            with_bit_width(8),
            as_bytes(),
            with_dynamic_presence()
        ),
        codec_field!(
            "VisualRandomAccessEntries",
            10,
            with_bit_width(8),
            as_bytes(),
            with_dynamic_presence()
        ),
        codec_field!(
            "VisualRandomAccessEntriesL",
            11,
            with_bit_width(8),
            as_bytes(),
            with_dynamic_presence()
        ),
        codec_field!(
            "TemporalLevelEntries",
            12,
            with_bit_width(8),
            as_bytes(),
            with_dynamic_presence()
        ),
        codec_field!(
            "TemporalLevelEntriesL",
            13,
            with_bit_width(8),
            as_bytes(),
            with_dynamic_presence()
        ),
        codec_field!(
            "SeigEntries",
            14,
            with_bit_width(8),
            as_bytes(),
            with_dynamic_presence()
        ),
        codec_field!(
            "SeigEntriesL",
            15,
            with_bit_width(8),
            as_bytes(),
            with_dynamic_presence()
        ),
        codec_field!(
            "Unsupported",
            16,
            with_bit_width(8),
            as_bytes(),
            with_dynamic_presence()
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[1, 2];
}

/// One indexed byte range inside an [`SsixSubsegment`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SsixRange {
    pub level: u8,
    pub range_size: u32,
}

/// One subsegment entry inside [`Ssix`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SsixSubsegment {
    pub range_count: u32,
    pub ranges: Vec<SsixRange>,
}

/// Subsegment-index box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ssix {
    full_box: FullBoxState,
    pub subsegment_count: u32,
    pub subsegments: Vec<SsixSubsegment>,
}

fn format_ssix_subsegments(subsegments: &[SsixSubsegment]) -> String {
    render_array(subsegments.iter().map(|subsegment| {
        format!(
            "{{RangeCount={} Ranges={}}}",
            subsegment.range_count,
            render_array(subsegment.ranges.iter().map(|range| {
                format!("{{Level={} RangeSize={}}}", range.level, range.range_size)
            }))
        )
    }))
}

fn encode_ssix_subsegments(
    field_name: &'static str,
    subsegments: &[SsixSubsegment],
) -> Result<Vec<u8>, FieldValueError> {
    let mut bytes = Vec::new();
    for subsegment in subsegments {
        require_count(field_name, subsegment.range_count, subsegment.ranges.len())?;
        bytes.extend_from_slice(&subsegment.range_count.to_be_bytes());
        for range in &subsegment.ranges {
            if range.range_size > 0x00ff_ffff {
                return Err(invalid_value(
                    field_name,
                    "range size does not fit in 24 bits",
                ));
            }
            bytes.push(range.level);
            push_uint(field_name, &mut bytes, 3, u64::from(range.range_size))?;
        }
    }
    Ok(bytes)
}

fn parse_ssix_subsegments(
    field_name: &'static str,
    subsegment_count: u32,
    bytes: &[u8],
) -> Result<Vec<SsixSubsegment>, FieldValueError> {
    let mut subsegments = Vec::with_capacity(untrusted_prealloc_hint(
        usize::try_from(subsegment_count).unwrap_or(0),
    ));
    let mut offset = 0usize;

    for _ in 0..subsegment_count {
        if bytes.len().saturating_sub(offset) < 4 {
            return Err(invalid_value(field_name, "subsegment payload is truncated"));
        }

        let range_count = read_u32(bytes, offset);
        offset += 4;
        let mut ranges = Vec::with_capacity(untrusted_prealloc_hint(
            usize::try_from(range_count).unwrap_or(0),
        ));
        for _ in 0..range_count {
            if bytes.len().saturating_sub(offset) < 4 {
                return Err(invalid_value(field_name, "range payload is truncated"));
            }

            ranges.push(SsixRange {
                level: bytes[offset],
                range_size: read_uint(bytes, offset + 1, 3) as u32,
            });
            offset += 4;
        }

        subsegments.push(SsixSubsegment {
            range_count,
            ranges,
        });
    }

    if offset != bytes.len() {
        return Err(invalid_value(
            field_name,
            "subsegment payload length does not match the subsegment count",
        ));
    }

    Ok(subsegments)
}

impl FieldHooks for Ssix {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Subsegments" => Some(format_ssix_subsegments(&self.subsegments)),
            _ => None,
        }
    }
}

impl_full_box!(Ssix, *b"ssix");

impl FieldValueRead for Ssix {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "SubsegmentCount" => Ok(FieldValue::Unsigned(u64::from(self.subsegment_count))),
            "Subsegments" => {
                require_count(field_name, self.subsegment_count, self.subsegments.len())?;
                Ok(FieldValue::Bytes(encode_ssix_subsegments(
                    field_name,
                    &self.subsegments,
                )?))
            }
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Ssix {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("SubsegmentCount", FieldValue::Unsigned(value)) => {
                self.subsegment_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Subsegments", FieldValue::Bytes(bytes)) => {
                self.subsegments =
                    parse_ssix_subsegments(field_name, self.subsegment_count, &bytes)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Ssix {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("SubsegmentCount", 2, with_bit_width(32)),
        codec_field!("Subsegments", 3, with_bit_width(8), as_bytes()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// One segment index reference entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SidxReference {
    pub reference_type: bool,
    pub referenced_size: u32,
    pub subsegment_duration: u32,
    pub starts_with_sap: bool,
    pub sap_type: u32,
    pub sap_delta_time: u32,
}

/// Segment index box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sidx {
    full_box: FullBoxState,
    pub reference_id: u32,
    pub timescale: u32,
    pub earliest_presentation_time_v0: u32,
    pub first_offset_v0: u32,
    pub earliest_presentation_time_v1: u64,
    pub first_offset_v1: u64,
    pub reference_count: u16,
    pub references: Vec<SidxReference>,
}

impl FieldHooks for Sidx {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "References" => Some(render_array(self.references.iter().map(|entry| {
                format!(
                    "{{ReferenceType={} ReferencedSize={} SubsegmentDuration={} StartsWithSAP={} SAPType={} SAPDeltaTime={}}}",
                    entry.reference_type,
                    entry.referenced_size,
                    entry.subsegment_duration,
                    entry.starts_with_sap,
                    entry.sap_type,
                    entry.sap_delta_time
                )
            }))),
            _ => None,
        }
    }
}

impl_full_box!(Sidx, *b"sidx");

impl Sidx {
    /// Returns the active earliest presentation time for the current box version.
    pub fn earliest_presentation_time(&self) -> u64 {
        match self.version() {
            0 => u64::from(self.earliest_presentation_time_v0),
            1 => self.earliest_presentation_time_v1,
            _ => 0,
        }
    }

    /// Returns the active first offset for the current box version.
    pub fn first_offset(&self) -> u64 {
        match self.version() {
            0 => u64::from(self.first_offset_v0),
            1 => self.first_offset_v1,
            _ => 0,
        }
    }
}

impl FieldValueRead for Sidx {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "ReferenceID" => Ok(FieldValue::Unsigned(u64::from(self.reference_id))),
            "Timescale" => Ok(FieldValue::Unsigned(u64::from(self.timescale))),
            "EarliestPresentationTimeV0" => Ok(FieldValue::Unsigned(u64::from(
                self.earliest_presentation_time_v0,
            ))),
            "FirstOffsetV0" => Ok(FieldValue::Unsigned(u64::from(self.first_offset_v0))),
            "EarliestPresentationTimeV1" => {
                Ok(FieldValue::Unsigned(self.earliest_presentation_time_v1))
            }
            "FirstOffsetV1" => Ok(FieldValue::Unsigned(self.first_offset_v1)),
            "ReferenceCount" => Ok(FieldValue::Unsigned(u64::from(self.reference_count))),
            "References" => {
                require_count(
                    field_name,
                    u32::from(self.reference_count),
                    self.references.len(),
                )?;
                let mut bytes = Vec::with_capacity(self.references.len() * 12);
                for entry in &self.references {
                    if entry.referenced_size > 0x7fff_ffff {
                        return Err(invalid_value(
                            field_name,
                            "referenced size does not fit in 31 bits",
                        ));
                    }
                    if entry.sap_type > 0x07 {
                        return Err(invalid_value(field_name, "SAP type does not fit in 3 bits"));
                    }
                    if entry.sap_delta_time > 0x0fff_ffff {
                        return Err(invalid_value(
                            field_name,
                            "SAP delta time does not fit in 28 bits",
                        ));
                    }

                    // The reference and SAP records pack their high-bit flags into the same 32-bit words as the payload values.
                    let reference_word =
                        (u32::from(entry.reference_type) << 31) | entry.referenced_size;
                    let sap_word = (u32::from(entry.starts_with_sap) << 31)
                        | (entry.sap_type << 28)
                        | entry.sap_delta_time;
                    bytes.extend_from_slice(&reference_word.to_be_bytes());
                    bytes.extend_from_slice(&entry.subsegment_duration.to_be_bytes());
                    bytes.extend_from_slice(&sap_word.to_be_bytes());
                }
                Ok(FieldValue::Bytes(bytes))
            }
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Sidx {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("ReferenceID", FieldValue::Unsigned(value)) => {
                self.reference_id = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Timescale", FieldValue::Unsigned(value)) => {
                self.timescale = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("EarliestPresentationTimeV0", FieldValue::Unsigned(value)) => {
                self.earliest_presentation_time_v0 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("FirstOffsetV0", FieldValue::Unsigned(value)) => {
                self.first_offset_v0 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("EarliestPresentationTimeV1", FieldValue::Unsigned(value)) => {
                self.earliest_presentation_time_v1 = value;
                Ok(())
            }
            ("FirstOffsetV1", FieldValue::Unsigned(value)) => {
                self.first_offset_v1 = value;
                Ok(())
            }
            ("ReferenceCount", FieldValue::Unsigned(value)) => {
                self.reference_count = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("References", FieldValue::Bytes(bytes)) => {
                let expected_len = field_len_from_count(u32::from(self.reference_count), 12)
                    .map(|len| len as usize)
                    .unwrap_or(0);
                if bytes.len() != expected_len {
                    return Err(invalid_value(
                        field_name,
                        "reference payload length does not match the reference count",
                    ));
                }

                self.references =
                    parse_fixed_chunks(field_name, &bytes, 12, |chunk| SidxReference {
                        reference_type: read_u32(chunk, 0) & 0x8000_0000 != 0,
                        referenced_size: read_u32(chunk, 0) & 0x7fff_ffff,
                        subsegment_duration: read_u32(chunk, 4),
                        starts_with_sap: read_u32(chunk, 8) & 0x8000_0000 != 0,
                        sap_type: (read_u32(chunk, 8) >> 28) & 0x07,
                        sap_delta_time: read_u32(chunk, 8) & 0x0fff_ffff,
                    })?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Sidx {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("ReferenceID", 2, with_bit_width(32)),
        codec_field!("Timescale", 3, with_bit_width(32)),
        codec_field!(
            "EarliestPresentationTimeV0",
            4,
            with_bit_width(32),
            with_version(0)
        ),
        codec_field!("FirstOffsetV0", 5, with_bit_width(32), with_version(0)),
        codec_field!(
            "EarliestPresentationTimeV1",
            6,
            with_bit_width(64),
            with_version(1)
        ),
        codec_field!("FirstOffsetV1", 7, with_bit_width(64), with_version(1)),
        codec_field!("Reserved", 8, with_bit_width(16), with_constant("0")),
        codec_field!("ReferenceCount", 9, with_bit_width(16)),
        codec_field!("References", 10, with_bit_width(8), as_bytes()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// One track-fragment random-access entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TfraEntry {
    pub time_v0: u32,
    pub moof_offset_v0: u32,
    pub time_v1: u64,
    pub moof_offset_v1: u64,
    pub traf_number: u32,
    pub trun_number: u32,
    pub sample_number: u32,
}

/// Track fragment random access box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tfra {
    full_box: FullBoxState,
    pub track_id: u32,
    pub length_size_of_traf_num: u8,
    pub length_size_of_trun_num: u8,
    pub length_size_of_sample_num: u8,
    pub number_of_entry: u32,
    pub entries: Vec<TfraEntry>,
}

impl Tfra {
    fn entry_size_bytes(&self) -> usize {
        // Each stored length field is encoded as "size minus one", so add one byte to recover the actual width.
        let traf_bytes = usize::from(self.length_size_of_traf_num) + 1;
        let trun_bytes = usize::from(self.length_size_of_trun_num) + 1;
        let sample_bytes = usize::from(self.length_size_of_sample_num) + 1;
        match self.version() {
            0 => 8 + traf_bytes + trun_bytes + sample_bytes,
            1 => 16 + traf_bytes + trun_bytes + sample_bytes,
            _ => traf_bytes + trun_bytes + sample_bytes,
        }
    }

    /// Returns the active random-access time for `index`.
    pub fn time(&self, index: usize) -> u64 {
        match self.version() {
            0 => u64::from(self.entries[index].time_v0),
            1 => self.entries[index].time_v1,
            _ => 0,
        }
    }

    /// Returns the active `moof` offset for `index`.
    pub fn moof_offset(&self, index: usize) -> u64 {
        match self.version() {
            0 => u64::from(self.entries[index].moof_offset_v0),
            1 => self.entries[index].moof_offset_v1,
            _ => 0,
        }
    }
}

impl FieldHooks for Tfra {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Entries" => Some(render_array(self.entries.iter().map(|entry| {
                if self.version() == 0 {
                    format!(
                        "{{TimeV0={} MoofOffsetV0={} TrafNumber={} TrunNumber={} SampleNumber={}}}",
                        entry.time_v0,
                        entry.moof_offset_v0,
                        entry.traf_number,
                        entry.trun_number,
                        entry.sample_number
                    )
                } else {
                    format!(
                        "{{TimeV1={} MoofOffsetV1={} TrafNumber={} TrunNumber={} SampleNumber={}}}",
                        entry.time_v1,
                        entry.moof_offset_v1,
                        entry.traf_number,
                        entry.trun_number,
                        entry.sample_number
                    )
                }
            }))),
            _ => None,
        }
    }
}

impl_full_box!(Tfra, *b"tfra");

impl FieldValueRead for Tfra {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "TrackID" => Ok(FieldValue::Unsigned(u64::from(self.track_id))),
            "LengthSizeOfTrafNum" => Ok(FieldValue::Unsigned(u64::from(
                self.length_size_of_traf_num,
            ))),
            "LengthSizeOfTrunNum" => Ok(FieldValue::Unsigned(u64::from(
                self.length_size_of_trun_num,
            ))),
            "LengthSizeOfSampleNum" => Ok(FieldValue::Unsigned(u64::from(
                self.length_size_of_sample_num,
            ))),
            "NumberOfEntry" => Ok(FieldValue::Unsigned(u64::from(self.number_of_entry))),
            "Entries" => {
                require_count(field_name, self.number_of_entry, self.entries.len())?;
                let traf_bytes = usize::from(self.length_size_of_traf_num) + 1;
                let trun_bytes = usize::from(self.length_size_of_trun_num) + 1;
                let sample_bytes = usize::from(self.length_size_of_sample_num) + 1;
                let mut bytes = Vec::with_capacity(self.entries.len() * self.entry_size_bytes());
                for entry in &self.entries {
                    if self.version() == 0 {
                        bytes.extend_from_slice(&entry.time_v0.to_be_bytes());
                        bytes.extend_from_slice(&entry.moof_offset_v0.to_be_bytes());
                    } else {
                        bytes.extend_from_slice(&entry.time_v1.to_be_bytes());
                        bytes.extend_from_slice(&entry.moof_offset_v1.to_be_bytes());
                    }
                    push_uint(
                        field_name,
                        &mut bytes,
                        traf_bytes,
                        u64::from(entry.traf_number),
                    )?;
                    push_uint(
                        field_name,
                        &mut bytes,
                        trun_bytes,
                        u64::from(entry.trun_number),
                    )?;
                    push_uint(
                        field_name,
                        &mut bytes,
                        sample_bytes,
                        u64::from(entry.sample_number),
                    )?;
                }
                Ok(FieldValue::Bytes(bytes))
            }
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Tfra {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("TrackID", FieldValue::Unsigned(value)) => {
                self.track_id = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("LengthSizeOfTrafNum", FieldValue::Unsigned(value)) => {
                self.length_size_of_traf_num = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("LengthSizeOfTrunNum", FieldValue::Unsigned(value)) => {
                self.length_size_of_trun_num = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("LengthSizeOfSampleNum", FieldValue::Unsigned(value)) => {
                self.length_size_of_sample_num = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("NumberOfEntry", FieldValue::Unsigned(value)) => {
                self.number_of_entry = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Entries", FieldValue::Bytes(bytes)) => {
                let entry_size = self.entry_size_bytes();
                let expected_len = field_len_from_count(self.number_of_entry, entry_size)
                    .map(|len| len as usize)
                    .unwrap_or(0);
                if bytes.len() != expected_len {
                    return Err(invalid_value(
                        field_name,
                        "random access payload length does not match the entry count",
                    ));
                }

                let traf_bytes = usize::from(self.length_size_of_traf_num) + 1;
                let trun_bytes = usize::from(self.length_size_of_trun_num) + 1;
                let sample_bytes = usize::from(self.length_size_of_sample_num) + 1;
                self.entries = bytes
                    .chunks_exact(entry_size)
                    .map(|chunk| {
                        let mut offset = 0;
                        let mut entry = TfraEntry::default();
                        if self.version() == 0 {
                            entry.time_v0 = read_u32(chunk, offset);
                            offset += 4;
                            entry.moof_offset_v0 = read_u32(chunk, offset);
                            offset += 4;
                        } else {
                            entry.time_v1 = read_u64(chunk, offset);
                            offset += 8;
                            entry.moof_offset_v1 = read_u64(chunk, offset);
                            offset += 8;
                        }
                        entry.traf_number =
                            u32_from_unsigned(field_name, read_uint(chunk, offset, traf_bytes))?;
                        offset += traf_bytes;
                        entry.trun_number =
                            u32_from_unsigned(field_name, read_uint(chunk, offset, trun_bytes))?;
                        offset += trun_bytes;
                        entry.sample_number =
                            u32_from_unsigned(field_name, read_uint(chunk, offset, sample_bytes))?;
                        Ok(entry)
                    })
                    .collect::<Result<Vec<_>, FieldValueError>>()?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Tfra {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("TrackID", 2, with_bit_width(32)),
        codec_field!("Reserved", 3, with_bit_width(26), with_constant("0")),
        codec_field!("LengthSizeOfTrafNum", 4, with_bit_width(2), as_hex()),
        codec_field!("LengthSizeOfTrunNum", 5, with_bit_width(2), as_hex()),
        codec_field!("LengthSizeOfSampleNum", 6, with_bit_width(2), as_hex()),
        codec_field!("NumberOfEntry", 7, with_bit_width(32)),
        codec_field!("Entries", 8, with_bit_width(8), as_bytes()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// Bitrate declaration box for sample entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Btrt {
    pub buffer_size_db: u32,
    pub max_bitrate: u32,
    pub avg_bitrate: u32,
}

impl_leaf_box!(Btrt, *b"btrt");

impl FieldValueRead for Btrt {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "BufferSizeDB" => Ok(FieldValue::Unsigned(u64::from(self.buffer_size_db))),
            "MaxBitrate" => Ok(FieldValue::Unsigned(u64::from(self.max_bitrate))),
            "AvgBitrate" => Ok(FieldValue::Unsigned(u64::from(self.avg_bitrate))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Btrt {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("BufferSizeDB", FieldValue::Unsigned(value)) => {
                self.buffer_size_db = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("MaxBitrate", FieldValue::Unsigned(value)) => {
                self.max_bitrate = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("AvgBitrate", FieldValue::Unsigned(value)) => {
                self.avg_bitrate = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Btrt {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("BufferSizeDB", 0, with_bit_width(32)),
        codec_field!("MaxBitrate", 1, with_bit_width(32)),
        codec_field!("AvgBitrate", 2, with_bit_width(32)),
    ]);
}

/// Clean-aperture box that refines the displayed picture region for a visual sample entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Clap {
    pub clean_aperture_width_n: u32,
    pub clean_aperture_width_d: u32,
    pub clean_aperture_height_n: u32,
    pub clean_aperture_height_d: u32,
    pub horiz_off_n: u32,
    pub horiz_off_d: u32,
    pub vert_off_n: u32,
    pub vert_off_d: u32,
}

impl_leaf_box!(Clap, *b"clap");

impl FieldValueRead for Clap {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "CleanApertureWidthN" => {
                Ok(FieldValue::Unsigned(u64::from(self.clean_aperture_width_n)))
            }
            "CleanApertureWidthD" => {
                Ok(FieldValue::Unsigned(u64::from(self.clean_aperture_width_d)))
            }
            "CleanApertureHeightN" => Ok(FieldValue::Unsigned(u64::from(
                self.clean_aperture_height_n,
            ))),
            "CleanApertureHeightD" => Ok(FieldValue::Unsigned(u64::from(
                self.clean_aperture_height_d,
            ))),
            "HorizOffN" => Ok(FieldValue::Unsigned(u64::from(self.horiz_off_n))),
            "HorizOffD" => Ok(FieldValue::Unsigned(u64::from(self.horiz_off_d))),
            "VertOffN" => Ok(FieldValue::Unsigned(u64::from(self.vert_off_n))),
            "VertOffD" => Ok(FieldValue::Unsigned(u64::from(self.vert_off_d))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Clap {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("CleanApertureWidthN", FieldValue::Unsigned(value)) => {
                self.clean_aperture_width_n = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("CleanApertureWidthD", FieldValue::Unsigned(value)) => {
                self.clean_aperture_width_d = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("CleanApertureHeightN", FieldValue::Unsigned(value)) => {
                self.clean_aperture_height_n = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("CleanApertureHeightD", FieldValue::Unsigned(value)) => {
                self.clean_aperture_height_d = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("HorizOffN", FieldValue::Unsigned(value)) => {
                self.horiz_off_n = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("HorizOffD", FieldValue::Unsigned(value)) => {
                self.horiz_off_d = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("VertOffN", FieldValue::Unsigned(value)) => {
                self.vert_off_n = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("VertOffD", FieldValue::Unsigned(value)) => {
                self.vert_off_d = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Clap {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("CleanApertureWidthN", 0, with_bit_width(32)),
        codec_field!("CleanApertureWidthD", 1, with_bit_width(32)),
        codec_field!("CleanApertureHeightN", 2, with_bit_width(32)),
        codec_field!("CleanApertureHeightD", 3, with_bit_width(32)),
        codec_field!("HorizOffN", 4, with_bit_width(32)),
        codec_field!("HorizOffD", 5, with_bit_width(32)),
        codec_field!("VertOffN", 6, with_bit_width(32)),
        codec_field!("VertOffD", 7, with_bit_width(32)),
    ]);
}

/// Content-light-level box carried by some visual sample entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CoLL {
    full_box: FullBoxState,
    pub max_cll: u16,
    pub max_fall: u16,
}

impl FieldHooks for CoLL {}

impl_full_box!(CoLL, *b"CoLL");

impl FieldValueRead for CoLL {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Version" => Ok(FieldValue::Unsigned(u64::from(self.version()))),
            "Flags" => Ok(FieldValue::Unsigned(u64::from(self.flags()))),
            "MaxCLL" => Ok(FieldValue::Unsigned(u64::from(self.max_cll))),
            "MaxFALL" => Ok(FieldValue::Unsigned(u64::from(self.max_fall))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for CoLL {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Version", FieldValue::Unsigned(value)) => {
                self.set_version(u8_from_unsigned(field_name, value)?);
                Ok(())
            }
            ("Flags", FieldValue::Unsigned(value)) => {
                self.set_flags(u32_from_unsigned(field_name, value)?);
                Ok(())
            }
            ("MaxCLL", FieldValue::Unsigned(value)) => {
                self.max_cll = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("MaxFALL", FieldValue::Unsigned(value)) => {
                self.max_fall = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for CoLL {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("MaxCLL", 2, with_bit_width(16)),
        codec_field!("MaxFALL", 3, with_bit_width(16)),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Color information leaf whose active fields depend on the stored colour type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Colr {
    pub colour_type: FourCc,
    pub colour_primaries: u16,
    pub transfer_characteristics: u16,
    pub matrix_coefficients: u16,
    pub full_range_flag: bool,
    pub reserved: u8,
    pub profile: Vec<u8>,
    pub unknown: Vec<u8>,
}

impl Default for Colr {
    fn default() -> Self {
        Self {
            colour_type: FourCc::ANY,
            colour_primaries: 0,
            transfer_characteristics: 0,
            matrix_coefficients: 0,
            full_range_flag: false,
            reserved: 0,
            profile: Vec::new(),
            unknown: Vec::new(),
        }
    }
}

impl FieldHooks for Colr {
    fn field_enabled(&self, name: &'static str) -> Option<bool> {
        match name {
            "ColourPrimaries"
            | "TransferCharacteristics"
            | "MatrixCoefficients"
            | "FullRangeFlag"
            | "Reserved" => Some(self.colour_type == COLR_NCLX),
            "Profile" => Some(matches!(self.colour_type, COLR_RICC | COLR_PROF)),
            "Unknown" => Some(!matches!(
                self.colour_type,
                COLR_NCLX | COLR_RICC | COLR_PROF
            )),
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "ColourType" => Some(quoted_fourcc(self.colour_type)),
            _ => None,
        }
    }
}

impl ImmutableBox for Colr {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"colr")
    }
}

impl MutableBox for Colr {}

impl FieldValueRead for Colr {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "ColourType" => Ok(FieldValue::Bytes(self.colour_type.as_bytes().to_vec())),
            "ColourPrimaries" => Ok(FieldValue::Unsigned(u64::from(self.colour_primaries))),
            "TransferCharacteristics" => Ok(FieldValue::Unsigned(u64::from(
                self.transfer_characteristics,
            ))),
            "MatrixCoefficients" => Ok(FieldValue::Unsigned(u64::from(self.matrix_coefficients))),
            "FullRangeFlag" => Ok(FieldValue::Boolean(self.full_range_flag)),
            "Reserved" => Ok(FieldValue::Unsigned(u64::from(self.reserved))),
            "Profile" => Ok(FieldValue::Bytes(self.profile.clone())),
            "Unknown" => Ok(FieldValue::Bytes(self.unknown.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Colr {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("ColourType", FieldValue::Bytes(bytes)) => {
                self.colour_type = bytes_to_fourcc(field_name, bytes)?;
                Ok(())
            }
            ("ColourPrimaries", FieldValue::Unsigned(value)) => {
                self.colour_primaries = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("TransferCharacteristics", FieldValue::Unsigned(value)) => {
                self.transfer_characteristics = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("MatrixCoefficients", FieldValue::Unsigned(value)) => {
                self.matrix_coefficients = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("FullRangeFlag", FieldValue::Boolean(value)) => {
                self.full_range_flag = value;
                Ok(())
            }
            ("Reserved", FieldValue::Unsigned(value)) => {
                self.reserved = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Profile", FieldValue::Bytes(value)) => {
                self.profile = value;
                Ok(())
            }
            ("Unknown", FieldValue::Bytes(value)) => {
                self.unknown = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Colr {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!(
            "ColourType",
            0,
            with_bit_width(8),
            with_length(4),
            as_bytes()
        ),
        codec_field!(
            "ColourPrimaries",
            1,
            with_bit_width(16),
            with_dynamic_presence()
        ),
        codec_field!(
            "TransferCharacteristics",
            2,
            with_bit_width(16),
            with_dynamic_presence()
        ),
        codec_field!(
            "MatrixCoefficients",
            3,
            with_bit_width(16),
            with_dynamic_presence()
        ),
        codec_field!(
            "FullRangeFlag",
            4,
            with_bit_width(1),
            as_boolean(),
            with_dynamic_presence()
        ),
        codec_field!(
            "Reserved",
            5,
            with_bit_width(7),
            as_hex(),
            with_dynamic_presence()
        ),
        codec_field!(
            "Profile",
            6,
            with_bit_width(8),
            as_bytes(),
            with_dynamic_presence()
        ),
        codec_field!(
            "Unknown",
            7,
            with_bit_width(8),
            as_bytes(),
            with_dynamic_presence()
        ),
    ]);
}

/// Event-message box whose field order changes with the encoded version.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Emsg {
    full_box: FullBoxState,
    pub scheme_id_uri: String,
    pub value: String,
    pub timescale: u32,
    pub presentation_time_delta: u32,
    pub presentation_time: u64,
    pub event_duration: u32,
    pub id: u32,
    pub message_data: Vec<u8>,
}

impl FieldHooks for Emsg {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "MessageData" => Some(quote_bytes(&self.message_data)),
            _ => None,
        }
    }
}

impl_full_box!(Emsg, *b"emsg");

impl FieldValueRead for Emsg {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "SchemeIdUri" => Ok(FieldValue::String(self.scheme_id_uri.clone())),
            "Value" => Ok(FieldValue::String(self.value.clone())),
            "Timescale" => Ok(FieldValue::Unsigned(u64::from(self.timescale))),
            "PresentationTimeDelta" => Ok(FieldValue::Unsigned(u64::from(
                self.presentation_time_delta,
            ))),
            "PresentationTime" => Ok(FieldValue::Unsigned(self.presentation_time)),
            "EventDuration" => Ok(FieldValue::Unsigned(u64::from(self.event_duration))),
            "Id" => Ok(FieldValue::Unsigned(u64::from(self.id))),
            "MessageData" => Ok(FieldValue::Bytes(self.message_data.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Emsg {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("SchemeIdUri", FieldValue::String(value)) => {
                self.scheme_id_uri = value;
                Ok(())
            }
            ("Value", FieldValue::String(value)) => {
                self.value = value;
                Ok(())
            }
            ("Timescale", FieldValue::Unsigned(value)) => {
                self.timescale = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("PresentationTimeDelta", FieldValue::Unsigned(value)) => {
                self.presentation_time_delta = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("PresentationTime", FieldValue::Unsigned(value)) => {
                self.presentation_time = value;
                Ok(())
            }
            ("EventDuration", FieldValue::Unsigned(value)) => {
                self.event_duration = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Id", FieldValue::Unsigned(value)) => {
                self.id = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("MessageData", FieldValue::Bytes(value)) => {
                self.message_data = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Emsg {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!(
            "SchemeIdUri",
            2,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated),
            with_version(0)
        ),
        codec_field!(
            "Value",
            3,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated),
            with_version(0)
        ),
        codec_field!("Timescale", 4, with_bit_width(32)),
        codec_field!(
            "PresentationTimeDelta",
            5,
            with_bit_width(32),
            with_version(0)
        ),
        codec_field!(
            "PresentationTime",
            6,
            with_bit_width(64),
            with_version(1),
            with_display_order(5)
        ),
        codec_field!(
            "EventDuration",
            7,
            with_bit_width(32),
            with_display_order(6)
        ),
        codec_field!("Id", 8, with_bit_width(32), with_display_order(7)),
        codec_field!(
            "SchemeIdUri",
            9,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated),
            with_version(1),
            with_display_order(2)
        ),
        codec_field!(
            "Value",
            10,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated),
            with_version(1),
            with_display_order(3)
        ),
        codec_field!(
            "MessageData",
            11,
            with_bit_width(8),
            as_bytes(),
            with_display_order(8)
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

/// Event-message sample entry carried under `stsd`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventMessageSampleEntry {
    pub sample_entry: SampleEntry,
}

impl Default for EventMessageSampleEntry {
    fn default() -> Self {
        Self {
            sample_entry: SampleEntry {
                box_type: FourCc::from_bytes(*b"evte"),
                data_reference_index: 0,
            },
        }
    }
}

impl FieldHooks for EventMessageSampleEntry {}

impl ImmutableBox for EventMessageSampleEntry {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"evte")
    }
}

impl MutableBox for EventMessageSampleEntry {}

impl FieldValueRead for EventMessageSampleEntry {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DataReferenceIndex" => Ok(FieldValue::Unsigned(u64::from(
                self.sample_entry.data_reference_index,
            ))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for EventMessageSampleEntry {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DataReferenceIndex", FieldValue::Unsigned(value)) => {
                self.sample_entry.data_reference_index = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for EventMessageSampleEntry {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Reserved0A", 0, with_bit_width(16), with_constant("0")),
        codec_field!("Reserved0B", 1, with_bit_width(16), with_constant("0")),
        codec_field!("Reserved0C", 2, with_bit_width(16), with_constant("0")),
        codec_field!("DataReferenceIndex", 3, with_bit_width(16)),
    ]);
}

/// One scheme-identification record carried by [`Silb`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SilbEntry {
    pub scheme_id_uri: String,
    pub value: String,
    pub at_least_one_flag: bool,
}

/// Scheme-identifier box carried by `evte` sample entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Silb {
    full_box: FullBoxState,
    pub scheme_count: u32,
    pub schemes: Vec<SilbEntry>,
    pub other_schemes_flag: bool,
}

fn format_silb_schemes(schemes: &[SilbEntry]) -> String {
    render_array(schemes.iter().map(|scheme| {
        format!(
            "{{SchemeIdUri=\"{}\" Value=\"{}\" AtLeastOneFlag={}}}",
            scheme.scheme_id_uri, scheme.value, scheme.at_least_one_flag
        )
    }))
}

fn encode_silb_schemes(
    field_name: &'static str,
    schemes: &[SilbEntry],
) -> Result<Vec<u8>, FieldValueError> {
    let mut bytes = Vec::new();
    for scheme in schemes {
        validate_c_string_value(field_name, &scheme.scheme_id_uri)?;
        validate_c_string_value(field_name, &scheme.value)?;
        bytes.extend_from_slice(scheme.scheme_id_uri.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(scheme.value.as_bytes());
        bytes.push(0);
        bytes.push(u8::from(scheme.at_least_one_flag));
    }
    Ok(bytes)
}

fn parse_silb_schemes(
    field_name: &'static str,
    scheme_count: u32,
    bytes: &[u8],
) -> Result<Vec<SilbEntry>, FieldValueError> {
    let mut schemes = Vec::with_capacity(untrusted_prealloc_hint(
        usize::try_from(scheme_count).unwrap_or(0),
    ));
    let mut offset = 0usize;

    for _ in 0..scheme_count {
        let (scheme_id_uri, consumed) = parse_required_c_string(field_name, &bytes[offset..])?;
        offset += consumed;

        let (value, consumed) = parse_required_c_string(field_name, &bytes[offset..])?;
        offset += consumed;

        if bytes.len().saturating_sub(offset) < 1 {
            return Err(invalid_value(
                field_name,
                "scheme flag payload is truncated",
            ));
        }

        let at_least_one_flag = match bytes[offset] {
            0 => false,
            1 => true,
            _ => {
                return Err(invalid_value(field_name, "scheme flag byte must be 0 or 1"));
            }
        };
        offset += 1;

        schemes.push(SilbEntry {
            scheme_id_uri,
            value,
            at_least_one_flag,
        });
    }

    if offset != bytes.len() {
        return Err(invalid_value(
            field_name,
            "scheme payload length does not match the scheme count",
        ));
    }

    Ok(schemes)
}

impl FieldHooks for Silb {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Schemes" => Some(format_silb_schemes(&self.schemes)),
            _ => None,
        }
    }
}

impl_full_box!(Silb, *b"silb");

impl FieldValueRead for Silb {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "SchemeCount" => Ok(FieldValue::Unsigned(u64::from(self.scheme_count))),
            "Schemes" => {
                require_count(field_name, self.scheme_count, self.schemes.len())?;
                Ok(FieldValue::Bytes(encode_silb_schemes(
                    field_name,
                    &self.schemes,
                )?))
            }
            "OtherSchemesFlag" => Ok(FieldValue::Boolean(self.other_schemes_flag)),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Silb {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("SchemeCount", FieldValue::Unsigned(value)) => {
                self.scheme_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Schemes", FieldValue::Bytes(bytes)) => {
                self.schemes = parse_silb_schemes(field_name, self.scheme_count, &bytes)?;
                Ok(())
            }
            ("OtherSchemesFlag", FieldValue::Boolean(value)) => {
                self.other_schemes_flag = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Silb {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("SchemeCount", 2, with_bit_width(32)),
        codec_field!("Schemes", 3, with_bit_width(8), as_bytes()),
        codec_field!("OtherSchemesFlag", 4, with_bit_width(8), as_boolean()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];

    fn custom_marshal(&self, writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        if self.version() != 0 {
            return Err(CodecError::UnsupportedVersion {
                box_type: self.box_type(),
                version: self.version(),
            });
        }

        require_count("Schemes", self.scheme_count, self.schemes.len())?;

        let mut payload = Vec::new();
        payload.push(self.version());
        push_uint("Flags", &mut payload, 3, u64::from(self.flags()))?;
        payload.extend_from_slice(&self.scheme_count.to_be_bytes());
        payload.extend_from_slice(&encode_silb_schemes("Schemes", &self.schemes)?);
        payload.push(u8::from(self.other_schemes_flag));
        writer.write_all(&payload)?;
        Ok(Some(payload.len() as u64))
    }

    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, CodecError> {
        let payload_len = usize::try_from(payload_size)
            .map_err(|_| invalid_value("Payload", "payload is too large to decode"))?;
        let payload = read_exact_vec_untrusted(reader, payload_len).map_err(CodecError::Io)?;

        if payload.len() < 9 {
            return Err(invalid_value("Payload", "payload is too short").into());
        }

        let version = payload[0];
        if version != 0 {
            return Err(CodecError::UnsupportedVersion {
                box_type: self.box_type(),
                version,
            });
        }

        let other_schemes_flag = match payload[payload.len() - 1] {
            0 => false,
            1 => true,
            _ => {
                return Err(invalid_value("OtherSchemesFlag", "flag byte must be 0 or 1").into());
            }
        };

        self.full_box = FullBoxState {
            version,
            flags: read_uint(&payload, 1, 3) as u32,
        };
        self.scheme_count = read_u32(&payload, 4);
        self.schemes =
            parse_silb_schemes("Schemes", self.scheme_count, &payload[8..payload.len() - 1])?;
        self.other_schemes_flag = other_schemes_flag;

        Ok(Some(payload_size))
    }
}

/// Embedded event-message instance box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Emib {
    full_box: FullBoxState,
    pub presentation_time_delta: i64,
    pub event_duration: u32,
    pub id: u32,
    pub scheme_id_uri: String,
    pub value: String,
    pub message_data: Vec<u8>,
}

impl FieldHooks for Emib {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "MessageData" => Some(quote_bytes(&self.message_data)),
            _ => None,
        }
    }
}

impl_full_box!(Emib, *b"emib");

impl FieldValueRead for Emib {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "PresentationTimeDelta" => Ok(FieldValue::Signed(self.presentation_time_delta)),
            "EventDuration" => Ok(FieldValue::Unsigned(u64::from(self.event_duration))),
            "Id" => Ok(FieldValue::Unsigned(u64::from(self.id))),
            "SchemeIdUri" => Ok(FieldValue::String(self.scheme_id_uri.clone())),
            "Value" => Ok(FieldValue::String(self.value.clone())),
            "MessageData" => Ok(FieldValue::Bytes(self.message_data.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Emib {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("PresentationTimeDelta", FieldValue::Signed(value)) => {
                self.presentation_time_delta = value;
                Ok(())
            }
            ("EventDuration", FieldValue::Unsigned(value)) => {
                self.event_duration = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Id", FieldValue::Unsigned(value)) => {
                self.id = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("SchemeIdUri", FieldValue::String(value)) => {
                validate_c_string_value(field_name, &value)?;
                self.scheme_id_uri = value;
                Ok(())
            }
            ("Value", FieldValue::String(value)) => {
                validate_c_string_value(field_name, &value)?;
                self.value = value;
                Ok(())
            }
            ("MessageData", FieldValue::Bytes(value)) => {
                self.message_data = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Emib {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("PresentationTimeDelta", 2, with_bit_width(64), as_signed()),
        codec_field!("EventDuration", 3, with_bit_width(32)),
        codec_field!("Id", 4, with_bit_width(32)),
        codec_field!(
            "SchemeIdUri",
            5,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated)
        ),
        codec_field!(
            "Value",
            6,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated)
        ),
        codec_field!("MessageData", 7, with_bit_width(8), as_bytes()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];

    fn custom_marshal(&self, writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        validate_c_string_value("SchemeIdUri", &self.scheme_id_uri)?;
        validate_c_string_value("Value", &self.value)?;
        if self.version() != 0 {
            return Err(CodecError::UnsupportedVersion {
                box_type: self.box_type(),
                version: self.version(),
            });
        }

        let mut payload = Vec::with_capacity(
            24 + self.scheme_id_uri.len() + self.value.len() + self.message_data.len() + 2,
        );
        payload.push(self.version());
        push_uint("Flags", &mut payload, 3, u64::from(self.flags()))?;
        payload.extend_from_slice(&0_u32.to_be_bytes());
        payload.extend_from_slice(&self.presentation_time_delta.to_be_bytes());
        payload.extend_from_slice(&self.event_duration.to_be_bytes());
        payload.extend_from_slice(&self.id.to_be_bytes());
        payload.extend_from_slice(self.scheme_id_uri.as_bytes());
        payload.push(0);
        payload.extend_from_slice(self.value.as_bytes());
        payload.push(0);
        payload.extend_from_slice(&self.message_data);
        writer.write_all(&payload)?;
        Ok(Some(payload.len() as u64))
    }

    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, CodecError> {
        let payload_len = usize::try_from(payload_size)
            .map_err(|_| invalid_value("Payload", "payload is too large to decode"))?;
        let payload = read_exact_vec_untrusted(reader, payload_len).map_err(CodecError::Io)?;

        if payload.len() < 24 {
            return Err(invalid_value("Payload", "payload is too short").into());
        }

        let version = payload[0];
        if version != 0 {
            return Err(CodecError::UnsupportedVersion {
                box_type: self.box_type(),
                version,
            });
        }

        if read_u32(&payload, 4) != 0 {
            return Err(invalid_value("Reserved", "reserved field must be zero").into());
        }

        let (scheme_id_uri, scheme_len) = decode_required_c_string("SchemeIdUri", &payload[24..])?;
        let value_offset = 24 + scheme_len;
        let (value, value_len) = decode_required_c_string("Value", &payload[value_offset..])?;
        let message_offset = value_offset + value_len;

        self.full_box = FullBoxState {
            version,
            flags: read_uint(&payload, 1, 3) as u32,
        };
        self.presentation_time_delta = read_i64(&payload, 8);
        self.event_duration = read_u32(&payload, 16);
        self.id = read_u32(&payload, 20);
        self.scheme_id_uri = scheme_id_uri;
        self.value = value;
        self.message_data = payload[message_offset..].to_vec();

        Ok(Some(payload_size))
    }
}

/// Empty embedded event-message box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Emeb;

impl FieldHooks for Emeb {}

impl ImmutableBox for Emeb {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"emeb")
    }
}

impl MutableBox for Emeb {}

impl FieldValueRead for Emeb {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        Err(missing_field(field_name))
    }
}

impl FieldValueWrite for Emeb {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        Err(unexpected_field(field_name, value))
    }
}

impl CodecBox for Emeb {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[]);

    fn custom_marshal(&self, _writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        Ok(Some(0))
    }

    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, CodecError> {
        let start = reader.stream_position()?;
        if payload_size != 0 {
            reader.seek(SeekFrom::Start(start))?;
            return Err(invalid_value("Payload", "payload must be empty").into());
        }
        Ok(Some(0))
    }
}

/// Field-ordering leaf used by some video sample entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fiel {
    pub field_count: u8,
    pub field_ordering: u8,
}

impl_leaf_box!(Fiel, *b"fiel");

impl FieldValueRead for Fiel {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "FieldCount" => Ok(FieldValue::Unsigned(u64::from(self.field_count))),
            "FieldOrdering" => Ok(FieldValue::Unsigned(u64::from(self.field_ordering))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Fiel {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("FieldCount", FieldValue::Unsigned(value)) => {
                self.field_count = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("FieldOrdering", FieldValue::Unsigned(value)) => {
                self.field_ordering = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Fiel {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("FieldCount", 0, with_bit_width(8), as_hex()),
        codec_field!("FieldOrdering", 1, with_bit_width(8), as_hex()),
    ]);
}

/// Original-format indicator inside protection-scheme sample-entry paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frma {
    pub data_format: FourCc,
}

impl Default for Frma {
    fn default() -> Self {
        Self {
            data_format: FourCc::ANY,
        }
    }
}

impl FieldHooks for Frma {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "DataFormat" => Some(quoted_fourcc(self.data_format)),
            _ => None,
        }
    }
}

impl ImmutableBox for Frma {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"frma")
    }
}

impl MutableBox for Frma {}

impl FieldValueRead for Frma {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DataFormat" => Ok(FieldValue::Bytes(self.data_format.as_bytes().to_vec())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Frma {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DataFormat", FieldValue::Bytes(bytes)) => {
                self.data_format = bytes_to_fourcc(field_name, bytes)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Frma {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[codec_field!(
        "DataFormat",
        0,
        with_bit_width(8),
        with_length(4),
        as_bytes()
    )]);
}

/// Pixel-aspect-ratio box carried by visual sample entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pasp {
    pub h_spacing: u32,
    pub v_spacing: u32,
}

impl_leaf_box!(Pasp, *b"pasp");

impl FieldValueRead for Pasp {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "HSpacing" => Ok(FieldValue::Unsigned(u64::from(self.h_spacing))),
            "VSpacing" => Ok(FieldValue::Unsigned(u64::from(self.v_spacing))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Pasp {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("HSpacing", FieldValue::Unsigned(value)) => {
                self.h_spacing = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("VSpacing", FieldValue::Unsigned(value)) => {
                self.v_spacing = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Pasp {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("HSpacing", 0, with_bit_width(32)),
        codec_field!("VSpacing", 1, with_bit_width(32)),
    ]);
}

/// Mastering-display metadata box carried by some visual sample entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SmDm {
    full_box: FullBoxState,
    pub primary_r_chromaticity_x: u16,
    pub primary_r_chromaticity_y: u16,
    pub primary_g_chromaticity_x: u16,
    pub primary_g_chromaticity_y: u16,
    pub primary_b_chromaticity_x: u16,
    pub primary_b_chromaticity_y: u16,
    pub white_point_chromaticity_x: u16,
    pub white_point_chromaticity_y: u16,
    pub luminance_max: u32,
    pub luminance_min: u32,
}

impl FieldHooks for SmDm {}

impl_full_box!(SmDm, *b"SmDm");

impl FieldValueRead for SmDm {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Version" => Ok(FieldValue::Unsigned(u64::from(self.version()))),
            "Flags" => Ok(FieldValue::Unsigned(u64::from(self.flags()))),
            "PrimaryRChromaticityX" => Ok(FieldValue::Unsigned(u64::from(
                self.primary_r_chromaticity_x,
            ))),
            "PrimaryRChromaticityY" => Ok(FieldValue::Unsigned(u64::from(
                self.primary_r_chromaticity_y,
            ))),
            "PrimaryGChromaticityX" => Ok(FieldValue::Unsigned(u64::from(
                self.primary_g_chromaticity_x,
            ))),
            "PrimaryGChromaticityY" => Ok(FieldValue::Unsigned(u64::from(
                self.primary_g_chromaticity_y,
            ))),
            "PrimaryBChromaticityX" => Ok(FieldValue::Unsigned(u64::from(
                self.primary_b_chromaticity_x,
            ))),
            "PrimaryBChromaticityY" => Ok(FieldValue::Unsigned(u64::from(
                self.primary_b_chromaticity_y,
            ))),
            "WhitePointChromaticityX" => Ok(FieldValue::Unsigned(u64::from(
                self.white_point_chromaticity_x,
            ))),
            "WhitePointChromaticityY" => Ok(FieldValue::Unsigned(u64::from(
                self.white_point_chromaticity_y,
            ))),
            "LuminanceMax" => Ok(FieldValue::Unsigned(u64::from(self.luminance_max))),
            "LuminanceMin" => Ok(FieldValue::Unsigned(u64::from(self.luminance_min))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for SmDm {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Version", FieldValue::Unsigned(value)) => {
                self.set_version(u8_from_unsigned(field_name, value)?);
                Ok(())
            }
            ("Flags", FieldValue::Unsigned(value)) => {
                self.set_flags(u32_from_unsigned(field_name, value)?);
                Ok(())
            }
            ("PrimaryRChromaticityX", FieldValue::Unsigned(value)) => {
                self.primary_r_chromaticity_x = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("PrimaryRChromaticityY", FieldValue::Unsigned(value)) => {
                self.primary_r_chromaticity_y = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("PrimaryGChromaticityX", FieldValue::Unsigned(value)) => {
                self.primary_g_chromaticity_x = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("PrimaryGChromaticityY", FieldValue::Unsigned(value)) => {
                self.primary_g_chromaticity_y = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("PrimaryBChromaticityX", FieldValue::Unsigned(value)) => {
                self.primary_b_chromaticity_x = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("PrimaryBChromaticityY", FieldValue::Unsigned(value)) => {
                self.primary_b_chromaticity_y = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("WhitePointChromaticityX", FieldValue::Unsigned(value)) => {
                self.white_point_chromaticity_x = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("WhitePointChromaticityY", FieldValue::Unsigned(value)) => {
                self.white_point_chromaticity_y = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("LuminanceMax", FieldValue::Unsigned(value)) => {
                self.luminance_max = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("LuminanceMin", FieldValue::Unsigned(value)) => {
                self.luminance_min = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for SmDm {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("PrimaryRChromaticityX", 2, with_bit_width(16)),
        codec_field!("PrimaryRChromaticityY", 3, with_bit_width(16)),
        codec_field!("PrimaryGChromaticityX", 4, with_bit_width(16)),
        codec_field!("PrimaryGChromaticityY", 5, with_bit_width(16)),
        codec_field!("PrimaryBChromaticityX", 6, with_bit_width(16)),
        codec_field!("PrimaryBChromaticityY", 7, with_bit_width(16)),
        codec_field!("WhitePointChromaticityX", 8, with_bit_width(16)),
        codec_field!("WhitePointChromaticityY", 9, with_bit_width(16)),
        codec_field!("LuminanceMax", 10, with_bit_width(32)),
        codec_field!("LuminanceMin", 11, with_bit_width(32)),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Scheme-type declaration box inside a protection-scheme path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Schm {
    full_box: FullBoxState,
    pub scheme_type: FourCc,
    pub scheme_version: u32,
    pub scheme_uri: String,
}

impl Default for Schm {
    fn default() -> Self {
        Self {
            full_box: FullBoxState::default(),
            scheme_type: FourCc::ANY,
            scheme_version: 0,
            scheme_uri: String::new(),
        }
    }
}

impl FieldHooks for Schm {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "SchemeType" => Some(quoted_fourcc(self.scheme_type)),
            _ => None,
        }
    }
}

impl_full_box!(Schm, *b"schm");

impl FieldValueRead for Schm {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "SchemeType" => Ok(FieldValue::Bytes(self.scheme_type.as_bytes().to_vec())),
            "SchemeVersion" => Ok(FieldValue::Unsigned(u64::from(self.scheme_version))),
            "SchemeUri" => Ok(FieldValue::String(self.scheme_uri.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Schm {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("SchemeType", FieldValue::Bytes(bytes)) => {
                self.scheme_type = bytes_to_fourcc(field_name, bytes)?;
                Ok(())
            }
            ("SchemeVersion", FieldValue::Unsigned(value)) => {
                self.scheme_version = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("SchemeUri", FieldValue::String(value)) => {
                self.scheme_uri = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Schm {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!(
            "SchemeType",
            2,
            with_bit_width(8),
            with_length(4),
            as_bytes()
        ),
        codec_field!("SchemeVersion", 3, with_bit_width(32), as_hex()),
        codec_field!(
            "SchemeUri",
            4,
            with_bit_width(8),
            as_string(StringFieldMode::RawBox),
            with_required_flags(SCHEME_URI_PRESENT)
        ),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// Shared header fields carried by concrete sample-entry wrappers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SampleEntry {
    pub box_type: FourCc,
    pub data_reference_index: u16,
}

impl Default for SampleEntry {
    fn default() -> Self {
        Self {
            box_type: FourCc::ANY,
            data_reference_index: 0,
        }
    }
}

/// Visual sample-entry wrapper used by multiple codec-specific visual types.
///
/// Child boxes remain outside this typed header model and are still traversed through the normal
/// structure-walking APIs. Some files also carry opaque bytes after the last valid child box; the
/// box walker and rewrite helpers preserve those layouts instead of failing late while descending
/// into the trailing payload.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VisualSampleEntry {
    pub sample_entry: SampleEntry,
    pub pre_defined: u16,
    pub pre_defined2: [u32; 3],
    pub width: u16,
    pub height: u16,
    pub horizresolution: u32,
    pub vertresolution: u32,
    pub reserved2: u32,
    pub frame_count: u16,
    pub compressorname: [u8; 32],
    pub depth: u16,
    pub pre_defined3: i16,
}

impl FieldHooks for VisualSampleEntry {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Compressorname" if self.compressorname[0] <= 31 => {
                let visible_len = usize::from(self.compressorname[0]).min(31);
                Some(quote_bytes(&self.compressorname[1..1 + visible_len]))
            }
            _ => None,
        }
    }
}

impl ImmutableBox for VisualSampleEntry {
    fn box_type(&self) -> FourCc {
        self.sample_entry.box_type
    }
}

impl MutableBox for VisualSampleEntry {}

impl AnyTypeBox for VisualSampleEntry {
    fn set_box_type(&mut self, box_type: FourCc) {
        self.sample_entry.box_type = box_type;
    }
}

impl FieldValueRead for VisualSampleEntry {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DataReferenceIndex" => Ok(FieldValue::Unsigned(u64::from(
                self.sample_entry.data_reference_index,
            ))),
            "PreDefined" => Ok(FieldValue::Unsigned(u64::from(self.pre_defined))),
            "PreDefined2" => Ok(FieldValue::UnsignedArray(
                self.pre_defined2.iter().copied().map(u64::from).collect(),
            )),
            "Width" => Ok(FieldValue::Unsigned(u64::from(self.width))),
            "Height" => Ok(FieldValue::Unsigned(u64::from(self.height))),
            "Horizresolution" => Ok(FieldValue::Unsigned(u64::from(self.horizresolution))),
            "Vertresolution" => Ok(FieldValue::Unsigned(u64::from(self.vertresolution))),
            "Reserved2" => Ok(FieldValue::Unsigned(u64::from(self.reserved2))),
            "FrameCount" => Ok(FieldValue::Unsigned(u64::from(self.frame_count))),
            "Compressorname" => Ok(FieldValue::Bytes(self.compressorname.to_vec())),
            "Depth" => Ok(FieldValue::Unsigned(u64::from(self.depth))),
            "PreDefined3" => Ok(FieldValue::Signed(i64::from(self.pre_defined3))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for VisualSampleEntry {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DataReferenceIndex", FieldValue::Unsigned(value)) => {
                self.sample_entry.data_reference_index = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("PreDefined", FieldValue::Unsigned(value)) => {
                self.pre_defined = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("PreDefined2", FieldValue::UnsignedArray(values)) => {
                if values.len() != 3 {
                    return Err(invalid_value(
                        field_name,
                        "value must contain exactly 3 entries",
                    ));
                }
                self.pre_defined2 = [
                    u32_from_unsigned(field_name, values[0])?,
                    u32_from_unsigned(field_name, values[1])?,
                    u32_from_unsigned(field_name, values[2])?,
                ];
                Ok(())
            }
            ("Width", FieldValue::Unsigned(value)) => {
                self.width = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Height", FieldValue::Unsigned(value)) => {
                self.height = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Horizresolution", FieldValue::Unsigned(value)) => {
                self.horizresolution = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Vertresolution", FieldValue::Unsigned(value)) => {
                self.vertresolution = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Reserved2", FieldValue::Unsigned(value)) => {
                self.reserved2 = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("FrameCount", FieldValue::Unsigned(value)) => {
                self.frame_count = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Compressorname", FieldValue::Bytes(bytes)) => {
                self.compressorname = bytes
                    .try_into()
                    .map_err(|_| invalid_value(field_name, "value must be exactly 32 bytes"))?;
                Ok(())
            }
            ("Depth", FieldValue::Unsigned(value)) => {
                self.depth = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("PreDefined3", FieldValue::Signed(value)) => {
                self.pre_defined3 = i16_from_signed(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for VisualSampleEntry {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Reserved0A", 0, with_bit_width(16), with_constant("0")),
        codec_field!("Reserved0B", 1, with_bit_width(16), with_constant("0")),
        codec_field!("Reserved0C", 2, with_bit_width(16), with_constant("0")),
        codec_field!("DataReferenceIndex", 3, with_bit_width(16)),
        codec_field!("PreDefined", 4, with_bit_width(16)),
        codec_field!("Reserved1", 5, with_bit_width(16), with_constant("0")),
        codec_field!("PreDefined2", 6, with_bit_width(32), with_length(3)),
        codec_field!("Width", 7, with_bit_width(16)),
        codec_field!("Height", 8, with_bit_width(16)),
        codec_field!("Horizresolution", 9, with_bit_width(32)),
        codec_field!("Vertresolution", 10, with_bit_width(32)),
        codec_field!("Reserved2", 11, with_bit_width(32), as_hidden()),
        codec_field!("FrameCount", 12, with_bit_width(16)),
        codec_field!(
            "Compressorname",
            13,
            with_bit_width(8),
            with_length(32),
            as_bytes()
        ),
        codec_field!("Depth", 14, with_bit_width(16)),
        codec_field!("PreDefined3", 15, with_bit_width(16), as_signed()),
    ]);

    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, CodecError> {
        const VISUAL_SAMPLE_ENTRY_HEADER_SIZE: usize = 78;

        let start = reader.stream_position()?;
        let payload_len = usize::try_from(payload_size)
            .map_err(|_| invalid_value("Payload", "payload is too large to decode"))?;
        let payload = read_exact_vec_untrusted(reader, payload_len).map_err(CodecError::Io)?;
        if payload.len() < VISUAL_SAMPLE_ENTRY_HEADER_SIZE {
            return Err(invalid_value("Payload", "payload is too short").into());
        }

        if read_u16(&payload, 0) != 0 {
            return Err(CodecError::ConstantMismatch {
                field_name: "Reserved0A",
                constant: "0",
            });
        }
        if read_u16(&payload, 2) != 0 {
            return Err(CodecError::ConstantMismatch {
                field_name: "Reserved0B",
                constant: "0",
            });
        }
        if read_u16(&payload, 4) != 0 {
            return Err(CodecError::ConstantMismatch {
                field_name: "Reserved0C",
                constant: "0",
            });
        }
        if read_u16(&payload, 10) != 0 {
            return Err(CodecError::ConstantMismatch {
                field_name: "Reserved1",
                constant: "0",
            });
        }

        self.sample_entry.data_reference_index = read_u16(&payload, 6);
        self.pre_defined = read_u16(&payload, 8);
        self.pre_defined2 = [
            read_u32(&payload, 12),
            read_u32(&payload, 16),
            read_u32(&payload, 20),
        ];
        self.width = read_u16(&payload, 24);
        self.height = read_u16(&payload, 26);
        self.horizresolution = read_u32(&payload, 28);
        self.vertresolution = read_u32(&payload, 32);
        self.reserved2 = read_u32(&payload, 36);
        self.frame_count = read_u16(&payload, 40);
        self.compressorname = payload[42..74].try_into().unwrap();
        self.depth = read_u16(&payload, 74);
        self.pre_defined3 = read_i16(&payload, 76);

        reader.seek(SeekFrom::Start(
            start + VISUAL_SAMPLE_ENTRY_HEADER_SIZE as u64,
        ))?;
        Ok(Some(VISUAL_SAMPLE_ENTRY_HEADER_SIZE as u64))
    }
}

pub(crate) fn split_box_children_with_optional_trailing_bytes(bytes: &[u8]) -> usize {
    let mut cursor = Cursor::new(bytes);
    let mut child_payload_len = 0usize;

    loop {
        let start = cursor.position();
        let remaining = (bytes.len() as u64).saturating_sub(start);
        if remaining < SMALL_HEADER_SIZE {
            break;
        }

        let info = match BoxInfo::read(&mut cursor) {
            Ok(info) => info,
            Err(_) => break,
        };
        if info.extend_to_eof() || info.size() < info.header_size() || info.size() > remaining {
            break;
        }

        cursor.set_position(start + info.size());
        child_payload_len = cursor.position() as usize;
    }

    child_payload_len
}

/// Audio sample-entry wrapper used by multiple codec-specific audio types.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioSampleEntry {
    pub sample_entry: SampleEntry,
    pub entry_version: u16,
    pub channel_count: u16,
    pub sample_size: u16,
    pub pre_defined: u16,
    pub sample_rate: u32,
    pub quicktime_data: Vec<u8>,
}

impl FieldHooks for AudioSampleEntry {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "QuickTimeData" => match self.entry_version {
                1 => Some(16),
                2 => Some(36),
                _ => None,
            },
            _ => None,
        }
    }

    fn field_enabled(&self, name: &'static str) -> Option<bool> {
        match name {
            "QuickTimeData" => Some(matches!(self.entry_version, 1 | 2)),
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "SampleRate" => Some(format_fixed_16_16_unsigned(self.sample_rate)),
            _ => None,
        }
    }
}

impl ImmutableBox for AudioSampleEntry {
    fn box_type(&self) -> FourCc {
        self.sample_entry.box_type
    }
}

impl MutableBox for AudioSampleEntry {}

impl AnyTypeBox for AudioSampleEntry {
    fn set_box_type(&mut self, box_type: FourCc) {
        self.sample_entry.box_type = box_type;
    }
}

impl AudioSampleEntry {
    /// Returns the 16.16 fixed-point sample rate as a floating-point value.
    pub fn sample_rate_value(&self) -> f64 {
        f64::from(self.sample_rate) / 65536.0
    }

    /// Returns the integer component of the 16.16 fixed-point sample rate.
    pub fn sample_rate_int(&self) -> u16 {
        (self.sample_rate >> 16) as u16
    }
}

impl FieldValueRead for AudioSampleEntry {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DataReferenceIndex" => Ok(FieldValue::Unsigned(u64::from(
                self.sample_entry.data_reference_index,
            ))),
            "EntryVersion" => Ok(FieldValue::Unsigned(u64::from(self.entry_version))),
            "ChannelCount" => Ok(FieldValue::Unsigned(u64::from(self.channel_count))),
            "SampleSize" => Ok(FieldValue::Unsigned(u64::from(self.sample_size))),
            "PreDefined" => Ok(FieldValue::Unsigned(u64::from(self.pre_defined))),
            "SampleRate" => Ok(FieldValue::Unsigned(u64::from(self.sample_rate))),
            "QuickTimeData" => Ok(FieldValue::Bytes(self.quicktime_data.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for AudioSampleEntry {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DataReferenceIndex", FieldValue::Unsigned(value)) => {
                self.sample_entry.data_reference_index = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("EntryVersion", FieldValue::Unsigned(value)) => {
                self.entry_version = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ChannelCount", FieldValue::Unsigned(value)) => {
                self.channel_count = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("SampleSize", FieldValue::Unsigned(value)) => {
                self.sample_size = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("PreDefined", FieldValue::Unsigned(value)) => {
                self.pre_defined = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("SampleRate", FieldValue::Unsigned(value)) => {
                self.sample_rate = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("QuickTimeData", FieldValue::Bytes(value)) => {
                self.quicktime_data = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for AudioSampleEntry {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Reserved0A", 0, with_bit_width(16), with_constant("0")),
        codec_field!("Reserved0B", 1, with_bit_width(16), with_constant("0")),
        codec_field!("Reserved0C", 2, with_bit_width(16), with_constant("0")),
        codec_field!("DataReferenceIndex", 3, with_bit_width(16)),
        codec_field!("EntryVersion", 4, with_bit_width(16)),
        codec_field!("Reserved1A", 5, with_bit_width(16), with_constant("0")),
        codec_field!("Reserved1B", 6, with_bit_width(16), with_constant("0")),
        codec_field!("Reserved1C", 7, with_bit_width(16), with_constant("0")),
        codec_field!("ChannelCount", 8, with_bit_width(16)),
        codec_field!("SampleSize", 9, with_bit_width(16)),
        codec_field!("PreDefined", 10, with_bit_width(16)),
        codec_field!("Reserved2", 11, with_bit_width(16), with_constant("0")),
        codec_field!("SampleRate", 12, with_bit_width(32)),
        codec_field!(
            "QuickTimeData",
            13,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes(),
            with_dynamic_presence()
        ),
    ]);
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WaveAudioData {
    box_type: FourCc,
    quicktime_data: Vec<u8>,
}

impl Default for WaveAudioData {
    fn default() -> Self {
        Self {
            box_type: FourCc::ANY,
            quicktime_data: Vec::new(),
        }
    }
}

impl FieldHooks for WaveAudioData {}

impl ImmutableBox for WaveAudioData {
    fn box_type(&self) -> FourCc {
        self.box_type
    }
}

impl MutableBox for WaveAudioData {}

impl AnyTypeBox for WaveAudioData {
    fn set_box_type(&mut self, box_type: FourCc) {
        self.box_type = box_type;
    }
}

impl FieldValueRead for WaveAudioData {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "QuickTimeData" => Ok(FieldValue::Bytes(self.quicktime_data.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for WaveAudioData {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("QuickTimeData", FieldValue::Bytes(value)) => {
                self.quicktime_data = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for WaveAudioData {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[codec_field!(
        "QuickTimeData",
        0,
        with_bit_width(8),
        as_bytes()
    )]);
}

/// One length-prefixed AVC parameter-set record carried by `avcC`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AVCParameterSet {
    pub length: u16,
    pub nal_unit: Vec<u8>,
}

/// AVC decoder configuration carried by visual sample entries such as `avc1`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AVCDecoderConfiguration {
    pub configuration_version: u8,
    pub profile: u8,
    pub profile_compatibility: u8,
    pub level: u8,
    pub length_size_minus_one: u8,
    pub num_of_sequence_parameter_sets: u8,
    pub sequence_parameter_sets: Vec<AVCParameterSet>,
    pub num_of_picture_parameter_sets: u8,
    pub picture_parameter_sets: Vec<AVCParameterSet>,
    pub high_profile_fields_enabled: bool,
    pub chroma_format: u8,
    pub bit_depth_luma_minus8: u8,
    pub bit_depth_chroma_minus8: u8,
    pub num_of_sequence_parameter_set_ext: u8,
    pub sequence_parameter_sets_ext: Vec<AVCParameterSet>,
}

impl FieldHooks for AVCDecoderConfiguration {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "SequenceParameterSets" => {
                encoded_avc_parameter_sets_len(name, &self.sequence_parameter_sets).ok()
            }
            "PictureParameterSets" => {
                encoded_avc_parameter_sets_len(name, &self.picture_parameter_sets).ok()
            }
            "SequenceParameterSetsExt" => {
                encoded_avc_parameter_sets_len(name, &self.sequence_parameter_sets_ext).ok()
            }
            _ => None,
        }
    }

    fn field_enabled(&self, name: &'static str) -> Option<bool> {
        match name {
            "ChromaFormat"
            | "BitDepthLumaMinus8"
            | "BitDepthChromaMinus8"
            | "NumOfSequenceParameterSetExt"
            | "SequenceParameterSetsExt" => Some(self.high_profile_fields_enabled),
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "SequenceParameterSets" => {
                Some(render_avc_parameter_sets(&self.sequence_parameter_sets))
            }
            "PictureParameterSets" => Some(render_avc_parameter_sets(&self.picture_parameter_sets)),
            "SequenceParameterSetsExt" => {
                Some(render_avc_parameter_sets(&self.sequence_parameter_sets_ext))
            }
            _ => None,
        }
    }
}

impl ImmutableBox for AVCDecoderConfiguration {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"avcC")
    }
}

impl MutableBox for AVCDecoderConfiguration {}

impl FieldValueRead for AVCDecoderConfiguration {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "ConfigurationVersion" => {
                Ok(FieldValue::Unsigned(u64::from(self.configuration_version)))
            }
            "Profile" => Ok(FieldValue::Unsigned(u64::from(self.profile))),
            "ProfileCompatibility" => {
                Ok(FieldValue::Unsigned(u64::from(self.profile_compatibility)))
            }
            "Level" => Ok(FieldValue::Unsigned(u64::from(self.level))),
            "LengthSizeMinusOne" => Ok(FieldValue::Unsigned(u64::from(self.length_size_minus_one))),
            "NumOfSequenceParameterSets" => Ok(FieldValue::Unsigned(u64::from(
                self.num_of_sequence_parameter_sets,
            ))),
            "SequenceParameterSets" => Ok(FieldValue::Bytes(encode_avc_parameter_sets(
                field_name,
                &self.sequence_parameter_sets,
            )?)),
            "NumOfPictureParameterSets" => Ok(FieldValue::Unsigned(u64::from(
                self.num_of_picture_parameter_sets,
            ))),
            "PictureParameterSets" => Ok(FieldValue::Bytes(encode_avc_parameter_sets(
                field_name,
                &self.picture_parameter_sets,
            )?)),
            "ChromaFormat" => Ok(FieldValue::Unsigned(u64::from(self.chroma_format))),
            "BitDepthLumaMinus8" => Ok(FieldValue::Unsigned(u64::from(self.bit_depth_luma_minus8))),
            "BitDepthChromaMinus8" => Ok(FieldValue::Unsigned(u64::from(
                self.bit_depth_chroma_minus8,
            ))),
            "NumOfSequenceParameterSetExt" => Ok(FieldValue::Unsigned(u64::from(
                self.num_of_sequence_parameter_set_ext,
            ))),
            "SequenceParameterSetsExt" => Ok(FieldValue::Bytes(encode_avc_parameter_sets(
                field_name,
                &self.sequence_parameter_sets_ext,
            )?)),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for AVCDecoderConfiguration {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("ConfigurationVersion", FieldValue::Unsigned(value)) => {
                self.configuration_version = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Profile", FieldValue::Unsigned(value)) => {
                self.profile = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ProfileCompatibility", FieldValue::Unsigned(value)) => {
                self.profile_compatibility = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Level", FieldValue::Unsigned(value)) => {
                self.level = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("LengthSizeMinusOne", FieldValue::Unsigned(value)) => {
                self.length_size_minus_one = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("NumOfSequenceParameterSets", FieldValue::Unsigned(value)) => {
                self.num_of_sequence_parameter_sets = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("SequenceParameterSets", FieldValue::Bytes(value)) => {
                self.sequence_parameter_sets = parse_avc_parameter_sets(
                    field_name,
                    &value,
                    self.num_of_sequence_parameter_sets,
                )?;
                Ok(())
            }
            ("NumOfPictureParameterSets", FieldValue::Unsigned(value)) => {
                self.num_of_picture_parameter_sets = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("PictureParameterSets", FieldValue::Bytes(value)) => {
                self.picture_parameter_sets = parse_avc_parameter_sets(
                    field_name,
                    &value,
                    self.num_of_picture_parameter_sets,
                )?;
                Ok(())
            }
            ("ChromaFormat", FieldValue::Unsigned(value)) => {
                self.chroma_format = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("BitDepthLumaMinus8", FieldValue::Unsigned(value)) => {
                self.bit_depth_luma_minus8 = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("BitDepthChromaMinus8", FieldValue::Unsigned(value)) => {
                self.bit_depth_chroma_minus8 = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("NumOfSequenceParameterSetExt", FieldValue::Unsigned(value)) => {
                self.num_of_sequence_parameter_set_ext = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("SequenceParameterSetsExt", FieldValue::Bytes(value)) => {
                self.sequence_parameter_sets_ext = parse_avc_parameter_sets(
                    field_name,
                    &value,
                    self.num_of_sequence_parameter_set_ext,
                )?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for AVCDecoderConfiguration {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("ConfigurationVersion", 0, with_bit_width(8), as_hex()),
        codec_field!("Profile", 1, with_bit_width(8), as_hex()),
        codec_field!("ProfileCompatibility", 2, with_bit_width(8), as_hex()),
        codec_field!("Level", 3, with_bit_width(8), as_hex()),
        codec_field!("LengthSizeMinusOne", 4, with_bit_width(8), as_hex()),
        codec_field!("NumOfSequenceParameterSets", 5, with_bit_width(8), as_hex()),
        codec_field!(
            "SequenceParameterSets",
            6,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes()
        ),
        codec_field!("NumOfPictureParameterSets", 7, with_bit_width(8), as_hex()),
        codec_field!(
            "PictureParameterSets",
            8,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes()
        ),
        codec_field!(
            "ChromaFormat",
            9,
            with_bit_width(8),
            as_hex(),
            with_dynamic_presence()
        ),
        codec_field!(
            "BitDepthLumaMinus8",
            10,
            with_bit_width(8),
            as_hex(),
            with_dynamic_presence()
        ),
        codec_field!(
            "BitDepthChromaMinus8",
            11,
            with_bit_width(8),
            as_hex(),
            with_dynamic_presence()
        ),
        codec_field!(
            "NumOfSequenceParameterSetExt",
            12,
            with_bit_width(8),
            as_hex(),
            with_dynamic_presence()
        ),
        codec_field!(
            "SequenceParameterSetsExt",
            13,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes(),
            with_dynamic_presence()
        ),
    ]);

    fn custom_marshal(&self, writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        if self.length_size_minus_one > 0x03 {
            return Err(invalid_value("LengthSizeMinusOne", "value does not fit in 2 bits").into());
        }
        if self.num_of_sequence_parameter_sets > 0x1f {
            return Err(invalid_value(
                "NumOfSequenceParameterSets",
                "value does not fit in 5 bits",
            )
            .into());
        }

        require_count(
            "NumOfSequenceParameterSets",
            u32::from(self.num_of_sequence_parameter_sets),
            self.sequence_parameter_sets.len(),
        )?;
        require_count(
            "NumOfPictureParameterSets",
            u32::from(self.num_of_picture_parameter_sets),
            self.picture_parameter_sets.len(),
        )?;

        let mut payload = vec![
            self.configuration_version,
            self.profile,
            self.profile_compatibility,
            self.level,
            0xfc | self.length_size_minus_one,
            0xe0 | self.num_of_sequence_parameter_sets,
        ];
        payload.extend_from_slice(&encode_avc_parameter_sets(
            "SequenceParameterSets",
            &self.sequence_parameter_sets,
        )?);
        payload.push(self.num_of_picture_parameter_sets);
        payload.extend_from_slice(&encode_avc_parameter_sets(
            "PictureParameterSets",
            &self.picture_parameter_sets,
        )?);

        if self.high_profile_fields_enabled {
            if !avc_profile_supports_extensions(self.profile) {
                return Err(invalid_value(
                    "HighProfileFieldsEnabled",
                    "each values of Profile and HighProfileFieldsEnabled are inconsistent",
                )
                .into());
            }
            if self.chroma_format > 0x03 {
                return Err(invalid_value("ChromaFormat", "value does not fit in 2 bits").into());
            }
            if self.bit_depth_luma_minus8 > 0x07 {
                return Err(
                    invalid_value("BitDepthLumaMinus8", "value does not fit in 3 bits").into(),
                );
            }
            if self.bit_depth_chroma_minus8 > 0x07 {
                return Err(
                    invalid_value("BitDepthChromaMinus8", "value does not fit in 3 bits").into(),
                );
            }
            require_count(
                "NumOfSequenceParameterSetExt",
                u32::from(self.num_of_sequence_parameter_set_ext),
                self.sequence_parameter_sets_ext.len(),
            )?;

            payload.push(0xfc | self.chroma_format);
            payload.push(0xf8 | self.bit_depth_luma_minus8);
            payload.push(0xf8 | self.bit_depth_chroma_minus8);
            payload.push(self.num_of_sequence_parameter_set_ext);
            payload.extend_from_slice(&encode_avc_parameter_sets(
                "SequenceParameterSetsExt",
                &self.sequence_parameter_sets_ext,
            )?);
        }

        writer.write_all(&payload)?;
        Ok(Some(payload.len() as u64))
    }

    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, CodecError> {
        let start = reader.stream_position()?;
        let payload_len = usize::try_from(payload_size)
            .map_err(|_| invalid_value("Payload", "payload is too large to decode"))?;
        let payload = match read_exact_vec_untrusted(reader, payload_len) {
            Ok(payload) => payload,
            Err(error) => {
                reader.seek(SeekFrom::Start(start))?;
                return Err(error.into());
            }
        };

        let parse_result = (|| -> Result<(), CodecError> {
            if payload.len() < 6 {
                return Err(invalid_value("Payload", "payload is too short").into());
            }

            let mut offset = 0_usize;
            self.configuration_version = payload[offset];
            offset += 1;
            self.profile = payload[offset];
            offset += 1;
            self.profile_compatibility = payload[offset];
            offset += 1;
            self.level = payload[offset];
            offset += 1;

            let length_size = payload[offset];
            if length_size >> 2 != 0x3f {
                return Err(CodecError::ConstantMismatch {
                    field_name: "Reserved",
                    constant: "63",
                });
            }
            self.length_size_minus_one = length_size & 0x03;
            offset += 1;

            let sequence_count = payload[offset];
            if sequence_count >> 5 != 0x07 {
                return Err(CodecError::ConstantMismatch {
                    field_name: "Reserved2",
                    constant: "7",
                });
            }
            self.num_of_sequence_parameter_sets = sequence_count & 0x1f;
            offset += 1;

            let sequence_start = offset;
            self.sequence_parameter_sets = Vec::with_capacity(untrusted_prealloc_hint(
                usize::from(self.num_of_sequence_parameter_sets),
            ));
            for _ in 0..self.num_of_sequence_parameter_sets {
                if payload.len().saturating_sub(offset) < 2 {
                    return Err(invalid_value(
                        "SequenceParameterSets",
                        "parameter-set payload length does not match the entry count",
                    )
                    .into());
                }
                let length = read_u16(&payload, offset);
                offset += 2;
                let end = offset + usize::from(length);
                if end > payload.len() {
                    return Err(invalid_value(
                        "SequenceParameterSets",
                        "parameter-set payload length does not match the entry count",
                    )
                    .into());
                }
                self.sequence_parameter_sets.push(AVCParameterSet {
                    length,
                    nal_unit: payload[offset..end].to_vec(),
                });
                offset = end;
            }
            let _ = sequence_start;

            if offset >= payload.len() {
                return Err(invalid_value("Payload", "payload is too short").into());
            }
            self.num_of_picture_parameter_sets = payload[offset];
            offset += 1;

            self.picture_parameter_sets = Vec::with_capacity(untrusted_prealloc_hint(usize::from(
                self.num_of_picture_parameter_sets,
            )));
            for _ in 0..self.num_of_picture_parameter_sets {
                if payload.len().saturating_sub(offset) < 2 {
                    return Err(invalid_value(
                        "PictureParameterSets",
                        "parameter-set payload length does not match the entry count",
                    )
                    .into());
                }
                let length = read_u16(&payload, offset);
                offset += 2;
                let end = offset + usize::from(length);
                if end > payload.len() {
                    return Err(invalid_value(
                        "PictureParameterSets",
                        "parameter-set payload length does not match the entry count",
                    )
                    .into());
                }
                self.picture_parameter_sets.push(AVCParameterSet {
                    length,
                    nal_unit: payload[offset..end].to_vec(),
                });
                offset = end;
            }

            self.high_profile_fields_enabled = false;
            self.chroma_format = 0;
            self.bit_depth_luma_minus8 = 0;
            self.bit_depth_chroma_minus8 = 0;
            self.num_of_sequence_parameter_set_ext = 0;
            self.sequence_parameter_sets_ext.clear();

            let remaining = payload.len().saturating_sub(offset);
            if avc_profile_supports_extensions(self.profile) && remaining != 0 {
                if remaining < 4 {
                    return Err(invalid_value("Payload", "payload is truncated").into());
                }

                self.high_profile_fields_enabled = true;

                let chroma_format = payload[offset];
                if chroma_format >> 2 != 0x3f {
                    return Err(CodecError::ConstantMismatch {
                        field_name: "Reserved3",
                        constant: "63",
                    });
                }
                self.chroma_format = chroma_format & 0x03;
                offset += 1;

                let bit_depth_luma = payload[offset];
                if bit_depth_luma >> 3 != 0x1f {
                    return Err(CodecError::ConstantMismatch {
                        field_name: "Reserved4",
                        constant: "31",
                    });
                }
                self.bit_depth_luma_minus8 = bit_depth_luma & 0x07;
                offset += 1;

                let bit_depth_chroma = payload[offset];
                if bit_depth_chroma >> 3 != 0x1f {
                    return Err(CodecError::ConstantMismatch {
                        field_name: "Reserved5",
                        constant: "31",
                    });
                }
                self.bit_depth_chroma_minus8 = bit_depth_chroma & 0x07;
                offset += 1;

                self.num_of_sequence_parameter_set_ext = payload[offset];
                offset += 1;

                self.sequence_parameter_sets_ext = Vec::with_capacity(untrusted_prealloc_hint(
                    usize::from(self.num_of_sequence_parameter_set_ext),
                ));
                for _ in 0..self.num_of_sequence_parameter_set_ext {
                    if payload.len().saturating_sub(offset) < 2 {
                        return Err(invalid_value(
                            "SequenceParameterSetsExt",
                            "parameter-set payload length does not match the entry count",
                        )
                        .into());
                    }
                    let length = read_u16(&payload, offset);
                    offset += 2;
                    let end = offset + usize::from(length);
                    if end > payload.len() {
                        return Err(invalid_value(
                            "SequenceParameterSetsExt",
                            "parameter-set payload length does not match the entry count",
                        )
                        .into());
                    }
                    self.sequence_parameter_sets_ext.push(AVCParameterSet {
                        length,
                        nal_unit: payload[offset..end].to_vec(),
                    });
                    offset = end;
                }
            }

            if offset != payload.len() {
                return Err(invalid_value("Payload", "payload has trailing bytes").into());
            }

            Ok(())
        })();

        if let Err(error) = parse_result {
            reader.seek(SeekFrom::Start(start))?;
            return Err(error);
        }

        Ok(Some(payload_size))
    }
}

/// One length-prefixed HEVC NAL-unit record carried by `hvcC`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HEVCNalu {
    pub length: u16,
    pub nal_unit: Vec<u8>,
}

/// One HEVC NAL-array grouping carried by `hvcC`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HEVCNaluArray {
    pub completeness: bool,
    pub reserved: bool,
    pub nalu_type: u8,
    pub num_nalus: u16,
    pub nalus: Vec<HEVCNalu>,
}

/// HEVC decoder configuration carried by visual sample entries such as `hvc1` and `hev1`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HEVCDecoderConfiguration {
    pub configuration_version: u8,
    pub general_profile_space: u8,
    pub general_tier_flag: bool,
    pub general_profile_idc: u8,
    pub general_profile_compatibility: [bool; 32],
    pub general_constraint_indicator: [u8; 6],
    pub general_level_idc: u8,
    pub min_spatial_segmentation_idc: u16,
    pub parallelism_type: u8,
    pub chroma_format_idc: u8,
    pub bit_depth_luma_minus8: u8,
    pub bit_depth_chroma_minus8: u8,
    pub avg_frame_rate: u16,
    pub constant_frame_rate: u8,
    pub num_temporal_layers: u8,
    pub temporal_id_nested: u8,
    pub length_size_minus_one: u8,
    pub num_of_nalu_arrays: u8,
    pub nalu_arrays: Vec<HEVCNaluArray>,
}

impl FieldHooks for HEVCDecoderConfiguration {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        match name {
            "NaluArrays" => encoded_hevc_nalu_arrays_len(name, &self.nalu_arrays).ok(),
            _ => None,
        }
    }

    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "NaluArrays" => Some(render_hevc_nalu_arrays(&self.nalu_arrays)),
            _ => None,
        }
    }
}

impl ImmutableBox for HEVCDecoderConfiguration {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"hvcC")
    }
}

impl MutableBox for HEVCDecoderConfiguration {}

impl FieldValueRead for HEVCDecoderConfiguration {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "ConfigurationVersion" => {
                Ok(FieldValue::Unsigned(u64::from(self.configuration_version)))
            }
            "GeneralProfileSpace" => {
                Ok(FieldValue::Unsigned(u64::from(self.general_profile_space)))
            }
            "GeneralTierFlag" => Ok(FieldValue::Boolean(self.general_tier_flag)),
            "GeneralProfileIdc" => Ok(FieldValue::Unsigned(u64::from(self.general_profile_idc))),
            "GeneralProfileCompatibility" => Ok(FieldValue::BooleanArray(
                self.general_profile_compatibility.to_vec(),
            )),
            "GeneralConstraintIndicator" => Ok(FieldValue::UnsignedArray(
                self.general_constraint_indicator
                    .iter()
                    .copied()
                    .map(u64::from)
                    .collect(),
            )),
            "GeneralLevelIdc" => Ok(FieldValue::Unsigned(u64::from(self.general_level_idc))),
            "MinSpatialSegmentationIdc" => Ok(FieldValue::Unsigned(u64::from(
                self.min_spatial_segmentation_idc,
            ))),
            "ParallelismType" => Ok(FieldValue::Unsigned(u64::from(self.parallelism_type))),
            "ChromaFormatIdc" => Ok(FieldValue::Unsigned(u64::from(self.chroma_format_idc))),
            "BitDepthLumaMinus8" => Ok(FieldValue::Unsigned(u64::from(self.bit_depth_luma_minus8))),
            "BitDepthChromaMinus8" => Ok(FieldValue::Unsigned(u64::from(
                self.bit_depth_chroma_minus8,
            ))),
            "AvgFrameRate" => Ok(FieldValue::Unsigned(u64::from(self.avg_frame_rate))),
            "ConstantFrameRate" => Ok(FieldValue::Unsigned(u64::from(self.constant_frame_rate))),
            "NumTemporalLayers" => Ok(FieldValue::Unsigned(u64::from(self.num_temporal_layers))),
            "TemporalIdNested" => Ok(FieldValue::Unsigned(u64::from(self.temporal_id_nested))),
            "LengthSizeMinusOne" => Ok(FieldValue::Unsigned(u64::from(self.length_size_minus_one))),
            "NumOfNaluArrays" => Ok(FieldValue::Unsigned(u64::from(self.num_of_nalu_arrays))),
            "NaluArrays" => Ok(FieldValue::Bytes(encode_hevc_nalu_arrays(
                field_name,
                &self.nalu_arrays,
            )?)),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for HEVCDecoderConfiguration {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("ConfigurationVersion", FieldValue::Unsigned(value)) => {
                self.configuration_version = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("GeneralProfileSpace", FieldValue::Unsigned(value)) => {
                self.general_profile_space = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("GeneralTierFlag", FieldValue::Boolean(value)) => {
                self.general_tier_flag = value;
                Ok(())
            }
            ("GeneralProfileIdc", FieldValue::Unsigned(value)) => {
                self.general_profile_idc = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("GeneralProfileCompatibility", FieldValue::BooleanArray(value)) => {
                if value.len() != 32 {
                    return Err(invalid_value(
                        field_name,
                        "value must contain exactly 32 entries",
                    ));
                }
                let mut flags = [false; 32];
                flags.copy_from_slice(&value);
                self.general_profile_compatibility = flags;
                Ok(())
            }
            ("GeneralConstraintIndicator", FieldValue::UnsignedArray(values)) => {
                if values.len() != 6 {
                    return Err(invalid_value(
                        field_name,
                        "value must contain exactly 6 entries",
                    ));
                }
                let mut indicator = [0_u8; 6];
                for (slot, value) in indicator.iter_mut().zip(values) {
                    *slot = u8_from_unsigned(field_name, value)?;
                }
                self.general_constraint_indicator = indicator;
                Ok(())
            }
            ("GeneralLevelIdc", FieldValue::Unsigned(value)) => {
                self.general_level_idc = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("MinSpatialSegmentationIdc", FieldValue::Unsigned(value)) => {
                self.min_spatial_segmentation_idc = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ParallelismType", FieldValue::Unsigned(value)) => {
                self.parallelism_type = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ChromaFormatIdc", FieldValue::Unsigned(value)) => {
                self.chroma_format_idc = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("BitDepthLumaMinus8", FieldValue::Unsigned(value)) => {
                self.bit_depth_luma_minus8 = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("BitDepthChromaMinus8", FieldValue::Unsigned(value)) => {
                self.bit_depth_chroma_minus8 = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("AvgFrameRate", FieldValue::Unsigned(value)) => {
                self.avg_frame_rate = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ConstantFrameRate", FieldValue::Unsigned(value)) => {
                self.constant_frame_rate = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("NumTemporalLayers", FieldValue::Unsigned(value)) => {
                self.num_temporal_layers = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("TemporalIdNested", FieldValue::Unsigned(value)) => {
                self.temporal_id_nested = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("LengthSizeMinusOne", FieldValue::Unsigned(value)) => {
                self.length_size_minus_one = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("NumOfNaluArrays", FieldValue::Unsigned(value)) => {
                self.num_of_nalu_arrays = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("NaluArrays", FieldValue::Bytes(value)) => {
                self.nalu_arrays =
                    parse_hevc_nalu_arrays(field_name, &value, self.num_of_nalu_arrays)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for HEVCDecoderConfiguration {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("ConfigurationVersion", 0, with_bit_width(8), as_hex()),
        codec_field!("GeneralProfileSpace", 1, with_bit_width(2), as_hex()),
        codec_field!("GeneralTierFlag", 2, with_bit_width(1), as_boolean()),
        codec_field!("GeneralProfileIdc", 3, with_bit_width(5), as_hex()),
        codec_field!(
            "GeneralProfileCompatibility",
            4,
            with_bit_width(1),
            with_length(32)
        ),
        codec_field!(
            "GeneralConstraintIndicator",
            5,
            with_bit_width(8),
            with_length(6),
            as_hex()
        ),
        codec_field!("GeneralLevelIdc", 6, with_bit_width(8), as_hex()),
        codec_field!("MinSpatialSegmentationIdc", 7, with_bit_width(12)),
        codec_field!("ParallelismType", 8, with_bit_width(2), as_hex()),
        codec_field!("ChromaFormatIdc", 9, with_bit_width(2), as_hex()),
        codec_field!("BitDepthLumaMinus8", 10, with_bit_width(3), as_hex()),
        codec_field!("BitDepthChromaMinus8", 11, with_bit_width(3), as_hex()),
        codec_field!("AvgFrameRate", 12, with_bit_width(16)),
        codec_field!("ConstantFrameRate", 13, with_bit_width(2), as_hex()),
        codec_field!("NumTemporalLayers", 14, with_bit_width(2), as_hex()),
        codec_field!("TemporalIdNested", 15, with_bit_width(2), as_hex()),
        codec_field!("LengthSizeMinusOne", 16, with_bit_width(2), as_hex()),
        codec_field!("NumOfNaluArrays", 17, with_bit_width(8), as_hex()),
        codec_field!(
            "NaluArrays",
            18,
            with_bit_width(8),
            with_dynamic_length(),
            as_bytes()
        ),
    ]);

    fn custom_marshal(&self, writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        if self.general_profile_space > 0x03 {
            return Err(
                invalid_value("GeneralProfileSpace", "value does not fit in 2 bits").into(),
            );
        }
        if self.general_profile_idc > 0x1f {
            return Err(invalid_value("GeneralProfileIdc", "value does not fit in 5 bits").into());
        }
        if self.min_spatial_segmentation_idc > 0x0fff {
            return Err(invalid_value(
                "MinSpatialSegmentationIdc",
                "value does not fit in 12 bits",
            )
            .into());
        }
        if self.parallelism_type > 0x03 {
            return Err(invalid_value("ParallelismType", "value does not fit in 2 bits").into());
        }
        if self.chroma_format_idc > 0x03 {
            return Err(invalid_value("ChromaFormatIdc", "value does not fit in 2 bits").into());
        }
        if self.bit_depth_luma_minus8 > 0x07 {
            return Err(invalid_value("BitDepthLumaMinus8", "value does not fit in 3 bits").into());
        }
        if self.bit_depth_chroma_minus8 > 0x07 {
            return Err(
                invalid_value("BitDepthChromaMinus8", "value does not fit in 3 bits").into(),
            );
        }
        if self.constant_frame_rate > 0x03 {
            return Err(invalid_value("ConstantFrameRate", "value does not fit in 2 bits").into());
        }
        if self.num_temporal_layers > 0x03 {
            return Err(invalid_value("NumTemporalLayers", "value does not fit in 2 bits").into());
        }
        if self.temporal_id_nested > 0x03 {
            return Err(invalid_value("TemporalIdNested", "value does not fit in 2 bits").into());
        }
        if self.length_size_minus_one > 0x03 {
            return Err(invalid_value("LengthSizeMinusOne", "value does not fit in 2 bits").into());
        }

        require_count(
            "NumOfNaluArrays",
            u32::from(self.num_of_nalu_arrays),
            self.nalu_arrays.len(),
        )?;
        let nalu_arrays = encode_hevc_nalu_arrays("NaluArrays", &self.nalu_arrays)?;

        let mut payload = Vec::with_capacity(23 + nalu_arrays.len());
        payload.push(self.configuration_version);
        payload.push(
            (self.general_profile_space << 6)
                | (u8::from(self.general_tier_flag) << 5)
                | self.general_profile_idc,
        );
        payload.extend_from_slice(&pack_hevc_profile_compatibility(
            &self.general_profile_compatibility,
        ));
        payload.extend_from_slice(&self.general_constraint_indicator);
        payload.push(self.general_level_idc);
        payload.extend_from_slice(&(0xe000 | self.min_spatial_segmentation_idc).to_be_bytes());
        payload.push(0xfc | self.parallelism_type);
        payload.push(0xfc | self.chroma_format_idc);
        payload.push(0xf8 | self.bit_depth_luma_minus8);
        payload.push(0xf8 | self.bit_depth_chroma_minus8);
        payload.extend_from_slice(&self.avg_frame_rate.to_be_bytes());
        payload.push(
            (self.constant_frame_rate << 6)
                | (self.num_temporal_layers << 4)
                | (self.temporal_id_nested << 2)
                | self.length_size_minus_one,
        );
        payload.push(self.num_of_nalu_arrays);
        payload.extend_from_slice(&nalu_arrays);

        writer.write_all(&payload)?;
        Ok(Some(payload.len() as u64))
    }

    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, CodecError> {
        let start = reader.stream_position()?;
        let payload_len = usize::try_from(payload_size)
            .map_err(|_| invalid_value("Payload", "payload is too large to decode"))?;
        let payload = match read_exact_vec_untrusted(reader, payload_len) {
            Ok(payload) => payload,
            Err(error) => {
                reader.seek(SeekFrom::Start(start))?;
                return Err(error.into());
            }
        };

        let parse_result = (|| -> Result<(), CodecError> {
            if payload.len() < 23 {
                return Err(invalid_value("Payload", "payload is too short").into());
            }

            let mut offset = 0_usize;
            self.configuration_version = payload[offset];
            offset += 1;

            let profile_header = payload[offset];
            self.general_profile_space = profile_header >> 6;
            self.general_tier_flag = profile_header & 0x20 != 0;
            self.general_profile_idc = profile_header & 0x1f;
            offset += 1;

            let profile_compatibility: [u8; 4] = payload[offset..offset + 4].try_into().unwrap();
            self.general_profile_compatibility =
                unpack_hevc_profile_compatibility(&profile_compatibility);
            offset += 4;

            self.general_constraint_indicator = payload[offset..offset + 6].try_into().unwrap();
            offset += 6;

            self.general_level_idc = payload[offset];
            offset += 1;

            let segmentation = read_u16(&payload, offset);
            if segmentation >> 12 != 0x0e {
                return Err(CodecError::ConstantMismatch {
                    field_name: "Reserved1",
                    constant: "14",
                });
            }
            self.min_spatial_segmentation_idc = segmentation & 0x0fff;
            offset += 2;

            let parallelism = payload[offset];
            if parallelism >> 2 != 0x3f {
                return Err(CodecError::ConstantMismatch {
                    field_name: "Reserved2",
                    constant: "63",
                });
            }
            self.parallelism_type = parallelism & 0x03;
            offset += 1;

            let chroma_format = payload[offset];
            if chroma_format >> 2 != 0x3f {
                return Err(CodecError::ConstantMismatch {
                    field_name: "Reserved3",
                    constant: "63",
                });
            }
            self.chroma_format_idc = chroma_format & 0x03;
            offset += 1;

            let bit_depth_luma = payload[offset];
            if bit_depth_luma >> 3 != 0x1f {
                return Err(CodecError::ConstantMismatch {
                    field_name: "Reserved4",
                    constant: "31",
                });
            }
            self.bit_depth_luma_minus8 = bit_depth_luma & 0x07;
            offset += 1;

            let bit_depth_chroma = payload[offset];
            if bit_depth_chroma >> 3 != 0x1f {
                return Err(CodecError::ConstantMismatch {
                    field_name: "Reserved5",
                    constant: "31",
                });
            }
            self.bit_depth_chroma_minus8 = bit_depth_chroma & 0x07;
            offset += 1;

            self.avg_frame_rate = read_u16(&payload, offset);
            offset += 2;

            let layer_header = payload[offset];
            self.constant_frame_rate = layer_header >> 6;
            self.num_temporal_layers = (layer_header >> 4) & 0x03;
            self.temporal_id_nested = (layer_header >> 2) & 0x03;
            self.length_size_minus_one = layer_header & 0x03;
            offset += 1;

            self.num_of_nalu_arrays = payload[offset];
            offset += 1;

            self.nalu_arrays =
                parse_hevc_nalu_arrays("NaluArrays", &payload[offset..], self.num_of_nalu_arrays)?;

            Ok(())
        })();

        if let Err(error) = parse_result {
            reader.seek(SeekFrom::Start(start))?;
            return Err(error);
        }

        Ok(Some(payload_size))
    }
}

/// XML subtitle sample entry that stores namespace and schema strings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XMLSubtitleSampleEntry {
    pub sample_entry: SampleEntry,
    pub namespace: String,
    pub schema_location: String,
    pub auxiliary_mime_types: String,
}

impl Default for XMLSubtitleSampleEntry {
    fn default() -> Self {
        Self {
            sample_entry: SampleEntry {
                box_type: FourCc::from_bytes(*b"stpp"),
                data_reference_index: 0,
            },
            namespace: String::new(),
            schema_location: String::new(),
            auxiliary_mime_types: String::new(),
        }
    }
}

impl FieldHooks for XMLSubtitleSampleEntry {}

impl ImmutableBox for XMLSubtitleSampleEntry {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"stpp")
    }
}

impl MutableBox for XMLSubtitleSampleEntry {}

impl XMLSubtitleSampleEntry {
    /// Returns the whitespace-delimited namespace entries.
    pub fn namespace_list(&self) -> Vec<&str> {
        self.namespace.split_whitespace().collect()
    }

    /// Returns the whitespace-delimited schema-location entries.
    pub fn schema_location_list(&self) -> Vec<&str> {
        self.schema_location.split_whitespace().collect()
    }

    /// Returns the whitespace-delimited auxiliary MIME type entries.
    pub fn auxiliary_mime_types_list(&self) -> Vec<&str> {
        self.auxiliary_mime_types.split_whitespace().collect()
    }
}

impl FieldValueRead for XMLSubtitleSampleEntry {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DataReferenceIndex" => Ok(FieldValue::Unsigned(u64::from(
                self.sample_entry.data_reference_index,
            ))),
            "Namespace" => Ok(FieldValue::String(self.namespace.clone())),
            "SchemaLocation" => Ok(FieldValue::String(self.schema_location.clone())),
            "AuxiliaryMIMETypes" => Ok(FieldValue::String(self.auxiliary_mime_types.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for XMLSubtitleSampleEntry {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DataReferenceIndex", FieldValue::Unsigned(value)) => {
                self.sample_entry.data_reference_index = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Namespace", FieldValue::String(value)) => {
                self.namespace = value;
                Ok(())
            }
            ("SchemaLocation", FieldValue::String(value)) => {
                self.schema_location = value;
                Ok(())
            }
            ("AuxiliaryMIMETypes", FieldValue::String(value)) => {
                self.auxiliary_mime_types = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for XMLSubtitleSampleEntry {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Reserved0A", 0, with_bit_width(16), with_constant("0")),
        codec_field!("Reserved0B", 1, with_bit_width(16), with_constant("0")),
        codec_field!("Reserved0C", 2, with_bit_width(16), with_constant("0")),
        codec_field!("DataReferenceIndex", 3, with_bit_width(16)),
        codec_field!(
            "Namespace",
            4,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated)
        ),
        codec_field!(
            "SchemaLocation",
            5,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated)
        ),
        codec_field!(
            "AuxiliaryMIMETypes",
            6,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated)
        ),
    ]);
}

/// Text subtitle sample entry that stores an optional encoding label and MIME type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSubtitleSampleEntry {
    pub sample_entry: SampleEntry,
    pub content_encoding: String,
    pub mime_format: String,
}

impl Default for TextSubtitleSampleEntry {
    fn default() -> Self {
        Self {
            sample_entry: SampleEntry {
                box_type: FourCc::from_bytes(*b"sbtt"),
                data_reference_index: 0,
            },
            content_encoding: String::new(),
            mime_format: String::new(),
        }
    }
}

impl FieldHooks for TextSubtitleSampleEntry {}

impl ImmutableBox for TextSubtitleSampleEntry {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"sbtt")
    }
}

impl MutableBox for TextSubtitleSampleEntry {}

impl FieldValueRead for TextSubtitleSampleEntry {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DataReferenceIndex" => Ok(FieldValue::Unsigned(u64::from(
                self.sample_entry.data_reference_index,
            ))),
            "ContentEncoding" => Ok(FieldValue::String(self.content_encoding.clone())),
            "MIMEFormat" => Ok(FieldValue::String(self.mime_format.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for TextSubtitleSampleEntry {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DataReferenceIndex", FieldValue::Unsigned(value)) => {
                self.sample_entry.data_reference_index = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("ContentEncoding", FieldValue::String(value)) => {
                self.content_encoding = value;
                Ok(())
            }
            ("MIMEFormat", FieldValue::String(value)) => {
                self.mime_format = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for TextSubtitleSampleEntry {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Reserved0A", 0, with_bit_width(16), with_constant("0")),
        codec_field!("Reserved0B", 1, with_bit_width(16), with_constant("0")),
        codec_field!("Reserved0C", 2, with_bit_width(16), with_constant("0")),
        codec_field!("DataReferenceIndex", 3, with_bit_width(16)),
        codec_field!(
            "ContentEncoding",
            4,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated)
        ),
        codec_field!(
            "MIMEFormat",
            5,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated)
        ),
    ]);
}

fn is_quicktime_wave_audio_context(context: BoxLookupContext) -> bool {
    context.is_quicktime_compatible() && context.under_wave()
}

fn matches_audio_sample_entry_context(box_type: FourCc, context: BoxLookupContext) -> bool {
    (box_type == FourCc::from_bytes(*b"enca") || box_type == FourCc::from_bytes(*b"mp4a"))
        && !is_quicktime_wave_audio_context(context)
}

/// Registers the currently implemented ISO/IEC 14496-12 boxes in `registry`.
pub fn register_boxes(registry: &mut BoxRegistry) {
    registry.register::<AVCDecoderConfiguration>(FourCc::from_bytes(*b"avcC"));
    registry.register::<Btrt>(FourCc::from_bytes(*b"btrt"));
    registry.register::<Cdat>(FourCc::from_bytes(*b"cdat"));
    registry.register::<Clap>(FourCc::from_bytes(*b"clap"));
    registry.register::<Colr>(FourCc::from_bytes(*b"colr"));
    registry.register::<CoLL>(FourCc::from_bytes(*b"CoLL"));
    registry.register::<Co64>(FourCc::from_bytes(*b"co64"));
    registry.register::<Cslg>(FourCc::from_bytes(*b"cslg"));
    registry.register::<Ctts>(FourCc::from_bytes(*b"ctts"));
    registry.register::<Dinf>(FourCc::from_bytes(*b"dinf"));
    registry.register::<Dref>(FourCc::from_bytes(*b"dref"));
    registry.register::<Edts>(FourCc::from_bytes(*b"edts"));
    registry.register::<Elng>(FourCc::from_bytes(*b"elng"));
    registry.register::<Elst>(FourCc::from_bytes(*b"elst"));
    registry.register::<Emeb>(FourCc::from_bytes(*b"emeb"));
    registry.register::<Emib>(FourCc::from_bytes(*b"emib"));
    registry.register::<Emsg>(FourCc::from_bytes(*b"emsg"));
    registry.register::<EventMessageSampleEntry>(FourCc::from_bytes(*b"evte"));
    registry.register::<AlbumLoudnessInfo>(FourCc::from_bytes(*b"alou"));
    registry.register_any::<VisualSampleEntry>(FourCc::from_bytes(*b"avc1"));
    registry.register_contextual_any::<WaveAudioData>(
        FourCc::from_bytes(*b"enca"),
        is_quicktime_wave_audio_context,
    );
    registry.register_any::<VisualSampleEntry>(FourCc::from_bytes(*b"encv"));
    registry.register::<Fiel>(FourCc::from_bytes(*b"fiel"));
    registry.register::<Frma>(FourCc::from_bytes(*b"frma"));
    registry.register::<Free>(FourCc::from_bytes(*b"free"));
    registry.register::<Ftyp>(FourCc::from_bytes(*b"ftyp"));
    registry.register::<Hdlr>(FourCc::from_bytes(*b"hdlr"));
    registry.register::<HEVCDecoderConfiguration>(FourCc::from_bytes(*b"hvcC"));
    registry.register_any::<VisualSampleEntry>(FourCc::from_bytes(*b"hev1"));
    registry.register_any::<VisualSampleEntry>(FourCc::from_bytes(*b"hvc1"));
    registry.register::<Kind>(FourCc::from_bytes(*b"kind"));
    registry.register::<Leva>(FourCc::from_bytes(*b"leva"));
    registry.register::<Ludt>(FourCc::from_bytes(*b"ludt"));
    registry.register::<Mdat>(FourCc::from_bytes(*b"mdat"));
    registry.register::<Mdhd>(FourCc::from_bytes(*b"mdhd"));
    registry.register::<Mdia>(FourCc::from_bytes(*b"mdia"));
    registry.register::<Mehd>(FourCc::from_bytes(*b"mehd"));
    registry.register::<Meta>(FourCc::from_bytes(*b"meta"));
    registry.register::<Mfhd>(FourCc::from_bytes(*b"mfhd"));
    registry.register::<Mfra>(FourCc::from_bytes(*b"mfra"));
    registry.register::<Mfro>(FourCc::from_bytes(*b"mfro"));
    registry.register::<Mime>(FourCc::from_bytes(*b"mime"));
    registry.register::<Nmhd>(FourCc::from_bytes(*b"nmhd"));
    registry.register::<Prft>(FourCc::from_bytes(*b"prft"));
    registry.register::<Minf>(FourCc::from_bytes(*b"minf"));
    registry.register::<Moof>(FourCc::from_bytes(*b"moof"));
    registry.register::<Moov>(FourCc::from_bytes(*b"moov"));
    registry.register::<Mvex>(FourCc::from_bytes(*b"mvex"));
    registry.register::<Mvhd>(FourCc::from_bytes(*b"mvhd"));
    registry.register_contextual_any::<WaveAudioData>(
        FourCc::from_bytes(*b"mp4a"),
        is_quicktime_wave_audio_context,
    );
    registry.register_dynamic_any::<AudioSampleEntry>(matches_audio_sample_entry_context);
    registry.register_any::<VisualSampleEntry>(FourCc::from_bytes(*b"mp4v"));
    registry.register::<Pasp>(FourCc::from_bytes(*b"pasp"));
    registry.register::<Saio>(FourCc::from_bytes(*b"saio"));
    registry.register::<Saiz>(FourCc::from_bytes(*b"saiz"));
    registry.register::<Sbgp>(FourCc::from_bytes(*b"sbgp"));
    registry.register::<Schi>(FourCc::from_bytes(*b"schi"));
    registry.register::<Schm>(FourCc::from_bytes(*b"schm"));
    registry.register::<Silb>(FourCc::from_bytes(*b"silb"));
    registry.register::<TextSubtitleSampleEntry>(FourCc::from_bytes(*b"sbtt"));
    registry.register::<Sdtp>(FourCc::from_bytes(*b"sdtp"));
    registry.register::<Sgpd>(FourCc::from_bytes(*b"sgpd"));
    registry.register::<Sidx>(FourCc::from_bytes(*b"sidx"));
    registry.register::<Sinf>(FourCc::from_bytes(*b"sinf"));
    registry.register::<Skip>(FourCc::from_bytes(*b"skip"));
    registry.register::<Smhd>(FourCc::from_bytes(*b"smhd"));
    registry.register::<SmDm>(FourCc::from_bytes(*b"SmDm"));
    registry.register::<Ssix>(FourCc::from_bytes(*b"ssix"));
    registry.register::<Sthd>(FourCc::from_bytes(*b"sthd"));
    registry.register::<Stbl>(FourCc::from_bytes(*b"stbl"));
    registry.register::<Stco>(FourCc::from_bytes(*b"stco"));
    registry.register::<Stsc>(FourCc::from_bytes(*b"stsc"));
    registry.register::<Stsd>(FourCc::from_bytes(*b"stsd"));
    registry.register::<Stss>(FourCc::from_bytes(*b"stss"));
    registry.register::<Stsz>(FourCc::from_bytes(*b"stsz"));
    registry.register::<Stts>(FourCc::from_bytes(*b"stts"));
    registry.register::<Styp>(FourCc::from_bytes(*b"styp"));
    registry.register::<Subs>(FourCc::from_bytes(*b"subs"));
    registry.register::<Tfdt>(FourCc::from_bytes(*b"tfdt"));
    registry.register::<Tfhd>(FourCc::from_bytes(*b"tfhd"));
    registry.register::<Tfra>(FourCc::from_bytes(*b"tfra"));
    registry.register::<Traf>(FourCc::from_bytes(*b"traf"));
    registry.register::<Trak>(FourCc::from_bytes(*b"trak"));
    registry.register::<TrackLoudnessInfo>(FourCc::from_bytes(*b"tlou"));
    registry.register::<Tref>(FourCc::from_bytes(*b"tref"));
    registry.register::<Trep>(FourCc::from_bytes(*b"trep"));
    registry.register::<Trex>(FourCc::from_bytes(*b"trex"));
    registry.register::<Trun>(FourCc::from_bytes(*b"trun"));
    registry.register::<Tkhd>(FourCc::from_bytes(*b"tkhd"));
    registry.register::<Cdsc>(FourCc::from_bytes(*b"cdsc"));
    registry.register::<Dpnd>(FourCc::from_bytes(*b"dpnd"));
    registry.register::<Font>(FourCc::from_bytes(*b"font"));
    registry.register::<Hind>(FourCc::from_bytes(*b"hind"));
    registry.register::<Hint>(FourCc::from_bytes(*b"hint"));
    registry.register::<Ipir>(FourCc::from_bytes(*b"ipir"));
    registry.register::<Mpod>(FourCc::from_bytes(*b"mpod"));
    registry.register::<Subt>(FourCc::from_bytes(*b"subt"));
    registry.register::<Udta>(FourCc::from_bytes(*b"udta"));
    registry.register::<Uuid>(FourCc::from_bytes(*b"uuid"));
    registry.register::<Url>(FourCc::from_bytes(*b"url "));
    registry.register::<Urn>(FourCc::from_bytes(*b"urn "));
    registry.register::<Sync>(FourCc::from_bytes(*b"sync"));
    registry.register::<Vdep>(FourCc::from_bytes(*b"vdep"));
    registry.register::<Vplx>(FourCc::from_bytes(*b"vplx"));
    registry.register::<Vmhd>(FourCc::from_bytes(*b"vmhd"));
    registry.register::<Wave>(FourCc::from_bytes(*b"wave"));
    registry.register::<XMLSubtitleSampleEntry>(FourCc::from_bytes(*b"stpp"));
}
