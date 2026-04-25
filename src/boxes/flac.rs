//! FLAC sample-entry and decoder-configuration box definitions.

use std::io::Write;

use super::iso14496_12::AudioSampleEntry;
use crate::boxes::BoxRegistry;
use crate::codec::{
    CodecBox, CodecError, FieldHooks, FieldTable, FieldValue, FieldValueError, FieldValueRead,
    FieldValueWrite, ImmutableBox, MutableBox, ReadSeek, read_exact_vec_untrusted,
};
use crate::{FourCc, codec_field};

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

fn u32_from_unsigned(field_name: &'static str, value: u64) -> Result<u32, FieldValueError> {
    u32::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in u32"))
}

fn render_hex_bytes(bytes: &[u8]) -> String {
    format!(
        "[{}]",
        bytes
            .iter()
            .map(|byte| format!("0x{byte:x}"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn render_metadata_blocks(blocks: &[FlacMetadataBlock]) -> String {
    format!(
        "[{}]",
        blocks
            .iter()
            .map(|block| {
                format!(
                    "{{LastMetadataBlockFlag={} BlockType={} Length={} BlockData={}}}",
                    block.last_metadata_block_flag,
                    block.block_type,
                    block.length,
                    render_hex_bytes(&block.block_data)
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn write_u24(writer: &mut Vec<u8>, value: u32) {
    writer.push(((value >> 16) & 0xff) as u8);
    writer.push(((value >> 8) & 0xff) as u8);
    writer.push((value & 0xff) as u8);
}

fn read_u24(bytes: &[u8], offset: usize) -> u32 {
    (u32::from(bytes[offset]) << 16)
        | (u32::from(bytes[offset + 1]) << 8)
        | u32::from(bytes[offset + 2])
}

fn encode_metadata_blocks(
    field_name: &'static str,
    blocks: &[FlacMetadataBlock],
) -> Result<Vec<u8>, FieldValueError> {
    let mut bytes = Vec::new();

    for (index, block) in blocks.iter().enumerate() {
        if block.block_type > 0x7f {
            return Err(invalid_value("BlockType", "value does not fit in 7 bits"));
        }
        if block.length > 0x00ff_ffff {
            return Err(invalid_value("Length", "value does not fit in 24 bits"));
        }
        if usize::try_from(block.length).ok() != Some(block.block_data.len()) {
            return Err(invalid_value(
                field_name,
                "block length does not match BlockData length",
            ));
        }
        if index + 1 == blocks.len() {
            if !block.last_metadata_block_flag {
                return Err(invalid_value(
                    field_name,
                    "final metadata block flag must be set",
                ));
            }
        } else if block.last_metadata_block_flag {
            return Err(invalid_value(
                field_name,
                "last metadata block flag must only appear on the final block",
            ));
        }

        bytes.push((u8::from(block.last_metadata_block_flag) << 7) | block.block_type);
        write_u24(&mut bytes, block.length);
        bytes.extend_from_slice(&block.block_data);
    }

    Ok(bytes)
}

fn decode_metadata_blocks(
    field_name: &'static str,
    payload: &[u8],
) -> Result<Vec<FlacMetadataBlock>, FieldValueError> {
    let mut offset = 0usize;
    let mut blocks = Vec::new();

    while offset != payload.len() {
        if payload.len().saturating_sub(offset) < 4 {
            return Err(invalid_value(
                field_name,
                "metadata block header is truncated",
            ));
        }
        let first_byte = payload[offset];
        offset += 1;
        let last_metadata_block_flag = (first_byte & 0x80) != 0;
        let block_type = first_byte & 0x7f;
        let length = read_u24(payload, offset);
        offset += 3;
        let block_len = usize::try_from(length).map_err(|_| {
            invalid_value(field_name, "metadata block length does not fit in usize")
        })?;
        if payload.len().saturating_sub(offset) < block_len {
            return Err(invalid_value(
                field_name,
                "metadata block payload is truncated",
            ));
        }
        let block_data = payload[offset..offset + block_len].to_vec();
        offset += block_len;

        if last_metadata_block_flag && offset != payload.len() {
            return Err(invalid_value(
                field_name,
                "last metadata block flag must only appear on the final block",
            ));
        }

        blocks.push(FlacMetadataBlock {
            last_metadata_block_flag,
            block_type,
            length,
            block_data,
        });
    }

    if blocks
        .last()
        .is_some_and(|block| !block.last_metadata_block_flag)
    {
        return Err(invalid_value(
            field_name,
            "final metadata block flag must be set",
        ));
    }

    Ok(blocks)
}

/// One FLAC metadata block carried by `dfLa`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FlacMetadataBlock {
    /// Whether this block is the final block in the `dfLa` payload.
    pub last_metadata_block_flag: bool,
    /// Seven-bit FLAC metadata-block type.
    pub block_type: u8,
    /// Declared payload length in bytes.
    pub length: u32,
    /// Opaque block payload bytes.
    pub block_data: Vec<u8>,
}

/// FLAC-specific configuration box carried by `fLaC` sample entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DfLa {
    full_box: FullBoxState,
    /// Ordered FLAC metadata blocks.
    pub metadata_blocks: Vec<FlacMetadataBlock>,
}

impl FieldHooks for DfLa {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "MetadataBlocks" => Some(render_metadata_blocks(&self.metadata_blocks)),
            _ => None,
        }
    }
}

impl ImmutableBox for DfLa {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"dfLa")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for DfLa {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }
}

impl FieldValueRead for DfLa {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Version" => Ok(FieldValue::Unsigned(u64::from(self.version()))),
            "Flags" => Ok(FieldValue::Unsigned(u64::from(self.flags()))),
            "MetadataBlocks" => Ok(FieldValue::Bytes(encode_metadata_blocks(
                field_name,
                &self.metadata_blocks,
            )?)),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for DfLa {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Version", FieldValue::Unsigned(value)) => {
                self.full_box.version = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Flags", FieldValue::Unsigned(value)) => {
                self.full_box.flags = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("MetadataBlocks", FieldValue::Bytes(value)) => {
                self.metadata_blocks = decode_metadata_blocks(field_name, &value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for DfLa {
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("MetadataBlocks", 2, with_bit_width(8), as_bytes()),
    ]);

    fn custom_marshal(&self, writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        if self.version() != 0 {
            return Err(invalid_value("Version", "unsupported version").into());
        }
        if self.flags() != 0 {
            return Err(invalid_value("Flags", "unsupported flags").into());
        }

        let blocks = encode_metadata_blocks("MetadataBlocks", &self.metadata_blocks)?;
        writer.write_all(&[self.version()])?;
        writer.write_all(&self.flags().to_be_bytes()[1..])?;
        writer.write_all(&blocks)?;
        Ok(Some(4 + blocks.len() as u64))
    }

    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, CodecError> {
        if payload_size < 4 {
            return Err(invalid_value("Payload", "payload is too short").into());
        }

        let payload = read_exact_vec_untrusted(reader, payload_size as usize)?;
        let version = payload[0];
        let flags =
            (u32::from(payload[1]) << 16) | (u32::from(payload[2]) << 8) | u32::from(payload[3]);
        if version != 0 {
            return Err(invalid_value("Version", "unsupported version").into());
        }
        if flags != 0 {
            return Err(invalid_value("Flags", "unsupported flags").into());
        }

        self.full_box = FullBoxState { version, flags };
        self.metadata_blocks = decode_metadata_blocks("MetadataBlocks", &payload[4..])?;
        Ok(Some(payload_size))
    }
}

/// Registers the currently implemented FLAC boxes in `registry`.
pub fn register_boxes(registry: &mut BoxRegistry) {
    registry.register_any::<AudioSampleEntry>(FourCc::from_bytes(*b"fLaC"));
    registry.register::<DfLa>(FourCc::from_bytes(*b"dfLa"));
}
