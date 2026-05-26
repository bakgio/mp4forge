//! Dolby audio sample-entry child box definitions.

use std::io::{Cursor, Write};

#[cfg(feature = "async")]
use crate::async_io::{AsyncReadSeek, AsyncWriteSeek};
use crate::bitio::{BitReader, BitWriter};
use crate::boxes::BoxRegistry;
use crate::boxes::iso14496_12::AudioSampleEntry;
#[cfg(feature = "async")]
use crate::codec::CodecFuture;
use crate::codec::{
    CodecBox, CodecError, FieldHooks, FieldTable, FieldValue, FieldValueError, FieldValueRead,
    FieldValueWrite, ImmutableBox, MutableBox, ReadSeek,
};
use crate::{FourCc, codec_field};

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

fn u16_from_unsigned(field_name: &'static str, value: u64) -> Result<u16, FieldValueError> {
    u16::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in u16"))
}

fn u32_from_unsigned(field_name: &'static str, value: u64) -> Result<u32, FieldValueError> {
    u32::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in u32"))
}

fn read_bits_u16(
    reader: &mut BitReader<Cursor<&[u8]>>,
    width: usize,
    field_name: &'static str,
) -> Result<u16, CodecError> {
    let bits = reader.read_bits(width)?;
    let mut value = 0_u16;
    for byte in bits {
        value = (value << 8) | u16::from(byte);
    }
    let mask = if width == 16 {
        u16::MAX
    } else {
        (1_u16 << width) - 1
    };
    if value > mask {
        return Err(
            invalid_value(field_name, "value does not fit in the declared bit width").into(),
        );
    }
    Ok(value & mask)
}

/// Dolby TrueHD configuration box carried by `mlpa` sample entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dmlp {
    /// Packed TrueHD stream-format flags copied from the decoder configuration record.
    pub format_info: u32,
    /// Fifteen-bit peak-data-rate field from the decoder configuration record.
    pub peak_data_rate: u16,
}

impl FieldHooks for Dmlp {}

impl ImmutableBox for Dmlp {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"dmlp")
    }
}

impl MutableBox for Dmlp {}

impl FieldValueRead for Dmlp {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "FormatInfo" => Ok(FieldValue::Unsigned(u64::from(self.format_info))),
            "PeakDataRate" => Ok(FieldValue::Unsigned(u64::from(self.peak_data_rate))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Dmlp {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("FormatInfo", FieldValue::Unsigned(value)) => {
                self.format_info = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("PeakDataRate", FieldValue::Unsigned(value)) => {
                self.peak_data_rate = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Dmlp {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("FormatInfo", 0, with_bit_width(32)),
        codec_field!("PeakDataRate", 1, with_bit_width(15)),
    ]);

    fn custom_marshal(&self, writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        if self.peak_data_rate > 0x7FFF {
            return Err(invalid_value("PeakDataRate", "value does not fit in 15 bits").into());
        }
        writer.write_all(&self.format_info.to_be_bytes())?;
        let mut bit_writer = BitWriter::new(Vec::new());
        bit_writer.write_bit(false)?;
        bit_writer.write_bits(&self.peak_data_rate.to_be_bytes(), 15)?;
        let rate_bits = bit_writer.into_inner()?;
        writer.write_all(&rate_bits)?;
        writer.write_all(&[0, 0, 0, 0])?;
        Ok(Some(10))
    }

    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, CodecError> {
        if payload_size != 10 {
            return Err(invalid_value("Dmlp", "payload size must be exactly 10 bytes").into());
        }
        let mut payload = [0_u8; 10];
        std::io::Read::read_exact(reader, &mut payload)?;
        self.format_info = u32::from_be_bytes(payload[..4].try_into().unwrap());
        let mut bit_reader = BitReader::new(Cursor::new(&payload[4..6]));
        let _reserved = bit_reader.read_bit()?;
        self.peak_data_rate = read_bits_u16(&mut bit_reader, 15, "PeakDataRate")?;
        Ok(Some(10))
    }

    #[cfg(feature = "async")]
    fn custom_marshal_async<'a>(
        &'a self,
        writer: &'a mut dyn AsyncWriteSeek,
    ) -> CodecFuture<'a, Result<Option<u64>, CodecError>> {
        Box::pin(async move {
            let mut bytes = Vec::new();
            let written = self.custom_marshal(&mut bytes)?.unwrap_or(0);
            tokio::io::AsyncWriteExt::write_all(writer, &bytes).await?;
            Ok(Some(written))
        })
    }

    #[cfg(feature = "async")]
    fn custom_unmarshal_async<'a>(
        &'a mut self,
        reader: &'a mut dyn AsyncReadSeek,
        payload_size: u64,
    ) -> CodecFuture<'a, Result<Option<u64>, CodecError>> {
        Box::pin(async move {
            if payload_size != 10 {
                return Err(invalid_value("Dmlp", "payload size must be exactly 10 bytes").into());
            }
            let mut payload = [0_u8; 10];
            tokio::io::AsyncReadExt::read_exact(reader, &mut payload).await?;
            self.format_info = u32::from_be_bytes(payload[..4].try_into().unwrap());
            let mut bit_reader = BitReader::new(Cursor::new(&payload[4..6]));
            let _reserved = bit_reader.read_bit()?;
            self.peak_data_rate = read_bits_u16(&mut bit_reader, 15, "PeakDataRate")?;
            Ok(Some(10))
        })
    }
}

/// Registers the currently implemented Dolby audio boxes in `registry`.
pub fn register_boxes(registry: &mut BoxRegistry) {
    registry.register_any::<AudioSampleEntry>(FourCc::from_bytes(*b"mlpa"));
    registry.register::<Dmlp>(FourCc::from_bytes(*b"dmlp"));
}
