//! IAMF sample-entry child box definitions.

use std::io::Write;

#[cfg(feature = "async")]
use crate::async_io::{AsyncReadSeek, AsyncWriteSeek};
use crate::codec::{
    CodecBox, CodecError, FieldHooks, FieldTable, FieldValue, FieldValueError, FieldValueRead,
    FieldValueWrite, ImmutableBox, MutableBox, ReadSeek, read_exact_vec_untrusted,
};
#[cfg(feature = "async")]
use crate::codec::{CodecFuture, read_exact_vec_untrusted_async};
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

fn u8_from_unsigned(field_name: &'static str, value: u64) -> Result<u8, FieldValueError> {
    u8::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in u8"))
}

fn write_leb128(
    writer: &mut dyn Write,
    field_name: &'static str,
    mut value: usize,
) -> Result<u64, CodecError> {
    let mut written = 0u64;
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        writer.write_all(&[byte])?;
        written += 1;
        if value == 0 {
            return Ok(written);
        }
        if written > 10 {
            return Err(
                invalid_value(field_name, "leb128 length exceeds the supported range").into(),
            );
        }
    }
}

fn read_leb128(
    reader: &mut dyn ReadSeek,
    field_name: &'static str,
) -> Result<(usize, u64), CodecError> {
    let mut value = 0usize;
    let mut shift = 0usize;
    let mut read = 0u64;
    loop {
        let mut byte = [0u8; 1];
        std::io::Read::read_exact(reader, &mut byte)?;
        read += 1;
        value |= usize::from(byte[0] & 0x7f) << shift;
        if byte[0] & 0x80 == 0 {
            return Ok((value, read));
        }
        shift += 7;
        if shift >= usize::BITS as usize {
            return Err(
                invalid_value(field_name, "leb128 length exceeds the supported range").into(),
            );
        }
    }
}

#[cfg(feature = "async")]
async fn write_leb128_async(
    writer: &mut dyn AsyncWriteSeek,
    field_name: &'static str,
    mut value: usize,
) -> Result<u64, CodecError> {
    let mut written = 0u64;
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        tokio::io::AsyncWriteExt::write_all(writer, &[byte]).await?;
        written += 1;
        if value == 0 {
            return Ok(written);
        }
        if written > 10 {
            return Err(
                invalid_value(field_name, "leb128 length exceeds the supported range").into(),
            );
        }
    }
}

#[cfg(feature = "async")]
async fn read_leb128_async(
    reader: &mut dyn AsyncReadSeek,
    field_name: &'static str,
) -> Result<(usize, u64), CodecError> {
    let mut value = 0usize;
    let mut shift = 0usize;
    let mut read = 0u64;
    loop {
        let mut byte = [0u8; 1];
        tokio::io::AsyncReadExt::read_exact(reader, &mut byte).await?;
        read += 1;
        value |= usize::from(byte[0] & 0x7f) << shift;
        if byte[0] & 0x80 == 0 {
            return Ok((value, read));
        }
        shift += 7;
        if shift >= usize::BITS as usize {
            return Err(
                invalid_value(field_name, "leb128 length exceeds the supported range").into(),
            );
        }
    }
}

/// IAMF configuration box carried by `iamf` sample entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Iacb {
    /// Configuration-record version.
    pub configuration_version: u8,
    /// Raw concatenated IAMF descriptor OBUs stored in the configuration record.
    pub config_obus: Vec<u8>,
}

impl FieldHooks for Iacb {}

impl ImmutableBox for Iacb {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"iacb")
    }
}

impl MutableBox for Iacb {}

impl FieldValueRead for Iacb {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "ConfigurationVersion" => {
                Ok(FieldValue::Unsigned(u64::from(self.configuration_version)))
            }
            "ConfigObus" => Ok(FieldValue::Bytes(self.config_obus.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Iacb {
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
            ("ConfigObus", FieldValue::Bytes(value)) => {
                self.config_obus = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Iacb {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("ConfigurationVersion", 0, with_bit_width(8)),
        codec_field!("ConfigObus", 1, with_bit_width(8), as_bytes()),
    ]);

    fn custom_marshal(&self, writer: &mut dyn Write) -> Result<Option<u64>, CodecError> {
        if self.configuration_version != 1 {
            return Err(invalid_value(
                "ConfigurationVersion",
                "only version 1 is currently supported",
            )
            .into());
        }
        writer.write_all(&[self.configuration_version])?;
        let mut written = 1u64;
        written += write_leb128(writer, "ConfigObus", self.config_obus.len())?;
        writer.write_all(&self.config_obus)?;
        written += u64::try_from(self.config_obus.len()).unwrap();
        Ok(Some(written))
    }

    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, CodecError> {
        let mut version = [0u8; 1];
        std::io::Read::read_exact(reader, &mut version)?;
        self.configuration_version = version[0];
        if self.configuration_version != 1 {
            return Err(invalid_value(
                "ConfigurationVersion",
                "only version 1 is currently supported",
            )
            .into());
        }
        let (config_len, leb128_len_read) = read_leb128(reader, "ConfigObus")?;
        let header_size = 1u64
            .checked_add(leb128_len_read)
            .ok_or_else(|| invalid_value("ConfigObus", "payload header size overflowed"))?;
        let total = header_size
            .checked_add(u64::try_from(config_len).unwrap())
            .ok_or_else(|| invalid_value("ConfigObus", "payload size overflowed"))?;
        if total != payload_size {
            return Err(invalid_value(
                "ConfigObus",
                "payload length did not match the declared leb128 size",
            )
            .into());
        }
        self.config_obus = read_exact_vec_untrusted(reader, config_len)?;
        Ok(Some(total))
    }

    #[cfg(feature = "async")]
    fn custom_marshal_async<'a>(
        &'a self,
        writer: &'a mut dyn AsyncWriteSeek,
    ) -> CodecFuture<'a, Result<Option<u64>, CodecError>> {
        Box::pin(async move {
            if self.configuration_version != 1 {
                return Err(invalid_value(
                    "ConfigurationVersion",
                    "only version 1 is currently supported",
                )
                .into());
            }
            tokio::io::AsyncWriteExt::write_all(writer, &[self.configuration_version]).await?;
            let mut written = 1u64;
            written += write_leb128_async(writer, "ConfigObus", self.config_obus.len()).await?;
            tokio::io::AsyncWriteExt::write_all(writer, &self.config_obus).await?;
            written += u64::try_from(self.config_obus.len()).unwrap();
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
            let mut version = [0u8; 1];
            tokio::io::AsyncReadExt::read_exact(reader, &mut version).await?;
            self.configuration_version = version[0];
            if self.configuration_version != 1 {
                return Err(invalid_value(
                    "ConfigurationVersion",
                    "only version 1 is currently supported",
                )
                .into());
            }
            let (config_len, leb128_len_read) = read_leb128_async(reader, "ConfigObus").await?;
            let header_size = 1u64
                .checked_add(leb128_len_read)
                .ok_or_else(|| invalid_value("ConfigObus", "payload header size overflowed"))?;
            let total = header_size
                .checked_add(u64::try_from(config_len).unwrap())
                .ok_or_else(|| invalid_value("ConfigObus", "payload size overflowed"))?;
            if total != payload_size {
                return Err(invalid_value(
                    "ConfigObus",
                    "payload length did not match the declared leb128 size",
                )
                .into());
            }
            self.config_obus = read_exact_vec_untrusted_async(reader, config_len).await?;
            Ok(Some(total))
        })
    }
}
