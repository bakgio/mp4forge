//! ETSI TS 102 366 AC-3 and E-AC-3 sample-entry and decoder-configuration box definitions.

use std::io::{Cursor, Read};

use super::iso14496_12::AudioSampleEntry;
use crate::bitio::{BitReader, BitWriter};
use crate::boxes::BoxRegistry;
use crate::codec::{
    CodecBox, FieldHooks, FieldTable, FieldValue, FieldValueError, FieldValueRead, FieldValueWrite,
    ImmutableBox, MutableBox,
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

fn u8_from_unsigned(field_name: &'static str, value: u64) -> Result<u8, FieldValueError> {
    u8::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in u8"))
}

fn u16_from_unsigned(field_name: &'static str, value: u64) -> Result<u16, FieldValueError> {
    u16::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in u16"))
}

fn read_u8_bits(
    reader: &mut BitReader<Cursor<&[u8]>>,
    width: usize,
    field_name: &'static str,
) -> Result<u8, FieldValueError> {
    let data = reader
        .read_bits(width)
        .map_err(|_| invalid_value(field_name, "substream payload is truncated"))?;
    Ok(data
        .into_iter()
        .fold(0_u16, |acc, byte| (acc << 8) | u16::from(byte)) as u8)
}

fn read_u16_bits(
    reader: &mut BitReader<Cursor<&[u8]>>,
    width: usize,
    field_name: &'static str,
) -> Result<u16, FieldValueError> {
    let data = reader
        .read_bits(width)
        .map_err(|_| invalid_value(field_name, "substream payload is truncated"))?;
    Ok(data
        .into_iter()
        .fold(0_u16, |acc, byte| (acc << 8) | u16::from(byte)))
}

fn format_bytes(bytes: &[u8]) -> String {
    let rendered = bytes
        .iter()
        .map(|byte| format!("0x{byte:x}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{rendered}]")
}

fn render_ec3_substream(substream: &Ec3Substream) -> String {
    format!(
        "{{FSCod=0x{:x} BSID=0x{:x} ASVC=0x{:x} BSMod=0x{:x} ACMod=0x{:x} LFEOn=0x{:x} NumDepSub=0x{:x} ChanLoc=0x{:x}}}",
        substream.fscod,
        substream.bsid,
        substream.asvc,
        substream.bsmod,
        substream.acmod,
        substream.lfe_on,
        substream.num_dep_sub,
        substream.chan_loc
    )
}

fn render_ec3_substreams(substreams: &[Ec3Substream]) -> String {
    let rendered = substreams
        .iter()
        .map(render_ec3_substream)
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{rendered}]")
}

fn require_ec3_substream_count(
    field_name: &'static str,
    num_ind_sub: u8,
    actual_count: usize,
) -> Result<(), FieldValueError> {
    let expected_count = usize::from(num_ind_sub) + 1;
    if actual_count != expected_count {
        return Err(invalid_value(
            field_name,
            "num_ind_sub does not match the parsed substream count",
        ));
    }
    Ok(())
}

fn validate_ec3_substream(
    field_name: &'static str,
    substream: &Ec3Substream,
) -> Result<(), FieldValueError> {
    if substream.fscod > 0x03 {
        return Err(invalid_value(
            field_name,
            "substream fscod does not fit in 2 bits",
        ));
    }
    if substream.bsid > 0x1f {
        return Err(invalid_value(
            field_name,
            "substream bsid does not fit in 5 bits",
        ));
    }
    if substream.asvc > 0x01 {
        return Err(invalid_value(
            field_name,
            "substream asvc does not fit in 1 bit",
        ));
    }
    if substream.bsmod > 0x07 {
        return Err(invalid_value(
            field_name,
            "substream bsmod does not fit in 3 bits",
        ));
    }
    if substream.acmod > 0x07 {
        return Err(invalid_value(
            field_name,
            "substream acmod does not fit in 3 bits",
        ));
    }
    if substream.lfe_on > 0x01 {
        return Err(invalid_value(
            field_name,
            "substream lfe_on does not fit in 1 bit",
        ));
    }
    if substream.num_dep_sub > 0x0f {
        return Err(invalid_value(
            field_name,
            "substream num_dep_sub does not fit in 4 bits",
        ));
    }
    if substream.chan_loc > 0x01ff {
        return Err(invalid_value(
            field_name,
            "substream chan_loc does not fit in 9 bits",
        ));
    }
    if substream.num_dep_sub == 0 && substream.chan_loc != 0 {
        return Err(invalid_value(
            field_name,
            "substream chan_loc requires num_dep_sub to be non-zero",
        ));
    }
    Ok(())
}

fn encode_ec3_substreams(
    field_name: &'static str,
    num_ind_sub: u8,
    substreams: &[Ec3Substream],
    reserved: &[u8],
) -> Result<Vec<u8>, FieldValueError> {
    require_ec3_substream_count(field_name, num_ind_sub, substreams.len())?;
    let mut writer = BitWriter::new(Vec::new());
    for substream in substreams {
        validate_ec3_substream(field_name, substream)?;
        writer
            .write_bits(&[substream.fscod], 2)
            .map_err(|_| invalid_value(field_name, "failed to encode substream payload"))?;
        writer
            .write_bits(&[substream.bsid], 5)
            .map_err(|_| invalid_value(field_name, "failed to encode substream payload"))?;
        writer
            .write_bits(&[0], 1)
            .map_err(|_| invalid_value(field_name, "failed to encode substream payload"))?;
        writer
            .write_bits(&[substream.asvc], 1)
            .map_err(|_| invalid_value(field_name, "failed to encode substream payload"))?;
        writer
            .write_bits(&[substream.bsmod], 3)
            .map_err(|_| invalid_value(field_name, "failed to encode substream payload"))?;
        writer
            .write_bits(&[substream.acmod], 3)
            .map_err(|_| invalid_value(field_name, "failed to encode substream payload"))?;
        writer
            .write_bits(&[substream.lfe_on], 1)
            .map_err(|_| invalid_value(field_name, "failed to encode substream payload"))?;
        writer
            .write_bits(&[0], 3)
            .map_err(|_| invalid_value(field_name, "failed to encode substream payload"))?;
        writer
            .write_bits(&[substream.num_dep_sub], 4)
            .map_err(|_| invalid_value(field_name, "failed to encode substream payload"))?;
        if substream.num_dep_sub > 0 {
            writer
                .write_bits(&substream.chan_loc.to_be_bytes(), 9)
                .map_err(|_| invalid_value(field_name, "failed to encode substream payload"))?;
        } else {
            writer
                .write_bits(&[0], 1)
                .map_err(|_| invalid_value(field_name, "failed to encode substream payload"))?;
        }
    }
    let mut encoded = writer
        .into_inner()
        .map_err(|_| invalid_value(field_name, "encoded substream payload is not aligned"))?;
    encoded.extend_from_slice(reserved);
    Ok(encoded)
}

fn parse_ec3_substreams(
    field_name: &'static str,
    num_ind_sub: u8,
    payload: &[u8],
) -> Result<(Vec<Ec3Substream>, Vec<u8>), FieldValueError> {
    let expected_count = usize::from(num_ind_sub) + 1;
    let mut reader = BitReader::new(Cursor::new(payload));
    let mut substreams = Vec::with_capacity(expected_count);

    for _ in 0..expected_count {
        let fscod = read_u8_bits(&mut reader, 2, field_name)?;
        let bsid = read_u8_bits(&mut reader, 5, field_name)?;
        if read_u8_bits(&mut reader, 1, field_name)? != 0 {
            return Err(invalid_value(
                field_name,
                "substream reserved bit is not zero",
            ));
        }
        let asvc = read_u8_bits(&mut reader, 1, field_name)?;
        let bsmod = read_u8_bits(&mut reader, 3, field_name)?;
        let acmod = read_u8_bits(&mut reader, 3, field_name)?;
        let lfe_on = read_u8_bits(&mut reader, 1, field_name)?;
        if read_u8_bits(&mut reader, 3, field_name)? != 0 {
            return Err(invalid_value(
                field_name,
                "substream reserved bits are not zero",
            ));
        }
        let num_dep_sub = read_u8_bits(&mut reader, 4, field_name)?;
        let chan_loc = if num_dep_sub > 0 {
            read_u16_bits(&mut reader, 9, field_name)?
        } else {
            if read_u8_bits(&mut reader, 1, field_name)? != 0 {
                return Err(invalid_value(
                    field_name,
                    "substream reserved chan_loc bit is not zero",
                ));
            }
            0
        };

        substreams.push(Ec3Substream {
            fscod,
            bsid,
            asvc,
            bsmod,
            acmod,
            lfe_on,
            num_dep_sub,
            chan_loc,
        });
    }

    let mut reserved = Vec::new();
    reader
        .read_to_end(&mut reserved)
        .map_err(|_| invalid_value(field_name, "substream payload is truncated"))?;

    Ok((substreams, reserved))
}

/// AC-3 decoder configuration box carried by `ac-3` sample entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dac3 {
    pub fscod: u8,
    pub bsid: u8,
    pub bsmod: u8,
    pub acmod: u8,
    pub lfe_on: u8,
    pub bit_rate_code: u8,
}

