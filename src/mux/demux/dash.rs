use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(feature = "async")]
use tokio::fs as tokio_fs;

use super::super::MuxError;
use super::super::import::{
    SegmentedMuxSourceSegment, SegmentedMuxSourceSegmentData, SegmentedMuxSourceSpec,
};

/// Parsed one local MPD source into one or more representation-backed staged sources.
#[derive(Clone)]
pub(in crate::mux) struct ParsedDashSource {
    pub(in crate::mux) periods: Vec<ParsedDashPeriodSource>,
}

#[derive(Clone)]
pub(in crate::mux) struct ParsedDashPeriodSource {
    pub(in crate::mux) start_millis: u64,
    pub(in crate::mux) sources: Vec<SegmentedMuxSourceSpec>,
}

struct ParsedDashManifest {
    periods: Vec<ParsedDashPeriodPlan>,
}

struct ParsedDashPeriodPlan {
    start_millis: u64,
    representations: Vec<DashRepresentationPlan>,
}

struct DashRepresentationPlan {
    manifest_path: PathBuf,
    base_url_parts: Vec<String>,
    representation_id: Option<String>,
    bandwidth: Option<usize>,
    initialization: Option<DashTemplateExpansion>,
    media_plan: DashMediaPlan,
}

enum DashMediaPlan {
    Explicit(Vec<DashTemplateExpansion>),
    NumberTemplate {
        media_template: String,
        start_number: usize,
    },
}

struct DashTemplateExpansion {
    template: String,
    number: Option<usize>,
    time: Option<u64>,
}

struct ResolvedDashSegmentPath {
    path: PathBuf,
    size: u32,
}

#[derive(Clone, Default)]
struct PendingRepresentationDefaults {
    template_initialization: Option<String>,
    template_media: Option<String>,
    template_start_number: usize,
    template_segment_times: Vec<Option<u64>>,
    template_next_time: Option<u64>,
    list_initialization: Option<String>,
    list_media: Vec<String>,
}

#[derive(Default)]
struct PendingRepresentation {
    representation_id: Option<String>,
    bandwidth: Option<usize>,
    base_url: Option<String>,
    template_initialization: Option<String>,
    template_media: Option<String>,
    template_start_number: usize,
    template_segment_times: Vec<Option<u64>>,
    template_next_time: Option<u64>,
    list_initialization: Option<String>,
    list_media: Vec<String>,
}

impl PendingRepresentation {
    fn from_defaults(defaults: Option<&PendingRepresentationDefaults>) -> Self {
        let mut pending = Self::default();
        if let Some(defaults) = defaults {
            pending.template_initialization = defaults.template_initialization.clone();
            pending.template_media = defaults.template_media.clone();
            pending.template_start_number = defaults.template_start_number;
            pending.template_segment_times = defaults.template_segment_times.clone();
            pending.template_next_time = defaults.template_next_time;
            pending.list_initialization = defaults.list_initialization.clone();
            pending.list_media = defaults.list_media.clone();
        }
        pending
    }
}

#[derive(Clone)]
struct XmlTag {
    name: String,
    attrs: BTreeMap<String, String>,
    self_closing: bool,
    closing: bool,
}

enum DashXmlEvent {
    Tag(XmlTag),
    Text(String),
}

#[derive(Clone, Copy)]
enum PendingBaseUrlTarget {
    Global,
    Period,
    Adaptation,
    Representation,
}

pub(in crate::mux) fn looks_like_dash_manifest_path(path: &Path, prefix: &[u8]) -> bool {
    let Some(root_name) = extract_xml_root_name(prefix) else {
        return path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("mpd"));
    };
    root_name.eq_ignore_ascii_case("MPD")
        || root_name.eq_ignore_ascii_case("Period")
        || path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("mpd"))
}

