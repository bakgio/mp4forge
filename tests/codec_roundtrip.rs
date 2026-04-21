use std::collections::BTreeMap;
use std::io::{Cursor, Seek};

use mp4forge::codec::{
    ANY_VERSION, CodecBox, CodecError, FieldHooks, FieldTable, FieldValue, FieldValueError,
    FieldValueRead, FieldValueWrite, ImmutableBox, MutableBox, StringFieldMode, marshal, unmarshal,
};
use mp4forge::{FourCc, codec_field};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct HookState {
    lengths: BTreeMap<&'static str, u32>,
    pascal_strings: BTreeMap<&'static str, bool>,
    consume_remaining_strings: BTreeMap<&'static str, bool>,
}

impl FieldHooks for HookState {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        self.lengths.get(name).copied()
    }

    fn is_pascal_string(
        &self,
        name: &'static str,
        _data: &[u8],
        _remaining_bytes: u64,
    ) -> Option<bool> {
        self.pascal_strings.get(name).copied()
    }

    fn consume_remaining_bytes_after_string(&self, name: &'static str) -> Option<bool> {
        self.consume_remaining_strings.get(name).copied()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SampleBox {
    box_type: FourCc,
    version: u8,
    flags: u32,
    version_one_only: u8,
    counter: u16,
    signed_delta: i16,
    enabled: bool,
    varint: u64,
    name: String,
    alias: String,
    raw: String,
    numbers: Vec<u16>,
    hooks: HookState,
}

impl Default for SampleBox {
    fn default() -> Self {
        Self {
            box_type: FourCc::from_bytes(*b"test"),
            version: ANY_VERSION,
            flags: 0,
            version_one_only: 0,
            counter: 0,
            signed_delta: 0,
            enabled: false,
            varint: 0,
            name: String::new(),
            alias: String::new(),
            raw: String::new(),
            numbers: Vec::new(),
            hooks: HookState::default(),
        }
    }
}

impl FieldHooks for SampleBox {
    fn field_length(&self, name: &'static str) -> Option<u32> {
        self.hooks.field_length(name)
    }

    fn is_pascal_string(
        &self,
        name: &'static str,
        data: &[u8],
        remaining_bytes: u64,
    ) -> Option<bool> {
        self.hooks.is_pascal_string(name, data, remaining_bytes)
    }

    fn consume_remaining_bytes_after_string(&self, name: &'static str) -> Option<bool> {
        self.hooks.consume_remaining_bytes_after_string(name)
    }
}

impl ImmutableBox for SampleBox {
    fn box_type(&self) -> FourCc {
        self.box_type
    }

    fn version(&self) -> u8 {
        self.version
    }

    fn flags(&self) -> u32 {
        self.flags
    }
}

impl MutableBox for SampleBox {
    fn set_version(&mut self, version: u8) {
        self.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.flags = flags;
    }
}

impl FieldValueRead for SampleBox {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "version_one_only" => Ok(FieldValue::Unsigned(u64::from(self.version_one_only))),
            "counter" => Ok(FieldValue::Unsigned(u64::from(self.counter))),
            "signed_delta" => Ok(FieldValue::Signed(i64::from(self.signed_delta))),
            "enabled" => Ok(FieldValue::Boolean(self.enabled)),
            "varint" => Ok(FieldValue::Unsigned(self.varint)),
            "name" => Ok(FieldValue::String(self.name.clone())),
            "alias" => Ok(FieldValue::String(self.alias.clone())),
            "raw" => Ok(FieldValue::String(self.raw.clone())),
            "numbers" => Ok(FieldValue::UnsignedArray(
                self.numbers.iter().copied().map(u64::from).collect(),
            )),
            _ => Err(FieldValueError::MissingField { field_name }),
        }
    }
}