impl FieldHooks for Dac3 {}

impl ImmutableBox for Dac3 {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"dac3")
    }
}

impl MutableBox for Dac3 {}

impl FieldValueRead for Dac3 {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Fscod" => Ok(FieldValue::Unsigned(u64::from(self.fscod))),
            "Bsid" => Ok(FieldValue::Unsigned(u64::from(self.bsid))),
            "Bsmod" => Ok(FieldValue::Unsigned(u64::from(self.bsmod))),
            "Acmod" => Ok(FieldValue::Unsigned(u64::from(self.acmod))),
            "LfeOn" => Ok(FieldValue::Unsigned(u64::from(self.lfe_on))),
            "BitRateCode" => Ok(FieldValue::Unsigned(u64::from(self.bit_rate_code))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Dac3 {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Fscod", FieldValue::Unsigned(value)) => {
                self.fscod = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Bsid", FieldValue::Unsigned(value)) => {
                self.bsid = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Bsmod", FieldValue::Unsigned(value)) => {
                self.bsmod = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Acmod", FieldValue::Unsigned(value)) => {
                self.acmod = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("LfeOn", FieldValue::Unsigned(value)) => {
                self.lfe_on = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("BitRateCode", FieldValue::Unsigned(value)) => {
                self.bit_rate_code = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Dac3 {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Fscod", 0, with_bit_width(2), as_hex()),
        codec_field!("Bsid", 1, with_bit_width(5), as_hex()),
        codec_field!("Bsmod", 2, with_bit_width(3), as_hex()),
        codec_field!("Acmod", 3, with_bit_width(3), as_hex()),
        codec_field!("LfeOn", 4, with_bit_width(1), as_hex()),
        codec_field!("BitRateCode", 5, with_bit_width(5), as_hex()),
        codec_field!("Reserved", 6, with_bit_width(5), with_constant("0")),
    ]);
}

/// E-AC-3 substream descriptor stored inside a `dec3` decoder configuration box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ec3Substream {
    pub fscod: u8,
    pub bsid: u8,
    pub asvc: u8,
    pub bsmod: u8,
    pub acmod: u8,
    pub lfe_on: u8,
    pub num_dep_sub: u8,
    pub chan_loc: u16,
}

/// E-AC-3 decoder configuration box carried by `ec-3` sample entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dec3 {
    pub data_rate: u16,
    pub num_ind_sub: u8,
    pub ec3_substreams: Vec<Ec3Substream>,
    pub reserved: Vec<u8>,
}

impl FieldHooks for Dec3 {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "EC3Subs" => Some(if self.reserved.is_empty() {
                render_ec3_substreams(&self.ec3_substreams)
            } else {
                format!(
                    "{} (Reserved={})",
                    render_ec3_substreams(&self.ec3_substreams),
                    format_bytes(&self.reserved)
                )
            }),
            _ => None,
        }
    }
}