pub(in crate::mux) fn parse_dash_source_sync(path: &Path) -> Result<ParsedDashSource, MuxError> {
    let bytes = fs::read(path)?;
    let manifest = parse_dash_source_bytes(path, &bytes)?;
    let mut periods = Vec::with_capacity(manifest.periods.len());
    for period in manifest.periods {
        let mut representation_sources = Vec::with_capacity(period.representations.len());
        for plan in period.representations {
            representation_sources.push(build_representation_source_spec_sync(plan)?);
        }
        periods.push(ParsedDashPeriodSource {
            start_millis: period.start_millis,
            sources: representation_sources,
        });
    }
    Ok(ParsedDashSource { periods })
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn parse_dash_source_async(
    path: &Path,
) -> Result<ParsedDashSource, MuxError> {
    let bytes = tokio_fs::read(path).await?;
    let manifest = parse_dash_source_bytes(path, &bytes)?;
    let mut periods = Vec::with_capacity(manifest.periods.len());
    for period in manifest.periods {
        let mut representation_sources = Vec::with_capacity(period.representations.len());
        for plan in period.representations {
            representation_sources.push(build_representation_source_spec_async(plan).await?);
        }
        periods.push(ParsedDashPeriodSource {
            start_millis: period.start_millis,
            sources: representation_sources,
        });
    }
    Ok(ParsedDashSource { periods })
}

fn parse_dash_source_bytes(path: &Path, bytes: &[u8]) -> Result<ParsedDashManifest, MuxError> {
    let mut text = std::str::from_utf8(bytes)
        .map_err(|_| invalid_dash_manifest(path, "manifest bytes are not valid UTF-8"))?;
    if let Some(stripped) = text.strip_prefix('\u{FEFF}') {
        text = stripped;
    }
    let mut saw_root = false;
    let mut period_open = false;
    let mut current_period_plans = Vec::new();
    let mut periods = Vec::new();
    let mut current_period_start_millis = 0_u64;
    let mut global_base_url = None::<String>;
    let mut period_base_url = None::<String>;
    let mut adaptation_base_url = None::<String>;
    let mut adaptation_defaults = None::<PendingRepresentationDefaults>;
    let mut representation = None::<PendingRepresentation>;
    let mut pending_base_url = None::<(PendingBaseUrlTarget, String)>;
    let mut cursor = 0usize;
    while let Some(event) = next_xml_event(text, &mut cursor)
        .map_err(|message| invalid_dash_manifest(path, &message))?
    {
        let DashXmlEvent::Tag(tag) = event else {
            if let DashXmlEvent::Text(value) = event
                && let Some((_, pending_text)) = pending_base_url.as_mut()
            {
                pending_text.push_str(&value);
            }
            continue;
        };
        let name = tag.name.as_str();
        if tag.closing {
            if name.eq_ignore_ascii_case("BaseURL") {
                if let Some((target, value)) = pending_base_url.take() {
                    let value = value.trim().to_string();
                    match target {
                        PendingBaseUrlTarget::Global => global_base_url = Some(value),
                        PendingBaseUrlTarget::Period => period_base_url = Some(value),
                        PendingBaseUrlTarget::Adaptation => adaptation_base_url = Some(value),
                        PendingBaseUrlTarget::Representation => {
                            if let Some(pending) = representation.as_mut() {
                                pending.base_url = Some(value);
                            }
                        }
                    }
                }
                continue;
            }
            if name.eq_ignore_ascii_case("Representation") {
                let Some(pending) = representation.take() else {
                    return Err(invalid_dash_manifest(
                        path,
                        "encountered `</Representation>` without `<Representation>`",
                    ));
                };
                current_period_plans.push(build_representation_plan(
                    path,
                    &global_base_url,
                    &period_base_url,
                    &adaptation_base_url,
                    pending,
                )?);
                continue;
            }
            if name.eq_ignore_ascii_case("AdaptationSet") {
                adaptation_base_url = None;
                adaptation_defaults = None;
                continue;
            }
            if name.eq_ignore_ascii_case("Period") {
                if !current_period_plans.is_empty() {
                    periods.push(ParsedDashPeriodPlan {
                        start_millis: current_period_start_millis,
                        representations: std::mem::take(&mut current_period_plans),
                    });
                }
                period_base_url = None;
                adaptation_base_url = None;
                adaptation_defaults = None;
                current_period_start_millis = 0;
                period_open = false;
                continue;
            }
            continue;
        }

        if name.eq_ignore_ascii_case("BaseURL") {
            let target = if representation.is_some() {
                PendingBaseUrlTarget::Representation
            } else if adaptation_defaults.is_some() {
                PendingBaseUrlTarget::Adaptation
            } else if period_open {
                PendingBaseUrlTarget::Period
            } else {
                PendingBaseUrlTarget::Global
            };
            if tag.self_closing {
                match target {
                    PendingBaseUrlTarget::Global => global_base_url = Some(String::new()),
                    PendingBaseUrlTarget::Period => period_base_url = Some(String::new()),
                    PendingBaseUrlTarget::Adaptation => adaptation_base_url = Some(String::new()),
                    PendingBaseUrlTarget::Representation => {
                        if let Some(pending) = representation.as_mut() {
                            pending.base_url = Some(String::new());
                        }
                    }
                }
            } else {
                pending_base_url = Some((target, String::new()));
            }
            continue;
        }

        if name.eq_ignore_ascii_case("MPD") {
            saw_root = true;
            continue;
        }
        if !saw_root {
            if name.eq_ignore_ascii_case("Period") {
                saw_root = true;
            } else {
                return Err(invalid_dash_manifest(path, "missing MPD root element"));
            }
        }
        if name.eq_ignore_ascii_case("Period") {
            if period_open && !current_period_plans.is_empty() {
                periods.push(ParsedDashPeriodPlan {
                    start_millis: current_period_start_millis,
                    representations: std::mem::take(&mut current_period_plans),
                });
            }
            period_base_url = None;
            period_open = true;
            adaptation_base_url = None;
            adaptation_defaults = None;
            current_period_start_millis = attrs_optional_string(path, &tag.attrs, "start")?
                .map(|value| parse_dash_duration_millis(path, &value))
                .transpose()?
                .unwrap_or(0);
            continue;
        }
        if name.eq_ignore_ascii_case("AdaptationSet") {
            if !period_open {
                return Err(invalid_dash_manifest(
                    path,
                    "encountered `<AdaptationSet>` outside `<Period>`",
                ));
            }
            adaptation_defaults = Some(PendingRepresentationDefaults::default());
            continue;
        }
        if name.eq_ignore_ascii_case("Representation") {
            if !period_open {
                return Err(invalid_dash_manifest(
                    path,
                    "encountered `<Representation>` outside `<Period>`",
                ));
            }
            if representation.is_some() {
                return Err(invalid_dash_manifest(
                    path,
                    "nested `<Representation>` elements are not supported",
                ));
            }
            let mut pending = PendingRepresentation::from_defaults(adaptation_defaults.as_ref());
            pending.representation_id = attrs_optional_string(path, &tag.attrs, "id")?;
            pending.bandwidth = attrs_optional_usize(path, &tag.attrs, "bandwidth")?;
            representation = Some(pending);
            if tag.self_closing {
                let pending = representation.take().unwrap();
                current_period_plans.push(build_representation_plan(
                    path,
                    &global_base_url,
                    &period_base_url,
                    &adaptation_base_url,
                    pending,
                )?);
            }
            continue;
        }
        let Some(mut pending) = representation
            .as_mut()
            .map(PendingDashTarget::Representation)
            .or_else(|| {
                adaptation_defaults
                    .as_mut()
                    .map(PendingDashTarget::AdaptationDefaults)
            })
        else {
            continue;
        };
        if name.eq_ignore_ascii_case("SegmentTemplate") {
            pending.set_template_initialization(attrs_optional_string(
                path,
                &tag.attrs,
                "initialization",
            )?);
            pending.set_template_media(attrs_optional_string(path, &tag.attrs, "media")?);
            pending.set_template_start_number(
                attrs_optional_usize(path, &tag.attrs, "startNumber")?.unwrap_or(1),
            );
            continue;
        }
        if name.eq_ignore_ascii_case("S") {
            let duration = attrs_optional_u64(path, &tag.attrs, "d")?;
            let start_time = attrs_optional_u64(path, &tag.attrs, "t")?;
            let repeat = attrs_optional_usize(path, &tag.attrs, "r")?.unwrap_or(0);
            if repeat != 0 && duration.is_none() {
                return Err(invalid_dash_manifest(
                    path,
                    "SegmentTimeline entries with `r` must also carry one `d` duration attribute",
                ));
            }
            if let Some(start_time) = start_time {
                pending.set_template_next_time(Some(start_time));
            } else if pending.template_next_time().is_none() && duration.is_some() {
                pending.set_template_next_time(Some(0));
            }
            for _ in 0..=repeat {
                pending.push_template_segment_time(pending.template_next_time());
                if let (Some(current_time), Some(duration)) =
                    (pending.template_next_time(), duration)
                {
                    pending.set_template_next_time(Some(
                        current_time
                            .checked_add(duration)
                            .ok_or(MuxError::LayoutOverflow("MPD segment timeline time"))?,
                    ));
                } else {
                    pending.set_template_next_time(None);
                }
            }
            continue;
        }
        if name.eq_ignore_ascii_case("Initialization") {
            pending.set_list_initialization(attrs_optional_string(path, &tag.attrs, "sourceURL")?);
            continue;
        }
        if name.eq_ignore_ascii_case("SegmentURL") {
            let Some(media) = attrs_optional_string(path, &tag.attrs, "media")? else {
                return Err(invalid_dash_manifest(
                    path,
                    "SegmentURL entries must carry one `media` attribute",
                ));
            };
            pending.push_list_media(media);
        }
    }

    if !saw_root {
        return Err(invalid_dash_manifest(path, "missing MPD root element"));
    }
    if let Some(pending) = representation.take() {
        current_period_plans.push(build_representation_plan(
            path,
            &global_base_url,
            &period_base_url,
            &adaptation_base_url,
            pending,
        )?);
    }
    if !current_period_plans.is_empty() {
        periods.push(ParsedDashPeriodPlan {
            start_millis: current_period_start_millis,
            representations: current_period_plans,
        });
    }
    if periods.is_empty() {
        return Err(invalid_dash_manifest(
            path,
            "MPD did not describe any local representation-backed sources",
        ));
    }
    if periods
        .iter()
        .any(|period| period.representations.is_empty())
    {
        return Err(invalid_dash_manifest(
            path,
            "one MPD Period did not describe any local representation-backed sources",
        ));
    }
    Ok(ParsedDashManifest { periods })
}

fn parse_dash_duration_millis(path: &Path, value: &str) -> Result<u64, MuxError> {
    let Some(mut remainder) = value.strip_prefix("PT") else {
        return Err(invalid_dash_manifest(
            path,
            "only `PT...` DASH duration strings are supported on the local path-only ingest surface",
        ));
    };
    if remainder.is_empty() {
        return Err(invalid_dash_manifest(path, "empty DASH duration string"));
    }

    let mut total_seconds = 0_f64;
    let mut saw_component = false;
    while !remainder.is_empty() {
        let component_len = remainder
            .find(|ch: char| !matches!(ch, '0'..='9' | '.'))
            .ok_or_else(|| {
                invalid_dash_manifest(path, "DASH duration component is missing a unit suffix")
            })?;
        let (number, rest) = remainder.split_at(component_len);
        let Some(unit) = rest.chars().next() else {
            return Err(invalid_dash_manifest(
                path,
                "DASH duration component is missing a unit suffix",
            ));
        };
        let magnitude = number.parse::<f64>().map_err(|_| {
            invalid_dash_manifest(path, "DASH duration component is not a valid number")
        })?;
        if !magnitude.is_finite() || magnitude < 0.0 {
            return Err(invalid_dash_manifest(
                path,
                "DASH duration component must be a finite non-negative value",
            ));
        }
        match unit {
            'H' => total_seconds += magnitude * 3600.0,
            'M' => total_seconds += magnitude * 60.0,
            'S' => total_seconds += magnitude,
            _ => {
                return Err(invalid_dash_manifest(
                    path,
                    "unsupported DASH duration unit on the local path-only ingest surface",
                ));
            }
        }
        saw_component = true;
        remainder = &rest[unit.len_utf8()..];
    }

    if !saw_component {
        return Err(invalid_dash_manifest(path, "empty DASH duration string"));
    }

    let millis = (total_seconds * 1000.0).round();
    if !millis.is_finite() || millis < 0.0 || millis > u64::MAX as f64 {
        return Err(invalid_dash_manifest(
            path,
            "DASH duration is too large for the current local ingest surface",
        ));
    }
    Ok(millis as u64)
}

enum PendingDashTarget<'a> {
    Representation(&'a mut PendingRepresentation),
    AdaptationDefaults(&'a mut PendingRepresentationDefaults),
}

impl PendingDashTarget<'_> {
    fn set_template_initialization(&mut self, value: Option<String>) {
        match self {
            Self::Representation(pending) => pending.template_initialization = value,
            Self::AdaptationDefaults(pending) => pending.template_initialization = value,
        }
    }

    fn set_template_media(&mut self, value: Option<String>) {
        match self {
            Self::Representation(pending) => pending.template_media = value,
            Self::AdaptationDefaults(pending) => pending.template_media = value,
        }
    }

    fn set_template_start_number(&mut self, value: usize) {
        match self {
            Self::Representation(pending) => pending.template_start_number = value,
            Self::AdaptationDefaults(pending) => pending.template_start_number = value,
        }
    }

    fn template_next_time(&self) -> Option<u64> {
        match self {
            Self::Representation(pending) => pending.template_next_time,
            Self::AdaptationDefaults(pending) => pending.template_next_time,
        }
    }

    fn set_template_next_time(&mut self, value: Option<u64>) {
        match self {
            Self::Representation(pending) => pending.template_next_time = value,
            Self::AdaptationDefaults(pending) => pending.template_next_time = value,
        }
    }

    fn push_template_segment_time(&mut self, value: Option<u64>) {
        match self {
            Self::Representation(pending) => pending.template_segment_times.push(value),
            Self::AdaptationDefaults(pending) => pending.template_segment_times.push(value),
        }
    }

    fn set_list_initialization(&mut self, value: Option<String>) {
        match self {
            Self::Representation(pending) => pending.list_initialization = value,
            Self::AdaptationDefaults(pending) => pending.list_initialization = value,
        }
    }

    fn push_list_media(&mut self, value: String) {
        match self {
            Self::Representation(pending) => pending.list_media.push(value),
            Self::AdaptationDefaults(pending) => pending.list_media.push(value),
        }
    }
}

fn build_representation_plan(
    manifest_path: &Path,
    global_base_url: &Option<String>,
    period_base_url: &Option<String>,
    adaptation_base_url: &Option<String>,
    representation: PendingRepresentation,
) -> Result<DashRepresentationPlan, MuxError> {
    let mut base_url_parts = Vec::new();
    if let Some(base_url) = global_base_url.as_ref().filter(|value| !value.is_empty()) {
        base_url_parts.push(base_url.clone());
    }
    if let Some(base_url) = period_base_url.as_ref().filter(|value| !value.is_empty()) {
        base_url_parts.push(base_url.clone());
    }
    if let Some(base_url) = adaptation_base_url
        .as_ref()
        .filter(|value| !value.is_empty())
    {
        base_url_parts.push(base_url.clone());
    }
    if let Some(base_url) = representation
        .base_url
        .as_ref()
        .filter(|value| !value.is_empty())
    {
        base_url_parts.push(base_url.clone());
    }
    let initialization = representation
        .list_initialization
        .or(representation.template_initialization)
        .map(|template| DashTemplateExpansion {
            template,
            number: None,
            time: None,
        });
    let media_plan = if !representation.list_media.is_empty() {
        DashMediaPlan::Explicit(
            representation
                .list_media
                .into_iter()
                .map(|template| DashTemplateExpansion {
                    template,
                    number: None,
                    time: None,
                })
                .collect(),
        )
    } else if let Some(media_template) = representation.template_media {
        if representation.template_segment_times.is_empty() {
            if dash_template_uses_token(&media_template, "Time") {
                return Err(invalid_dash_manifest(
                    manifest_path,
                    "SegmentTemplate used `$Time$` without one SegmentTimeline",
                ));
            }
            if !dash_template_uses_token(&media_template, "Number") {
                return Err(invalid_dash_manifest(
                    manifest_path,
                    "SegmentTemplate without one SegmentTimeline must use `$Number$` so the local path-only importer can enumerate segment files",
                ));
            }
            DashMediaPlan::NumberTemplate {
                media_template,
                start_number: representation.template_start_number,
            }
        } else {
            DashMediaPlan::Explicit(
                representation
                    .template_segment_times
                    .into_iter()
                    .enumerate()
                    .map(|(index, time)| {
                        let number = representation
                            .template_start_number
                            .checked_add(index)
                            .ok_or(MuxError::LayoutOverflow("MPD segment number"))?;
                        Ok(DashTemplateExpansion {
                            template: media_template.clone(),
                            number: Some(number),
                            time,
                        })
                    })
                    .collect::<Result<Vec<_>, MuxError>>()?,
            )
        }
    } else {
        DashMediaPlan::Explicit(Vec::new())
    };
    Ok(DashRepresentationPlan {
        manifest_path: manifest_path.to_path_buf(),
        base_url_parts,
        representation_id: representation.representation_id,
        bandwidth: representation.bandwidth,
        initialization,
        media_plan,
    })
}

fn build_representation_source_spec_sync(
    plan: DashRepresentationPlan,
) -> Result<SegmentedMuxSourceSpec, MuxError> {
    let source_paths = resolve_representation_paths_sync(&plan)?;
    build_segmented_source_spec(&plan.manifest_path, source_paths)
}

#[cfg(feature = "async")]
async fn build_representation_source_spec_async(
    plan: DashRepresentationPlan,
) -> Result<SegmentedMuxSourceSpec, MuxError> {
    let source_paths = resolve_representation_paths_async(&plan).await?;
    build_segmented_source_spec(&plan.manifest_path, source_paths)
}

fn resolve_representation_paths_sync(
    plan: &DashRepresentationPlan,
) -> Result<Vec<ResolvedDashSegmentPath>, MuxError> {
    let mut source_paths = Vec::new();
    if let Some(initialization) = &plan.initialization {
        source_paths.push(resolve_dash_segment_path_sync(plan, initialization)?);
    }
    match &plan.media_plan {
        DashMediaPlan::Explicit(media_entries) => {
            for media in media_entries {
                source_paths.push(resolve_dash_segment_path_sync(plan, media)?);
            }
        }
        DashMediaPlan::NumberTemplate {
            media_template,
            start_number,
        } => {
            source_paths.extend(resolve_dash_number_template_paths_sync(
                plan,
                media_template,
                *start_number,
            )?);
        }
    }
    if source_paths.is_empty() {
        return Err(invalid_dash_manifest(
            &plan.manifest_path,
            "Representation did not resolve to any initialization or segment file paths",
        ));
    }
    Ok(source_paths)
}

#[cfg(feature = "async")]
async fn resolve_representation_paths_async(
    plan: &DashRepresentationPlan,
) -> Result<Vec<ResolvedDashSegmentPath>, MuxError> {
    let mut source_paths = Vec::new();
    if let Some(initialization) = &plan.initialization {
        source_paths.push(resolve_dash_segment_path_async(plan, initialization).await?);
    }
    match &plan.media_plan {
        DashMediaPlan::Explicit(media_entries) => {
            for media in media_entries {
                source_paths.push(resolve_dash_segment_path_async(plan, media).await?);
            }
        }
        DashMediaPlan::NumberTemplate {
            media_template,
            start_number,
        } => {
            source_paths.extend(
                resolve_dash_number_template_paths_async(plan, media_template, *start_number)
                    .await?,
            );
        }
    }
    if source_paths.is_empty() {
        return Err(invalid_dash_manifest(
            &plan.manifest_path,
            "Representation did not resolve to any initialization or segment file paths",
        ));
    }
    Ok(source_paths)
}

fn resolve_dash_segment_path_sync(
    plan: &DashRepresentationPlan,
    expansion: &DashTemplateExpansion,
) -> Result<ResolvedDashSegmentPath, MuxError> {
    let media = expand_dash_template(
        &plan.manifest_path,
        &expansion.template,
        plan.representation_id.as_deref(),
        plan.bandwidth,
        expansion.number,
        expansion.time,
    )?;
    let path = resolve_dash_path(&plan.manifest_path, &plan.base_url_parts, &media)?;
    let size = fs::metadata(&path)
        .map_err(|error| {
            MuxError::Io(std::io::Error::new(
                error.kind(),
                format!("failed to stat DASH segment `{}`: {error}", path.display()),
            ))
        })?
        .len();
    let size = u32::try_from(size)
        .map_err(|_| invalid_dash_manifest(&plan.manifest_path, "segment size exceeds u32"))?;
    Ok(ResolvedDashSegmentPath { path, size })
}

#[cfg(feature = "async")]
async fn resolve_dash_segment_path_async(
    plan: &DashRepresentationPlan,
    expansion: &DashTemplateExpansion,
) -> Result<ResolvedDashSegmentPath, MuxError> {
    let media = expand_dash_template(
        &plan.manifest_path,
        &expansion.template,
        plan.representation_id.as_deref(),
        plan.bandwidth,
        expansion.number,
        expansion.time,
    )?;
    let path = resolve_dash_path(&plan.manifest_path, &plan.base_url_parts, &media)?;
    let size = tokio_fs::metadata(&path)
        .await
        .map_err(|error| {
            MuxError::Io(std::io::Error::new(
                error.kind(),
                format!("failed to stat DASH segment `{}`: {error}", path.display()),
            ))
        })?
        .len();
    let size = u32::try_from(size)
        .map_err(|_| invalid_dash_manifest(&plan.manifest_path, "segment size exceeds u32"))?;
    Ok(ResolvedDashSegmentPath { path, size })
}

fn resolve_dash_number_template_paths_sync(
    plan: &DashRepresentationPlan,
    media_template: &str,
    start_number: usize,
) -> Result<Vec<ResolvedDashSegmentPath>, MuxError> {
    let mut source_paths = Vec::new();
    let mut next_number = start_number;
    loop {
        let expansion = DashTemplateExpansion {
            template: media_template.to_string(),
            number: Some(next_number),
            time: None,
        };
        match resolve_dash_segment_path_sync(plan, &expansion) {
            Ok(resolved) => {
                source_paths.push(resolved);
                next_number = next_number
                    .checked_add(1)
                    .ok_or(MuxError::LayoutOverflow("MPD segment number"))?;
            }
            Err(MuxError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                if source_paths.is_empty() {
                    return Err(invalid_dash_manifest(
                        &plan.manifest_path,
                        "SegmentTemplate did not resolve any local numbered segment file paths",
                    ));
                }
                break;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(source_paths)
}

#[cfg(feature = "async")]
async fn resolve_dash_number_template_paths_async(
    plan: &DashRepresentationPlan,
    media_template: &str,
    start_number: usize,
) -> Result<Vec<ResolvedDashSegmentPath>, MuxError> {
    let mut source_paths = Vec::new();
    let mut next_number = start_number;
    loop {
        let expansion = DashTemplateExpansion {
            template: media_template.to_string(),
            number: Some(next_number),
            time: None,
        };
        match resolve_dash_segment_path_async(plan, &expansion).await {
            Ok(resolved) => {
                source_paths.push(resolved);
                next_number = next_number
                    .checked_add(1)
                    .ok_or(MuxError::LayoutOverflow("MPD segment number"))?;
            }
            Err(MuxError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                if source_paths.is_empty() {
                    return Err(invalid_dash_manifest(
                        &plan.manifest_path,
                        "SegmentTemplate did not resolve any local numbered segment file paths",
                    ));
                }
                break;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(source_paths)
}

fn build_segmented_source_spec(
    manifest_path: &Path,
    source_paths: Vec<ResolvedDashSegmentPath>,
) -> Result<SegmentedMuxSourceSpec, MuxError> {
    let mut logical_offset = 0_u64;
    let mut segments = Vec::with_capacity(source_paths.len());
    for resolved in source_paths {
        segments.push(SegmentedMuxSourceSegment {
            logical_offset,
            data: SegmentedMuxSourceSegmentData::ExternalFileRange {
                path: resolved.path,
                source_offset: 0,
                size: resolved.size,
            },
        });
        logical_offset = logical_offset
            .checked_add(u64::from(resolved.size))
            .ok_or(MuxError::LayoutOverflow("MPD segmented source size"))?;
    }
    Ok(SegmentedMuxSourceSpec {
        path: manifest_path.to_path_buf(),
        segments,
        total_size: logical_offset,
    })
}

fn expand_dash_template(
    manifest_path: &Path,
    template: &str,
    representation_id: Option<&str>,
    bandwidth: Option<usize>,
    number: Option<usize>,
    time: Option<u64>,
) -> Result<String, MuxError> {
    let mut expanded = String::with_capacity(template.len());
    let mut cursor = 0usize;
    while let Some(token_start_rel) = template[cursor..].find('$') {
        let token_start = cursor + token_start_rel;
        expanded.push_str(&template[cursor..token_start]);
        if template[token_start..].starts_with("$$") {
            expanded.push('$');
            cursor = token_start + 2;
            continue;
        }
        let token_end_rel = template[token_start + 1..].find('$').ok_or_else(|| {
            invalid_dash_manifest(
                manifest_path,
                &format!("unterminated SegmentTemplate token in `{template}`"),
            )
        })?;
        let token_end = token_start + 1 + token_end_rel;
        expanded.push_str(&expand_dash_template_token(
            manifest_path,
            &template[token_start + 1..token_end],
            representation_id,
            bandwidth,
            number,
            time,
        )?);
        cursor = token_end + 1;
    }
    expanded.push_str(&template[cursor..]);
    Ok(expanded)
}

fn dash_template_uses_token(template: &str, token_name: &str) -> bool {
    let mut cursor = 0usize;
    while let Some(token_start_rel) = template[cursor..].find('$') {
        let token_start = cursor + token_start_rel;
        if template[token_start..].starts_with("$$") {
            cursor = token_start + 2;
            continue;
        }
        let Some(token_end_rel) = template[token_start + 1..].find('$') else {
            return false;
        };
        let token_end = token_start + 1 + token_end_rel;
        let token = &template[token_start + 1..token_end];
        let name = token.split('%').next().unwrap_or(token);
        if name == token_name {
            return true;
        }
        cursor = token_end + 1;
    }
    false
}

fn expand_dash_template_token(
    manifest_path: &Path,
    token: &str,
    representation_id: Option<&str>,
    bandwidth: Option<usize>,
    number: Option<usize>,
    time: Option<u64>,
) -> Result<String, MuxError> {
    let (name, width, zero_pad) = parse_dash_token_format(manifest_path, token)?;
    match name {
        "RepresentationID" => {
            if width.is_some() || zero_pad {
                return Err(invalid_dash_manifest(
                    manifest_path,
                    "`$RepresentationID$` does not support integer-width formatting",
                ));
            }
            Ok(representation_id
                .ok_or_else(|| {
                    invalid_dash_manifest(
                        manifest_path,
                        "SegmentTemplate used `$RepresentationID$` without one Representation `id` attribute",
                    )
                })?
                .to_string())
        }
        "Bandwidth" => format_dash_template_number(
            manifest_path,
            bandwidth.map(|value| value as u64),
            width,
            zero_pad,
            "SegmentTemplate used `$Bandwidth$` without one Representation `bandwidth` attribute",
        ),
        "Number" => format_dash_template_number(
            manifest_path,
            number.map(|value| value as u64),
            width,
            zero_pad,
            "SegmentTemplate used `$Number$` outside one numbered media-template expansion",
        ),
        "Time" => format_dash_template_number(
            manifest_path,
            time,
            width,
            zero_pad,
            "SegmentTemplate used `$Time$` outside one timeline-backed media-template expansion",
        ),
        _ => Err(invalid_dash_manifest(
            manifest_path,
            &format!("unsupported SegmentTemplate token `${token}$`"),
        )),
    }
}

fn parse_dash_token_format<'a>(
    manifest_path: &Path,
    token: &'a str,
) -> Result<(&'a str, Option<usize>, bool), MuxError> {
    let Some(format_start) = token.find('%') else {
        return Ok((token, None, false));
    };
    let name = &token[..format_start];
    let format = &token[format_start + 1..];
    let Some(format_body) = format.strip_suffix('d') else {
        return Err(invalid_dash_manifest(
            manifest_path,
            &format!("unsupported SegmentTemplate integer formatter `%{format}` in `${token}$`"),
        ));
    };
    let (zero_pad, digits) = if let Some(rest) = format_body.strip_prefix('0') {
        (true, rest)
    } else {
        (false, format_body)
    };
    if digits.is_empty() || !digits.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(invalid_dash_manifest(
            manifest_path,
            &format!("unsupported SegmentTemplate integer formatter `%{format}` in `${token}$`"),
        ));
    }
    let width = digits.parse::<usize>().map_err(|_| {
        invalid_dash_manifest(
            manifest_path,
            &format!("unsupported SegmentTemplate integer formatter `%{format}` in `${token}$`"),
        )
    })?;
    Ok((name, Some(width), zero_pad))
}

fn format_dash_template_number(
    manifest_path: &Path,
    value: Option<u64>,
    width: Option<usize>,
    zero_pad: bool,
    missing_message: &'static str,
) -> Result<String, MuxError> {
    let value = value.ok_or_else(|| invalid_dash_manifest(manifest_path, missing_message))?;
    match width {
        Some(width) if zero_pad => Ok(format!("{value:0width$}")),
        Some(width) => Ok(format!("{value:width$}")),
        None => Ok(value.to_string()),
    }
}

fn resolve_dash_path(
    manifest_path: &Path,
    base_urls: &[String],
    url: &str,
) -> Result<PathBuf, MuxError> {
    let mut joined = manifest_path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_default();
    for base_url in base_urls {
        if let Some(local_path) = resolve_dash_local_file_uri(base_url) {
            joined = if local_path.is_absolute() {
                local_path
            } else {
                joined.join(local_path)
            };
            continue;
        }
        if base_url.contains("://") {
            return Err(invalid_dash_manifest(
                manifest_path,
                "remote MPD URLs are not supported on the current path-only ingest surface; only local paths and file:// URIs are supported",
            ));
        }
        joined = joined.join(PathBuf::from(base_url));
    }
    if let Some(local_path) = resolve_dash_local_file_uri(url) {
        return Ok(if local_path.is_absolute() {
            local_path
        } else {
            joined.join(local_path)
        });
    }
    if url.contains("://") {
        return Err(invalid_dash_manifest(
            manifest_path,
            "remote MPD URLs are not supported on the current path-only ingest surface; only local paths and file:// URIs are supported",
        ));
    }
    let candidate = PathBuf::from(url);
    if candidate.is_absolute() {
        Ok(candidate)
    } else {
        Ok(joined.join(candidate))
    }
}

fn resolve_dash_local_file_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    if rest.starts_with("//") {
        Some(PathBuf::from(format!(
            r"\\{}",
            rest.trim_start_matches('/').replace('/', "\\")
        )))
    } else if rest.starts_with('/')
        && rest.len() >= 3
        && rest.as_bytes()[2] == b':'
        && rest.as_bytes()[1].is_ascii_alphabetic()
    {
        Some(PathBuf::from(&rest[1..]))
    } else if rest.is_empty() {
        None
    } else {
        Some(PathBuf::from(rest))
    }
}

fn next_xml_event(input: &str, cursor: &mut usize) -> Result<Option<DashXmlEvent>, String> {
    while *cursor < input.len() {
        let rest = &input[*cursor..];
        if rest.starts_with("<?") {
            let Some(end) = rest.find("?>") else {
                return Err("unterminated XML declaration in DASH manifest".to_string());
            };
            *cursor += end + 2;
            continue;
        }
        if rest.starts_with("<!--") {
            let Some(end) = rest.find("-->") else {
                return Err("unterminated XML comment in DASH manifest".to_string());
            };
            *cursor += end + 3;
            continue;
        }
        if rest.starts_with('<') {
            let tag_end = find_xml_tag_end(rest)?;
            let tag = parse_xml_tag(&rest[..=tag_end])?;
            *cursor += tag_end + 1;
            return Ok(Some(DashXmlEvent::Tag(tag)));
        }
        let next_tag = rest.find('<').unwrap_or(rest.len());
        let text = xml_unescape_attr(&rest[..next_tag])?;
        *cursor += next_tag;
        if text.trim().is_empty() {
            continue;
        }
        return Ok(Some(DashXmlEvent::Text(text)));
    }
    Ok(None)
}

fn find_xml_tag_end(text: &str) -> Result<usize, String> {
    let bytes = text.as_bytes();
    let mut in_quotes = false;
    let mut index = 1usize;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => in_quotes = !in_quotes,
            b'>' if !in_quotes => return Ok(index),
            _ => {}
        }
        index += 1;
    }
    Err("unterminated XML tag in DASH manifest".to_string())
}

fn parse_xml_tag(tag_text: &str) -> Result<XmlTag, String> {
    let trimmed = tag_text.trim();
    if let Some(content) = trimmed
        .strip_prefix("</")
        .and_then(|value| value.strip_suffix('>'))
    {
        return Ok(XmlTag {
            name: content.trim().to_string(),
            attrs: BTreeMap::new(),
            self_closing: false,
            closing: true,
        });
    }
    let mut inner = trimmed
        .strip_prefix('<')
        .and_then(|value| value.strip_suffix('>'))
        .ok_or_else(|| format!("unsupported MPD tag `{trimmed}`"))?
        .trim();
    let self_closing = inner.ends_with('/');
    if self_closing {
        inner = inner[..inner.len() - 1].trim_end();
    }
    let name_end = inner.find(char::is_whitespace).unwrap_or(inner.len());
    let name = inner[..name_end].to_string();
    let mut attrs = BTreeMap::new();
    let mut cursor = inner[name_end..].trim_start();
    while !cursor.is_empty() {
        let Some(eq_pos) = cursor.find('=') else {
            return Err(format!("malformed MPD attribute list in `{trimmed}`"));
        };
        let key = cursor[..eq_pos].trim();
        if key.is_empty() {
            return Err(format!("malformed MPD attribute list in `{trimmed}`"));
        }
        let rest = cursor[eq_pos + 1..].trim_start();
        let Some(rest) = rest.strip_prefix('"') else {
            return Err(format!(
                "MPD attribute `{key}` in `{trimmed}` must use double quotes"
            ));
        };
        let Some(value_end) = rest.find('"') else {
            return Err(format!("unterminated MPD attribute `{key}` in `{trimmed}`"));
        };
        attrs.insert(key.to_string(), xml_unescape_attr(&rest[..value_end])?);
        cursor = rest[value_end + 1..].trim_start();
    }
    Ok(XmlTag {
        name,
        attrs,
        self_closing,
        closing: false,
    })
}

fn extract_xml_root_name(prefix: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(prefix).ok()?;
    let text = text.trim_start_matches('\u{FEFF}').trim_start();
    let text = if text.starts_with("<?xml") {
        let end = text.find("?>")?;
        text[end + 2..].trim_start()
    } else {
        text
    };
    let body = text.strip_prefix('<')?;
    let name_end = body
        .find(|ch: char| ch.is_whitespace() || ch == '>' || ch == '/')
        .unwrap_or(body.len());
    if name_end == 0 {
        None
    } else {
        Some(body[..name_end].to_string())
    }
}

fn xml_unescape_attr(value: &str) -> Result<String, String> {
    let mut rendered = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '&' {
            rendered.push(ch);
            continue;
        }
        let mut entity = String::new();
        for next in chars.by_ref() {
            if next == ';' {
                break;
            }
            entity.push(next);
        }
        match entity.as_str() {
            "amp" => rendered.push('&'),
            "lt" => rendered.push('<'),
            "gt" => rendered.push('>'),
            "quot" => rendered.push('"'),
            "#39" => rendered.push('\''),
            _ => return Err(format!("unsupported XML entity `&{entity};`")),
        }
    }
    Ok(rendered)
}

fn attrs_optional_string(
    path: &Path,
    attrs: &BTreeMap<String, String>,
    key: &str,
) -> Result<Option<String>, MuxError> {
    let Some(value) = attrs.get(key) else {
        return Ok(None);
    };
    if value.is_empty() {
        return Err(invalid_dash_manifest(
            path,
            &format!("attribute `{key}` must not be empty"),
        ));
    }
    Ok(Some(value.clone()))
}

fn attrs_optional_usize(
    path: &Path,
    attrs: &BTreeMap<String, String>,
    key: &str,
) -> Result<Option<usize>, MuxError> {
    let Some(value) = attrs.get(key) else {
        return Ok(None);
    };
    value.parse::<usize>().map(Some).map_err(|_| {
        invalid_dash_manifest(
            path,
            &format!("attribute `{key}` must be one platform-sized unsigned integer"),
        )
    })
}

fn attrs_optional_u64(
    path: &Path,
    attrs: &BTreeMap<String, String>,
    key: &str,
) -> Result<Option<u64>, MuxError> {
    let Some(value) = attrs.get(key) else {
        return Ok(None);
    };
    value.parse::<u64>().map(Some).map_err(|_| {
        invalid_dash_manifest(
            path,
            &format!("attribute `{key}` must be one unsigned 64-bit integer"),
        )
    })
}

fn invalid_dash_manifest(path: &Path, message: &str) -> MuxError {
    MuxError::UnsupportedTrackImport {
        spec: path.display().to_string(),
        message: format!("invalid DASH manifest: {message}"),
    }
}
