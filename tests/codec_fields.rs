use std::collections::BTreeMap;

use mp4forge::codec::{
    ANY_VERSION, FieldDescriptor, FieldHooks, FieldResolutionError, FieldTable, ImmutableBox,
    MutableBox, ResolvedFieldLength,
};
use mp4forge::{FourCc, codec_field};

#[derive(Clone, Debug, Default)]
struct HookState {
    sizes: BTreeMap<&'static str, u32>,
    lengths: BTreeMap<&'static str, u32>,
    enabled: BTreeMap<&'static str, bool>,
}

impl FieldHooks for HookState {
    fn field_size(&self, name: &'static str) -> Option<u32> {
        self.sizes.get(name).copied()
    }

    fn field_length(&self, name: &'static str) -> Option<u32> {
        self.lengths.get(name).copied()
    }

    fn field_enabled(&self, name: &'static str) -> Option<bool> {
        self.enabled.get(name).copied()
    }
}

#[derive(Clone, Debug)]
struct MockBox {
    box_type: FourCc,
    version: u8,
    flags: u32,
    hooks: HookState,
}

impl Default for MockBox {
    fn default() -> Self {
        Self {
            box_type: FourCc::from_bytes(*b"test"),
            version: ANY_VERSION,
            flags: 0,
            hooks: HookState::default(),
        }
    }
}

impl FieldHooks for MockBox {
    fn field_size(&self, name: &'static str) -> Option<u32> {
        self.hooks.field_size(name)
    }

    fn field_length(&self, name: &'static str) -> Option<u32> {
        self.hooks.field_length(name)
    }

    fn field_enabled(&self, name: &'static str) -> Option<bool> {
        self.hooks.field_enabled(name)
    }
}

impl ImmutableBox for MockBox {
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

impl MutableBox for MockBox {
    fn set_version(&mut self, version: u8) {
        self.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.flags = flags;
    }
}

#[test]
fn field_table_orders_descriptors_by_explicit_order() {
    const FIELDS: &[FieldDescriptor] = &[
        codec_field!("NotSorted22", 22, with_bit_width(8)),
        codec_field!("NotSorted23", 23, with_bit_width(8)),
        codec_field!("NotSorted21", 21, with_bit_width(8)),
    ];

    let table = FieldTable::new(FIELDS);
    let ordered_names = table
        .ordered()
        .into_iter()
        .map(|field| field.name)
        .collect::<Vec<_>>();

    assert_eq!(
        ordered_names,
        vec!["NotSorted21", "NotSorted22", "NotSorted23"]
    );
}

#[test]
fn field_table_applies_version_flag_and_dynamic_presence_gates() {
    const FIELDS: &[FieldDescriptor] = &[
        codec_field!("Always", 0, with_bit_width(8)),
        codec_field!("Version1", 1, with_bit_width(8), with_version(1)),
        codec_field!("NotVersion1", 2, with_bit_width(8), without_version(1)),
        codec_field!(
            "NeedFlag2",
            3,
            with_bit_width(8),
            with_required_flags(0x000002)
        ),
        codec_field!(
            "NeedFlag8",
            4,
            with_bit_width(8),
            with_required_flags(0x000008)
        ),
        codec_field!(
            "RejectFlag8",
            5,
            with_bit_width(8),
            with_forbidden_flags(0x000008)
        ),
        codec_field!("DynEnabled", 6, with_bit_width(8), with_dynamic_presence()),
        codec_field!("DynDisabled", 7, with_bit_width(8), with_dynamic_presence()),
    ];

    let mut box_ref = MockBox::default();
    box_ref.set_version(1);
    box_ref.set_flags(0x000006);
    box_ref.hooks.enabled.insert("DynEnabled", true);
    box_ref.hooks.enabled.insert("DynDisabled", false);

    let resolved = FieldTable::new(FIELDS)
        .resolve_active(&box_ref, None)
        .unwrap();
    let names = resolved
        .iter()
        .map(|field| field.name())
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        vec![
            "Always",
            "Version1",
            "NeedFlag2",
            "RejectFlag8",
            "DynEnabled"
        ]
    );
}

#[test]
fn dynamic_size_and_length_can_be_resolved_from_override_hooks() {
    const FIELDS: &[FieldDescriptor] = &[
        codec_field!("DynSize", 0, with_dynamic_bit_width()),
        codec_field!("DynLen", 1, with_bit_width(8), with_dynamic_length()),
    ];

    let mut owner = MockBox::default();
    owner.hooks.sizes.insert("DynSize", 16);
    owner.hooks.lengths.insert("DynLen", 2);

    let mut override_hooks = HookState::default();
    override_hooks.sizes.insert("DynSize", 32);
    override_hooks.lengths.insert("DynLen", 4);

    let resolved = FieldTable::new(FIELDS)
        .resolve_active(&owner, Some(&override_hooks))
        .unwrap();

    assert_eq!(resolved[0].bit_width, Some(32));
    assert_eq!(resolved[1].length, ResolvedFieldLength::Fixed(4));
}

#[test]
fn dynamic_size_and_length_require_hook_values() {
    const FIELDS: &[FieldDescriptor] = &[
        codec_field!("DynSize", 0, with_dynamic_bit_width()),
        codec_field!("DynLen", 1, with_bit_width(8), with_dynamic_length()),
    ];

    let error = FieldTable::new(FIELDS)
        .resolve_active(&MockBox::default(), None)
        .unwrap_err();

    assert_eq!(
        error,
        FieldResolutionError::MissingDynamicBitWidth {
            field_name: "DynSize"
        }
    );
}

#[test]
fn mutable_box_helper_methods_update_flags() {
    let mut box_ref = MockBox::default();

    box_ref.set_flags(0x000002);
    box_ref.add_flag(0x000004);
    assert_eq!(box_ref.flags(), 0x000006);
    assert!(box_ref.check_flag(0x000004));

    box_ref.remove_flag(0x000002);
    assert_eq!(box_ref.flags(), 0x000004);
}
