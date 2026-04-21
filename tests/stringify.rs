use mp4forge::codec::{
    ANY_VERSION, CodecBox, FieldHooks, FieldTable, FieldValue, FieldValueError, FieldValueRead,
    FieldValueWrite, ImmutableBox, MutableBox,
};
use mp4forge::stringify::{stringify, stringify_with_indent};
use mp4forge::{FourCc, codec_field};

#[derive(Clone, Debug, PartialEq, Eq)]
struct DisplayBox {
    box_type: FourCc,
    version: u8,
    flags: u32,
    title: String,
    language: Vec<u64>,
    delta: i16,
    numbers: Vec<u16>,
    uuid: [u8; 16],
    data: Vec<u8>,
    hidden: u8,
}

impl Default for DisplayBox {
    fn default() -> Self {
        Self {
            box_type: FourCc::from_bytes(*b"disp"),
            version: ANY_VERSION,
            flags: 0,
            title: String::new(),
            language: Vec::new(),
            delta: 0,
            numbers: Vec::new(),
            uuid: [0; 16],
            data: Vec::new(),
            hidden: 0,
        }
    }
}

impl FieldHooks for DisplayBox {}

impl ImmutableBox for DisplayBox {
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

impl MutableBox for DisplayBox {
    fn set_version(&mut self, version: u8) {
        self.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.flags = flags;
    }
}

impl FieldValueRead for DisplayBox {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Title" => Ok(FieldValue::String(self.title.clone())),
            "Language" => Ok(FieldValue::UnsignedArray(self.language.clone())),
            "Delta" => Ok(FieldValue::Signed(i64::from(self.delta))),
            "Numbers" => Ok(FieldValue::UnsignedArray(
                self.numbers.iter().copied().map(u64::from).collect(),
            )),
            "Uuid" => Ok(FieldValue::Bytes(self.uuid.to_vec())),
            "Data" => Ok(FieldValue::Bytes(self.data.clone())),
            "Hidden" => Ok(FieldValue::Unsigned(u64::from(self.hidden))),
            _ => Err(FieldValueError::MissingField { field_name }),
        }
    }
}

impl FieldValueWrite for DisplayBox {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Title", FieldValue::String(value)) => {
                self.title = value;
                Ok(())
            }
            ("Language", FieldValue::UnsignedArray(value)) => {
                self.language = value;
                Ok(())
            }
            ("Delta", FieldValue::Signed(value)) => {
                self.delta = i16::try_from(value).map_err(|_| FieldValueError::InvalidValue {
                    field_name,
                    reason: "value does not fit in i16",
                })?;
                Ok(())
            }
            ("Numbers", FieldValue::UnsignedArray(values)) => {
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
                Ok(())
            }
            ("Uuid", FieldValue::Bytes(value)) => {
                self.uuid = value
                    .try_into()
                    .map_err(|_| FieldValueError::InvalidValue {
                        field_name,
                        reason: "value must be 16 bytes",
                    })?;
                Ok(())
            }
            ("Data", FieldValue::Bytes(value)) => {
                self.data = value;
                Ok(())
            }
            ("Hidden", FieldValue::Unsigned(value)) => {
                self.hidden = u8::try_from(value).map_err(|_| FieldValueError::InvalidValue {
                    field_name,
                    reason: "value does not fit in u8",
                })?;
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

impl CodecBox for DisplayBox {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!(
            "Title",
            2,
            with_bit_width(8),
            as_string(mp4forge::codec::StringFieldMode::NullTerminated)
        ),
        codec_field!(
            "Language",
            3,
            with_bit_width(5),
            with_length(3),
            as_iso639_2()
        ),
        codec_field!("Delta", 4, with_bit_width(16), as_signed(), as_hex()),
        codec_field!("Numbers", 5, with_bit_width(16), with_length(2)),
        codec_field!(
            "Uuid",
            6,
            with_bit_width(8),
            with_length(16),
            as_bytes(),
            as_uuid()
        ),
        codec_field!("Data", 7, with_bit_width(8), with_length(3), as_bytes()),
        codec_field!("Hidden", 8, with_bit_width(8), as_hidden()),
    ]);
}