impl ImmutableBox for Dec3 {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"dec3")
    }
}

impl MutableBox for Dec3 {}

impl FieldValueRead for Dec3 {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DataRate" => Ok(FieldValue::Unsigned(u64::from(self.data_rate))),
            "NumIndSub" => Ok(FieldValue::Unsigned(u64::from(self.num_ind_sub))),
            "EC3Subs" => Ok(FieldValue::Bytes(encode_ec3_substreams(
                field_name,
                self.num_ind_sub,
                &self.ec3_substreams,
                &self.reserved,
            )?)),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Dec3 {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DataRate", FieldValue::Unsigned(value)) => {
                self.data_rate = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("NumIndSub", FieldValue::Unsigned(value)) => {
                self.num_ind_sub = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("EC3Subs", FieldValue::Bytes(value)) => {
                let (ec3_substreams, reserved) =
                    parse_ec3_substreams(field_name, self.num_ind_sub, &value)?;
                self.ec3_substreams = ec3_substreams;
                self.reserved = reserved;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Dec3 {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("DataRate", 0, with_bit_width(13)),
        codec_field!("NumIndSub", 1, with_bit_width(3)),
        codec_field!("EC3Subs", 2, with_bit_width(8), as_bytes()),
    ]);
}

/// Registers the currently implemented ETSI TS 102 366 boxes in `registry`.
pub fn register_boxes(registry: &mut BoxRegistry) {
    registry.register_any::<AudioSampleEntry>(FourCc::from_bytes(*b"ac-3"));
    registry.register_any::<AudioSampleEntry>(FourCc::from_bytes(*b"ec-3"));
    registry.register::<Dac3>(FourCc::from_bytes(*b"dac3"));
    registry.register::<Dec3>(FourCc::from_bytes(*b"dec3"));
}