impl FieldValueWrite for SampleBox {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("version_one_only", FieldValue::Unsigned(value)) => {
                self.version_one_only =
                    u8::try_from(value).map_err(|_| FieldValueError::InvalidValue {
                        field_name,
                        reason: "value does not fit in u8",
                    })?;
            }
            ("counter", FieldValue::Unsigned(value)) => {
                self.counter = u16::try_from(value).map_err(|_| FieldValueError::InvalidValue {
                    field_name,
                    reason: "value does not fit in u16",
                })?;
            }
            ("signed_delta", FieldValue::Signed(value)) => {
                self.signed_delta =
                    i16::try_from(value).map_err(|_| FieldValueError::InvalidValue {
                        field_name,
                        reason: "value does not fit in i16",
                    })?;
            }
            ("enabled", FieldValue::Boolean(value)) => self.enabled = value,
            ("varint", FieldValue::Unsigned(value)) => self.varint = value,
            ("name", FieldValue::String(value)) => self.name = value,
            ("alias", FieldValue::String(value)) => self.alias = value,
            ("raw", FieldValue::String(value)) => self.raw = value,
            ("numbers", FieldValue::UnsignedArray(values)) => {
                let mut numbers = Vec::with_capacity(values.len());
                for value in values {
                    numbers.push(u16::try_from(value).map_err(|_| {
                        FieldValueError::InvalidValue {
                            field_name,
                            reason: "value does not fit in u16",
                        }
                    })?);
                }
                self.numbers = numbers;
            }
            (field_name, value) => {
                return Err(FieldValueError::UnexpectedType {
                    field_name,
                    expected: "matching codec field value",
                    actual: value.kind_name(),
                });
            }
        }

        Ok(())
    }
}

impl CodecBox for SampleBox {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("version", 0, with_bit_width(8), as_version_field()),
        codec_field!("flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("version_one_only", 2, with_bit_width(8), with_version(1)),
        codec_field!("counter", 3, with_bit_width(16)),
        codec_field!("signed_delta", 4, with_bit_width(16), as_signed()),
        codec_field!("enabled", 5, with_bit_width(1), as_boolean()),
        codec_field!("padding", 6, with_bit_width(7), with_constant("0")),
        codec_field!("varint", 7, as_varint()),
        codec_field!(
            "name",
            8,
            with_bit_width(8),
            as_string(StringFieldMode::NullTerminated)
        ),
        codec_field!(
            "alias",
            9,
            with_bit_width(8),
            as_string(StringFieldMode::PascalCompatible)
        ),
        codec_field!(
            "raw",
            10,
            with_bit_width(8),
            with_dynamic_length(),
            as_string(StringFieldMode::RawBox)
        ),
        codec_field!("numbers", 11, with_bit_width(16), with_dynamic_length()),
    ]);

    fn is_supported_version(&self, version: u8) -> bool {
        matches!(version, 0 | 1)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct TerminalPascalStringBox {
    alias: String,
    hooks: HookState,
}

impl FieldHooks for TerminalPascalStringBox {
    fn is_pascal_string(
        &self,
        name: &'static str,
        data: &[u8],
        remaining_bytes: u64,
    ) -> Option<bool> {
        self.hooks.is_pascal_string(name, data, remaining_bytes)
    }

    fn consume_remaining_bytes_after_string(&self, name: &'static str) -> Option<bool> {
        self.hooks.consume_remaining_bytes_after_string(name)
    }
}

impl ImmutableBox for TerminalPascalStringBox {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"term")
    }
}

impl MutableBox for TerminalPascalStringBox {}

impl FieldValueRead for TerminalPascalStringBox {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "alias" => Ok(FieldValue::String(self.alias.clone())),
            _ => Err(FieldValueError::MissingField { field_name }),
        }
    }
}

impl FieldValueWrite for TerminalPascalStringBox {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("alias", FieldValue::String(value)) => {
                self.alias = value;
                Ok(())
            }
            (field_name, value) => Err(FieldValueError::UnexpectedType {
                field_name,
                expected: "matching codec field value",
                actual: value.kind_name(),
            }),
        }
    }
}

impl CodecBox for TerminalPascalStringBox {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[codec_field!(
        "alias",
        0,
        with_bit_width(8),
        as_string(StringFieldMode::PascalCompatible)
    )]);
}

fn sample_box() -> SampleBox {
    let mut hooks = HookState::default();
    hooks.lengths.insert("raw", 4);
    hooks.lengths.insert("numbers", 2);

    SampleBox {
        version: 1,
        flags: 0x000203,
        version_one_only: 0x77,
        counter: 0x1234,
        signed_delta: -2,
        enabled: true,
        varint: 0x1234,
        name: "rust".into(),
        alias: "forge".into(),
        raw: "ABCD".into(),
        numbers: vec![0x0001, 0x1234],
        hooks,
        ..SampleBox::default()
    }
}

