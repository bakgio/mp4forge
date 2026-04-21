//! Box definitions and box-specific codecs.

use std::collections::BTreeMap;

use crate::FourCc;
use crate::codec::{CodecBox, DynCodecBox};

/// AV1 sample-entry and codec-configuration box definitions.
pub mod av1;
/// ETSI TS 102 366 AC-3 sample-entry and decoder-configuration box definitions.
pub mod etsi_ts_102_366;
/// ISO/IEC 14496-12 box definitions and codec support.
pub mod iso14496_12;
/// ISO/IEC 14496-14 ES descriptor box definitions and codec support.
pub mod iso14496_14;
/// ISO/IEC 14496-30 WebVTT box definitions and codec support.
pub mod iso14496_30;
/// ISO/IEC 23001-5 uncompressed-audio box definitions and codec support.
pub mod iso23001_5;
/// ISO/IEC 23001-7 common-encryption box definitions and codec support.
pub mod iso23001_7;
/// Item-list metadata and key-table box definitions.
pub mod metadata;
/// Opus sample-entry and decoder-configuration box definitions.
pub mod opus;
/// 3GPP `udta`-scoped metadata string box definitions and codec support.
pub mod threegpp;
/// VP8/VP9 sample-entry and codec-configuration box definitions.
pub mod vp;

/// Trait implemented by runtime-typed box wrappers whose `FourCc` is supplied by the registry.
pub trait AnyTypeBox {
    /// Sets the concrete box type chosen by the registry.
    fn set_box_type(&mut self, box_type: FourCc);
}

type BoxConstructor = fn(FourCc) -> Box<dyn DynCodecBox>;
type ContextPredicate = fn(BoxLookupContext) -> bool;
type DynamicContextPredicate = fn(FourCc, BoxLookupContext) -> bool;

/// Parent-scope state used when selecting context-sensitive box registrations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BoxLookupContext {
    pub(crate) is_quicktime_compatible: bool,
    pub(crate) quicktime_keys_meta_entry_count: usize,
    pub(crate) ilst_meta_item: Option<FourCc>,
    pub(crate) under_wave: bool,
    pub(crate) under_ilst: bool,
    pub(crate) under_ilst_meta: bool,
    pub(crate) under_ilst_free_meta: bool,
    pub(crate) under_udta: bool,
}

impl BoxLookupContext {
    /// Creates an empty lookup context with no active parent scopes.
    pub const fn new() -> Self {
        Self {
            is_quicktime_compatible: false,
            quicktime_keys_meta_entry_count: 0,
            ilst_meta_item: None,
            under_wave: false,
            under_ilst: false,
            under_ilst_meta: false,
            under_ilst_free_meta: false,
            under_udta: false,
        }
    }

    /// Returns `true` when the file-level compatible-brand scan found the QuickTime brand.
    pub const fn is_quicktime_compatible(&self) -> bool {
        self.is_quicktime_compatible
    }

    /// Carries the QuickTime-compatible root flag into later child lookups.
    pub const fn with_quicktime_compatible(mut self, is_quicktime_compatible: bool) -> Self {
        self.is_quicktime_compatible = is_quicktime_compatible;
        self
    }

    /// Carries the parsed `keys` entry count into later numbered-item lookups.
    pub const fn with_metadata_keys_entry_count(
        mut self,
        quicktime_keys_meta_entry_count: usize,
    ) -> Self {
        self.quicktime_keys_meta_entry_count = quicktime_keys_meta_entry_count;
        self
    }

    /// Returns the current numbered-item upper bound learned from `keys`.
    pub const fn metadata_keys_entry_count(&self) -> usize {
        self.quicktime_keys_meta_entry_count
    }

    /// Returns the active item-list metadata identifier when walking under an `ilst` item box.
    pub const fn ilst_meta_item(&self) -> Option<FourCc> {
        self.ilst_meta_item
    }

    /// Returns `true` when the current lookup runs under a `wave` box.
    pub const fn under_wave(&self) -> bool {
        self.under_wave
    }

    /// Returns `true` when the current lookup runs under an `ilst` box.
    pub const fn under_ilst(&self) -> bool {
        self.under_ilst
    }

    /// Returns `true` when the current lookup runs under an `ilst` item container.
    pub const fn under_ilst_meta(&self) -> bool {
        self.under_ilst_meta
    }

    /// Returns `true` when the current lookup runs under a free-form `----` item container.
    pub const fn under_ilst_free_meta(&self) -> bool {
        self.under_ilst_free_meta
    }

    /// Returns `true` when the current lookup runs under a `udta` box.
    pub const fn under_udta(&self) -> bool {
        self.under_udta
    }

