use std::io::Cursor;

use mp4forge::boxes::{AnyTypeBox, BoxLookupContext, BoxRegistry};
use mp4forge::codec::{
    CodecBox, CodecError, FieldHooks, FieldTable, FieldValue, FieldValueError, FieldValueRead,
    FieldValueWrite, ImmutableBox, MutableBox, marshal, unmarshal_any, unmarshal_any_with_context,
};
use mp4forge::{FourCc, codec_field};

#[derive(Clone, Debug, PartialEq, Eq)]
struct RegistryBox {
    box_type: FourCc,
    version: u8,
    flags: u32,
    payload: u16,
}

impl Default for RegistryBox {
    fn default() -> Self {
        Self {
            box_type: FourCc::ANY,
            version: 0,
            flags: 0,
            payload: 0,
        }
    }
}

impl FieldHooks for RegistryBox {}

impl ImmutableBox for RegistryBox {
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

impl MutableBox for RegistryBox {
    fn set_version(&mut self, version: u8) {
        self.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.flags = flags;
    }
}

impl FieldValueRead for RegistryBox {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Payload" => Ok(FieldValue::Unsigned(u64::from(self.payload))),
            _ => Err(FieldValueError::MissingField { field_name }),
        }
    }
}

impl FieldValueWrite for RegistryBox {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Payload", FieldValue::Unsigned(value)) => {
                self.payload = u16::try_from(value).map_err(|_| FieldValueError::InvalidValue {
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

impl CodecBox for RegistryBox {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("Payload", 2, with_bit_width(16)),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0, 1];
}

impl AnyTypeBox for RegistryBox {
    fn set_box_type(&mut self, box_type: FourCc) {
        self.box_type = box_type;
    }
}

fn sample_any_box(box_type: FourCc) -> RegistryBox {
    RegistryBox {
        box_type,
        version: 1,
        flags: 0x000203,
        payload: 0x1234,
    }
}

fn is_under_wave(context: BoxLookupContext) -> bool {
    context.under_wave()
}

#[test]
fn registry_reports_supported_versions() {
    let box_type = FourCc::from_bytes(*b"regy");
    let mut registry = BoxRegistry::new();
    assert!(!registry.is_registered(box_type));

    registry.register::<RegistryBox>(box_type);

    assert!(registry.is_registered(box_type));
    assert_eq!(registry.supported_versions(box_type), Some(&[0, 1][..]));
    assert!(registry.is_supported_version(box_type, 0));
    assert!(registry.is_supported_version(box_type, 1));
    assert!(!registry.is_supported_version(box_type, 2));
    assert!(!registry.is_supported_version(FourCc::from_bytes(*b"miss"), 0));
}

#[test]
fn registry_contextual_registration_activates_only_in_matching_context() {
    let box_type = FourCc::from_bytes(*b"ctxf");
    let mut registry = BoxRegistry::new();
    let wave_context = BoxLookupContext::new().enter(FourCc::from_bytes(*b"wave"));

    registry.register_contextual::<RegistryBox>(box_type, is_under_wave);

    assert!(!registry.is_registered(box_type));
    assert!(!registry.is_registered_with_context(box_type, BoxLookupContext::new()));
    assert!(registry.is_registered_with_context(box_type, wave_context));
    assert_eq!(
        registry.supported_versions_with_context(box_type, wave_context),
        Some(&[0, 1][..])
    );
}

#[test]
fn unmarshal_any_uses_registered_any_type_constructor() {
    let box_type = FourCc::from_bytes(*b"dynx");
    let src = sample_any_box(box_type);
    let mut payload = Vec::new();
    let written = marshal(&mut payload, &src, None).unwrap();

    let mut registry = BoxRegistry::new();
    registry.register_any::<RegistryBox>(box_type);

    let mut cursor = Cursor::new(payload.clone());
    let (decoded, read) =
        unmarshal_any(&mut cursor, payload.len() as u64, box_type, &registry, None).unwrap();

    assert_eq!(read, written);
    let decoded = decoded.as_any().downcast_ref::<RegistryBox>().unwrap();
    assert_eq!(decoded, &src);
}

#[test]
fn unmarshal_any_uses_registered_contextual_any_type_constructor() {
    let box_type = FourCc::from_bytes(*b"ctxa");
    let src = sample_any_box(box_type);
    let mut payload = Vec::new();
    let written = marshal(&mut payload, &src, None).unwrap();
    let wave_context = BoxLookupContext::new().enter(FourCc::from_bytes(*b"wave"));

    let mut registry = BoxRegistry::new();
    registry.register_contextual_any::<RegistryBox>(box_type, is_under_wave);

    let mut missing_context_cursor = Cursor::new(payload.clone());
    match unmarshal_any_with_context(
        &mut missing_context_cursor,
        payload.len() as u64,
        box_type,
        &registry,
        BoxLookupContext::new(),
        None,
    ) {
        Err(CodecError::UnknownBoxType { box_type: actual }) => assert_eq!(actual, box_type),
        Ok(_) => panic!("unexpected success"),
        Err(other) => panic!("unexpected error: {other}"),
    }

    let mut wave_cursor = Cursor::new(payload);
    let (decoded, read) = unmarshal_any_with_context(
        &mut wave_cursor,
        written as u64,
        box_type,
        &registry,
        wave_context,
        None,
    )
    .unwrap();

    assert_eq!(read, written);
    let decoded = decoded.as_any().downcast_ref::<RegistryBox>().unwrap();
    assert_eq!(decoded, &src);
}

#[test]
fn unmarshal_any_rejects_unknown_box_types() {
    let mut cursor = Cursor::new(Vec::<u8>::new());
    let registry = BoxRegistry::new();
    let box_type = FourCc::from_bytes(*b"none");

    match unmarshal_any(&mut cursor, 0, box_type, &registry, None) {
        Err(CodecError::UnknownBoxType { box_type: actual }) => assert_eq!(actual, box_type),
        Ok(_) => panic!("unexpected success"),
        Err(other) => panic!("unexpected error: {other}"),
    }
}