#[test]
fn marshal_and_unmarshal_descriptor_driven_box() {
    let src = sample_box();
    let expected = vec![
        0x01, 0x00, 0x02, 0x03, 0x77, 0x12, 0x34, 0xff, 0xfe, 0x80, 0x80, 0x80, 0xa4, 0x34, b'r',
        b'u', b's', b't', 0x00, b'f', b'o', b'r', b'g', b'e', 0x00, b'A', b'B', b'C', b'D', 0x00,
        0x01, 0x12, 0x34,
    ];

    let mut encoded = Vec::new();
    let written = marshal(&mut encoded, &src, None).unwrap();
    assert_eq!(written, expected.len() as u64);
    assert_eq!(encoded, expected);

    let mut decoded = SampleBox {
        hooks: src.hooks.clone(),
        ..SampleBox::default()
    };
    let mut cursor = Cursor::new(expected);
    let payload_len = cursor.get_ref().len() as u64;
    let read = unmarshal(&mut cursor, payload_len, &mut decoded, None).unwrap();
    assert_eq!(read, payload_len);
    assert_eq!(decoded, src);
}

#[test]
fn pascal_compatible_string_can_switch_modes_during_decode() {
    let payload = vec![
        0x01, 0x00, 0x00, 0x03, 0x77, 0x12, 0x34, 0xff, 0xfe, 0x80, 0x80, 0x80, 0xa4, 0x34, b'h',
        b'e', b'l', b'l', b'o', 0x00, 0x05, b'f', b'o', b'r', b'g', b'e', b'W', b'X', b'Y', b'Z',
        0x00, 0x05, 0x00, 0x06,
    ];

    let mut hooks = HookState::default();
    hooks.lengths.insert("raw", 4);
    hooks.lengths.insert("numbers", 2);
    hooks.pascal_strings.insert("alias", true);

    let mut decoded = SampleBox {
        hooks,
        ..SampleBox::default()
    };
    let mut cursor = Cursor::new(payload.clone());
    let read = unmarshal(&mut cursor, payload.len() as u64, &mut decoded, None).unwrap();

    assert_eq!(read, payload.len() as u64);
    assert_eq!(decoded.version, 1);
    assert_eq!(decoded.flags, 0x000003);
    assert_eq!(decoded.version_one_only, 0x77);
    assert_eq!(decoded.name, "hello");
    assert_eq!(decoded.alias, "forge");
    assert_eq!(decoded.raw, "WXYZ");
    assert_eq!(decoded.numbers, vec![5, 6]);
}

#[test]
fn pascal_compatible_string_can_consume_remaining_padding_when_requested() {
    let payload = vec![b'f', b'o', b'r', b'g', b'e', 0x00, 0x00];

    let mut hooks = HookState::default();
    hooks.pascal_strings.insert("alias", false);
    hooks.consume_remaining_strings.insert("alias", true);

    let mut decoded = TerminalPascalStringBox {
        hooks,
        ..TerminalPascalStringBox::default()
    };
    let mut cursor = Cursor::new(payload.clone());
    let read = unmarshal(&mut cursor, payload.len() as u64, &mut decoded, None).unwrap();

    assert_eq!(read, payload.len() as u64);
    assert_eq!(decoded.alias, "forge");
}

#[test]
fn unsupported_version_rolls_back_stream_position_and_state() {
    let payload = vec![0x03, 0x00, 0x00, 0x03, 0xaa, 0xbb, 0xcc];
    let mut cursor = Cursor::new(payload.clone());
    let mut dst = SampleBox::default();
    dst.set_version(0);
    dst.set_flags(0x000111);

    let error = unmarshal(&mut cursor, payload.len() as u64, &mut dst, None).unwrap_err();

    match error {
        CodecError::UnsupportedVersion { box_type, version } => {
            assert_eq!(box_type, FourCc::from_bytes(*b"test"));
            assert_eq!(version, 3);
        }
        other => panic!("unexpected error: {other}"),
    }

    assert_eq!(cursor.stream_position().unwrap(), 0);
    assert_eq!(dst.version(), 0);
    assert_eq!(dst.flags(), 0x000111);
    assert_eq!(dst.counter, 0);
    assert!(dst.name.is_empty());
}