    /// Returns the child-lookup context that applies after entering `box_type`.
    pub fn enter(mut self, box_type: FourCc) -> Self {
        const WAVE: FourCc = FourCc::from_bytes(*b"wave");
        const ILST: FourCc = FourCc::from_bytes(*b"ilst");
        const UDTA: FourCc = FourCc::from_bytes(*b"udta");
        const FREE_FORM: FourCc = FourCc::from_bytes(*b"----");

        if box_type == WAVE {
            self.under_wave = true;
        } else if box_type == ILST {
            self.ilst_meta_item = None;
            self.under_ilst = true;
        } else if self.under_ilst
            && !self.under_ilst_meta
            && metadata::is_ilst_meta_box_type(box_type)
        {
            self.ilst_meta_item = Some(box_type);
            self.under_ilst_meta = true;
            if box_type == FREE_FORM {
                self.under_ilst_free_meta = true;
            }
        } else if box_type == UDTA {
            self.under_udta = true;
        }

        self
    }
}

/// Registry entry for a single supported box type.
#[derive(Clone, Copy)]
pub struct BoxRegistration {
    box_type: FourCc,
    supported_versions: &'static [u8],
    constructor: BoxConstructor,
}

impl BoxRegistration {
    fn new(
        box_type: FourCc,
        supported_versions: &'static [u8],
        constructor: BoxConstructor,
    ) -> Self {
        Self {
            box_type,
            supported_versions,
            constructor,
        }
    }

    /// Returns the registered four-character box type.
    pub const fn box_type(&self) -> FourCc {
        self.box_type
    }

    /// Returns the supported versions recorded for the box type.
    pub const fn supported_versions(&self) -> &'static [u8] {
        self.supported_versions
    }
}

#[derive(Clone, Copy)]
struct ContextualBoxRegistration {
    registration: BoxRegistration,
    matches: ContextPredicate,
}

impl ContextualBoxRegistration {
    const fn new(registration: BoxRegistration, matches: ContextPredicate) -> Self {
        Self {
            registration,
            matches,
        }
    }
}

#[derive(Clone, Copy)]
struct DynamicBoxRegistration {
    supported_versions: &'static [u8],
    constructor: BoxConstructor,
    matches: DynamicContextPredicate,
}

impl DynamicBoxRegistration {
    const fn new(
        supported_versions: &'static [u8],
        constructor: BoxConstructor,
        matches: DynamicContextPredicate,
    ) -> Self {
        Self {
            supported_versions,
            constructor,
            matches,
        }
    }
}

#[derive(Clone, Copy)]
struct ResolvedRegistration {
    supported_versions: &'static [u8],
    constructor: BoxConstructor,
}

impl ResolvedRegistration {
    const fn new(supported_versions: &'static [u8], constructor: BoxConstructor) -> Self {
        Self {
            supported_versions,
            constructor,
        }
    }
}

/// Registry that maps box identifiers to descriptor-backed constructors.
#[derive(Default)]
pub struct BoxRegistry {
    entries: BTreeMap<FourCc, BoxRegistration>,
    contextual_entries: BTreeMap<FourCc, Vec<ContextualBoxRegistration>>,
    dynamic_entries: Vec<DynamicBoxRegistration>,
}

