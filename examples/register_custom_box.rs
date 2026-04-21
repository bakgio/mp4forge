use std::error::Error;

use mp4forge::FourCc;
use mp4forge::boxes::{AnyTypeBox, BoxRegistry};
use mp4forge::codec::{
    CodecBox, FieldHooks, FieldTable, FieldValue, FieldValueError, FieldValueRead, FieldValueWrite,
    ImmutableBox, MutableBox, marshal,
};
use mp4forge::codec_field;
use mp4forge::stringify::{stringify, stringify_with_indent};

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExampleBox {
    box_type: FourCc,
    version: u8,
    flags: u32,
    ui32: u32,
    byte_array: Vec<u8>,
}

impl Default for ExampleBox {
    fn default() -> Self {
        Self {
            box_type: FourCc::ANY,
            version: 0,
            flags: 0,
            ui32: 0,
            byte_array: Vec::new(),
        }
    }
}

impl FieldHooks for ExampleBox {}

impl ImmutableBox for ExampleBox {
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

impl MutableBox for ExampleBox {
    fn set_version(&mut self, version: u8) {
        self.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.flags = flags;
    }
}

impl AnyTypeBox for ExampleBox {
    fn set_box_type(&mut self, box_type: FourCc) {
        self.box_type = box_type;
    }
}

impl FieldValueRead for ExampleBox {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "UI32" => Ok(FieldValue::Unsigned(u64::from(self.ui32))),
            "ByteArray" => Ok(FieldValue::Bytes(self.byte_array.clone())),
            _ => Err(FieldValueError::MissingField { field_name }),
        }
    }
}

impl FieldValueWrite for ExampleBox {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("UI32", FieldValue::Unsigned(value)) => {
                self.ui32 = u32::try_from(value).map_err(|_| FieldValueError::InvalidValue {
                    field_name,
                    reason: "value does not fit in u32",
                })?;
                Ok(())
            }
            ("ByteArray", FieldValue::Bytes(value)) => {
                self.byte_array = value;
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

impl CodecBox for ExampleBox {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("UI32", 2, with_bit_width(32)),
        codec_field!("ByteArray", 3, with_bit_width(8), as_bytes()),
    ]);
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let box_type = FourCc::from_bytes(*b"xxxx");
    let mut registry = BoxRegistry::new();
    registry.register_any::<ExampleBox>(box_type);

    let src = ExampleBox {
        box_type,
        version: 0,
        flags: 0x000001,
        ui32: 0x0102_0304,
        byte_array: vec![0xaa, 0xbb, 0xcc],
    };

    let mut encoded = Vec::new();
    marshal(&mut encoded, &src, None)?;

    println!("type {}", box_type);
    println!("registered {}", registry.is_registered(box_type));
    println!("single {}", stringify(&src, None)?);
    println!(
        "indent {}",
        stringify_with_indent(&src, "  ", None)?
            .trim_end_matches('\n')
            .replace('\n', "\\n")
    );
    println!("encoded {}", bytes_to_hex(&encoded));

    Ok(())
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut out = String::new();
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}