fn display_box() -> DisplayBox {
    DisplayBox {
        version: 1,
        flags: 0x000203,
        title: "rust\nforge".into(),
        language: vec![5, 14, 7],
        delta: -0x1234,
        numbers: vec![1, 0x1234],
        uuid: [
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef,
        ],
        data: vec![0x41, 0x00, 0x7f],
        hidden: 0xff,
        ..DisplayBox::default()
    }
}

#[test]
fn stringify_renders_descriptor_order_and_formats() {
    let rendered = stringify(&display_box(), None).unwrap();

    assert_eq!(
        rendered,
        "Version=1 Flags=0x000203 Title=\"rust.forge\" Language=\"eng\" Delta=-0x1234 Numbers=[1, 4660] Uuid=01234567-89ab-cdef-0123-456789abcdef Data=[0x41, 0x0, 0x7f]"
    );
}

#[test]
fn stringify_with_indent_emits_one_field_per_line() {
    let rendered = stringify_with_indent(&display_box(), "  ", None).unwrap();

    assert_eq!(
        rendered,
        concat!(
            "  Version=1\n",
            "  Flags=0x000203\n",
            "  Title=\"rust.forge\"\n",
            "  Language=\"eng\"\n",
            "  Delta=-0x1234\n",
            "  Numbers=[1, 4660]\n",
            "  Uuid=01234567-89ab-cdef-0123-456789abcdef\n",
            "  Data=[0x41, 0x0, 0x7f]\n",
        )
    );
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct OverrideBox {
    value: u16,
}

impl FieldHooks for OverrideBox {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Value" => Some(format!("override({})", self.value)),
            _ => None,
        }
    }
}

impl ImmutableBox for OverrideBox {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"ovrd")
    }
}

impl MutableBox for OverrideBox {}

impl FieldValueRead for OverrideBox {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Value" => Ok(FieldValue::Unsigned(u64::from(self.value))),
            _ => Err(FieldValueError::MissingField { field_name }),
        }
    }
}

impl FieldValueWrite for OverrideBox {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Value", FieldValue::Unsigned(value)) => {
                self.value = u16::try_from(value).map_err(|_| FieldValueError::InvalidValue {
                    field_name,
                    reason: "value does not fit in u16",
                })?;
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

impl CodecBox for OverrideBox {
    const FIELD_TABLE: FieldTable =
        FieldTable::new(&[codec_field!("Value", 0, with_bit_width(16))]);
}

#[test]
fn stringify_uses_field_display_override_when_available() {
    let rendered = stringify(&OverrideBox { value: 7 }, None).unwrap();

    assert_eq!(rendered, "Value=override(7)");
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct DisplayOrderBox {
    early: u16,
    late: u16,
}

impl FieldHooks for DisplayOrderBox {}

impl ImmutableBox for DisplayOrderBox {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"dord")
    }
}

impl MutableBox for DisplayOrderBox {}

impl FieldValueRead for DisplayOrderBox {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Early" => Ok(FieldValue::Unsigned(u64::from(self.early))),
            "Late" => Ok(FieldValue::Unsigned(u64::from(self.late))),
            _ => Err(FieldValueError::MissingField { field_name }),
        }
    }
}

impl FieldValueWrite for DisplayOrderBox {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Early", FieldValue::Unsigned(value)) => {
                self.early = u16::try_from(value).map_err(|_| FieldValueError::InvalidValue {
                    field_name,
                    reason: "value does not fit in u16",
                })?;
                Ok(())
            }
            ("Late", FieldValue::Unsigned(value)) => {
                self.late = u16::try_from(value).map_err(|_| FieldValueError::InvalidValue {
                    field_name,
                    reason: "value does not fit in u16",
                })?;
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

impl CodecBox for DisplayOrderBox {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Late", 0, with_bit_width(16), with_display_order(1)),
        codec_field!("Early", 1, with_bit_width(16), with_display_order(0)),
    ]);
}

#[test]
fn stringify_can_override_display_order_without_changing_wire_order() {
    let rendered = stringify(&DisplayOrderBox { early: 1, late: 2 }, None).unwrap();

    assert_eq!(rendered, "Early=1 Late=2");
}