impl BoxRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` when a constructor has been registered for `box_type`.
    pub fn is_registered(&self, box_type: FourCc) -> bool {
        self.is_registered_with_context(box_type, BoxLookupContext::new())
    }

    /// Returns `true` when `box_type` has an active constructor in `context`.
    pub fn is_registered_with_context(&self, box_type: FourCc, context: BoxLookupContext) -> bool {
        self.resolve_registration(box_type, context).is_some()
    }

    /// Returns the supported-version list for a registered box type.
    pub fn supported_versions(&self, box_type: FourCc) -> Option<&'static [u8]> {
        self.supported_versions_with_context(box_type, BoxLookupContext::new())
    }

    /// Returns the supported-version list for a registration active in `context`.
    pub fn supported_versions_with_context(
        &self,
        box_type: FourCc,
        context: BoxLookupContext,
    ) -> Option<&'static [u8]> {
        self.resolve_registration(box_type, context)
            .map(|registration| registration.supported_versions)
    }

    /// Returns `true` when `version` is accepted for the registered box type.
    pub fn is_supported_version(&self, box_type: FourCc, version: u8) -> bool {
        self.is_supported_version_with_context(box_type, version, BoxLookupContext::new())
    }

    /// Returns `true` when `version` is accepted for the registration active in `context`.
    pub fn is_supported_version_with_context(
        &self,
        box_type: FourCc,
        version: u8,
        context: BoxLookupContext,
    ) -> bool {
        let Some(supported_versions) = self.supported_versions_with_context(box_type, context)
        else {
            return false;
        };

        supported_versions.is_empty() || supported_versions.contains(&version)
    }

    /// Registers a fixed-type box whose runtime `FourCc` always matches the type parameter.
    pub fn register<T>(&mut self, box_type: FourCc) -> Option<BoxRegistration>
    where
        T: CodecBox + Default + 'static,
    {
        self.insert(BoxRegistration::new(
            box_type,
            T::SUPPORTED_VERSIONS,
            construct_fixed::<T>,
        ))
    }

    /// Registers a box whose runtime `FourCc` must be injected into the constructed value.
    pub fn register_any<T>(&mut self, box_type: FourCc) -> Option<BoxRegistration>
    where
        T: CodecBox + AnyTypeBox + Default + 'static,
    {
        self.insert(BoxRegistration::new(
            box_type,
            T::SUPPORTED_VERSIONS,
            construct_any::<T>,
        ))
    }

    /// Registers a fixed-type box that is only active when `matches(context)` returns `true`.
    pub fn register_contextual<T>(
        &mut self,
        box_type: FourCc,
        matches: fn(BoxLookupContext) -> bool,
    ) where
        T: CodecBox + Default + 'static,
    {
        let registration = ContextualBoxRegistration::new(
            BoxRegistration::new(box_type, T::SUPPORTED_VERSIONS, construct_fixed::<T>),
            matches,
        );
        self.contextual_entries
            .entry(box_type)
            .or_default()
            .push(registration);
    }

    /// Registers an any-type box that is only active when `matches(context)` returns `true`.
    pub fn register_contextual_any<T>(
        &mut self,
        box_type: FourCc,
        matches: fn(BoxLookupContext) -> bool,
    ) where
        T: CodecBox + AnyTypeBox + Default + 'static,
    {
        let registration = ContextualBoxRegistration::new(
            BoxRegistration::new(box_type, T::SUPPORTED_VERSIONS, construct_any::<T>),
            matches,
        );
        self.contextual_entries
            .entry(box_type)
            .or_default()
            .push(registration);
    }

    pub(crate) fn register_dynamic_any<T>(&mut self, matches: DynamicContextPredicate)
    where
        T: CodecBox + AnyTypeBox + Default + 'static,
    {
        self.dynamic_entries.push(DynamicBoxRegistration::new(
            T::SUPPORTED_VERSIONS,
            construct_any::<T>,
            matches,
        ));
    }

    /// Creates a new descriptor-backed box instance for `box_type`.
    pub fn new_box(&self, box_type: FourCc) -> Option<Box<dyn DynCodecBox>> {
        self.new_box_with_context(box_type, BoxLookupContext::new())
    }

    /// Creates a new descriptor-backed box instance for the registration active in `context`.
    pub fn new_box_with_context(
        &self,
        box_type: FourCc,
        context: BoxLookupContext,
    ) -> Option<Box<dyn DynCodecBox>> {
        self.resolve_registration(box_type, context)
            .map(|registration| (registration.constructor)(box_type))
    }

    fn insert(&mut self, registration: BoxRegistration) -> Option<BoxRegistration> {
        self.entries.insert(registration.box_type, registration)
    }

    fn resolve_registration(
        &self,
        box_type: FourCc,
        context: BoxLookupContext,
    ) -> Option<ResolvedRegistration> {
        if let Some(registration) = self.entries.get(&box_type) {
            return Some(ResolvedRegistration::new(
                registration.supported_versions(),
                registration.constructor,
            ));
        }

        if let Some(registrations) = self.contextual_entries.get(&box_type)
            && let Some(registration) = registrations
                .iter()
                .find(|registration| (registration.matches)(context))
        {
            return Some(ResolvedRegistration::new(
                registration.registration.supported_versions(),
                registration.registration.constructor,
            ));
        }

        self.dynamic_entries
            .iter()
            .find(|registration| (registration.matches)(box_type, context))
            .map(|registration| {
                ResolvedRegistration::new(registration.supported_versions, registration.constructor)
            })
    }
}

/// Builds the built-in registry for the currently landed box families.
pub fn default_registry() -> BoxRegistry {
    let mut registry = BoxRegistry::new();
    iso14496_12::register_boxes(&mut registry);
    iso14496_14::register_boxes(&mut registry);
    iso14496_30::register_boxes(&mut registry);
    metadata::register_boxes(&mut registry);
    threegpp::register_boxes(&mut registry);
    av1::register_boxes(&mut registry);
    etsi_ts_102_366::register_boxes(&mut registry);
    opus::register_boxes(&mut registry);
    vp::register_boxes(&mut registry);
    iso23001_5::register_boxes(&mut registry);
    iso23001_7::register_boxes(&mut registry);
    registry
}

fn construct_fixed<T>(_box_type: FourCc) -> Box<dyn DynCodecBox>
where
    T: CodecBox + Default + 'static,
{
    Box::new(T::default())
}

fn construct_any<T>(box_type: FourCc) -> Box<dyn DynCodecBox>
where
    T: CodecBox + AnyTypeBox + Default + 'static,
{
    let mut boxed = T::default();
    boxed.set_box_type(box_type);
    Box::new(boxed)
}
