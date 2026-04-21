//! Item-list metadata boxes, typed item value leaves, free-form metadata leaves, and key tables.

use std::io::{SeekFrom, Write};

use crate::boxes::{AnyTypeBox, BoxLookupContext, BoxRegistry};
use crate::codec::{
    CodecBox, FieldHooks, FieldTable, FieldValue, FieldValueError, FieldValueRead, FieldValueWrite,
    ImmutableBox, MutableBox, ReadSeek, read_exact_vec_untrusted, untrusted_prealloc_hint,
};
use crate::stringify::stringify;
use crate::{BoxInfo, FourCc, codec_field};

const FREE_FORM_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"----");
const ALBUM_ARTIST_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"aART");
const ACCOUNT_KIND_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"akID");
const APPLE_ID_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"apID");
const ARTIST_ID_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"atID");
const CMID_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"cmID");
const CNID_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"cnID");
const DESCRIPTION_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"desc");
const TRACK_NUMBER_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"trkn");
const DISK_NUMBER_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"disk");
const EPISODE_GUID_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"egid");
const GENRE_ID_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"geID");
const TEMPO_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"tmpo");
const MEDIA_TYPE_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"stik");
const COMPILATION_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"cpil");
const PODCAST_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"pcst");
const GAPLESS_PLAYBACK_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"pgap");
const PLAYLIST_ID_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"plID");
const PURCHASE_DATE_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"purd");
const PODCAST_URL_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"purl");
const RATING_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"rtng");
const SFID_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"sfID");
const ALBUM_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes([0xa9, b'a', b'l', b'b']);
const ARTIST_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes([0xa9, b'A', b'R', b'T']);
const COMMENT_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes([0xa9, b'c', b'm', b't']);
const COMPOSER_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes([0xa9, b'c', b'o', b'm']);
const COPYRIGHT_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"cprt");
const DAY_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes([0xa9, b'd', b'a', b'y']);
const GENRE_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes([0xa9, b'g', b'e', b'n']);
const GROUPING_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes([0xa9, b'g', b'r', b'p']);
const LEGACY_GENRE_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"gnre");
const NAME_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes([0xa9, b'n', b'a', b'm']);
const TOOL_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes([0xa9, b't', b'o', b'o']);
const SORT_ALBUM_ARTIST_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"soaa");
const SORT_ALBUM_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"soal");
const SORT_ARTIST_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"soar");
const SORT_COMPOSER_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"soco");
const SORT_NAME_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"sonm");
const SORT_SHOW_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"sosn");
const TV_EPISODE_ID_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"tven");
const TV_EPISODE_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"tves");
const TV_NETWORK_NAME_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"tvnn");
const TV_SHOW_NAME_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"tvsh");
const TV_SEASON_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes(*b"tvsn");
const WRITER_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes([0xa9, b'w', b'r', b't']);
const ILST_META_BOX_TYPES: &[FourCc] = &[
    FREE_FORM_METADATA_ITEM_TYPE,
    FourCc::from_bytes(*b"aART"),
    FourCc::from_bytes(*b"akID"),
    FourCc::from_bytes(*b"apID"),
    FourCc::from_bytes(*b"atID"),
    FourCc::from_bytes(*b"cmID"),
    FourCc::from_bytes(*b"cnID"),
    FourCc::from_bytes(*b"covr"),
    FourCc::from_bytes(*b"cpil"),
    FourCc::from_bytes(*b"cprt"),
    FourCc::from_bytes(*b"desc"),
    FourCc::from_bytes(*b"disk"),
    FourCc::from_bytes(*b"egid"),
    FourCc::from_bytes(*b"geID"),
    FourCc::from_bytes(*b"gnre"),
    FourCc::from_bytes(*b"pcst"),
    FourCc::from_bytes(*b"pgap"),
    FourCc::from_bytes(*b"plID"),
    FourCc::from_bytes(*b"purd"),
    FourCc::from_bytes(*b"purl"),
    FourCc::from_bytes(*b"rtng"),
    FourCc::from_bytes(*b"sfID"),
    FourCc::from_bytes(*b"soaa"),
    FourCc::from_bytes(*b"soal"),
    FourCc::from_bytes(*b"soar"),
    FourCc::from_bytes(*b"soco"),
    FourCc::from_bytes(*b"sonm"),
    FourCc::from_bytes(*b"sosn"),
    FourCc::from_bytes(*b"stik"),
    FourCc::from_bytes(*b"tmpo"),
    FourCc::from_bytes(*b"trkn"),
    FourCc::from_bytes(*b"tven"),
    FourCc::from_bytes(*b"tves"),
    FourCc::from_bytes(*b"tvnn"),
    FourCc::from_bytes(*b"tvsh"),
    FourCc::from_bytes(*b"tvsn"),
    FourCc::from_bytes([0xa9, b'A', b'R', b'T']),
    FourCc::from_bytes([0xa9, b'a', b'l', b'b']),
    FourCc::from_bytes([0xa9, b'c', b'm', b't']),
    FourCc::from_bytes([0xa9, b'c', b'o', b'm']),
    FourCc::from_bytes([0xa9, b'd', b'a', b'y']),
    FourCc::from_bytes([0xa9, b'g', b'e', b'n']),
    FourCc::from_bytes([0xa9, b'g', b'r', b'p']),
    FourCc::from_bytes([0xa9, b'n', b'a', b'm']),
    FourCc::from_bytes([0xa9, b't', b'o', b'o']),
    FourCc::from_bytes([0xa9, b'w', b'r', b't']),
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct FullBoxState {
    version: u8,
    flags: u32,
}

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

fn data_type_label(data_type: u32) -> Option<&'static str> {
    match data_type {
        DATA_TYPE_BINARY => Some("BINARY"),
        DATA_TYPE_STRING_UTF8 => Some("UTF8"),
        DATA_TYPE_STRING_UTF16 => Some("UTF16"),
        DATA_TYPE_STRING_MAC => Some("MAC_STR"),
        DATA_TYPE_STRING_JPEG => Some("JPEG"),
        DATA_TYPE_SIGNED_INT_BIG_ENDIAN => Some("INT"),
        DATA_TYPE_FLOAT32_BIG_ENDIAN => Some("FLOAT32"),
        DATA_TYPE_FLOAT64_BIG_ENDIAN => Some("FLOAT64"),
        _ => None,
    }
}

fn display_utf8_bytes(data_type: u32, bytes: &[u8]) -> Option<String> {
    (data_type == DATA_TYPE_STRING_UTF8).then(|| quote_bytes(bytes))
}

fn u32_from_unsigned(field_name: &'static str, value: u64) -> Result<u32, FieldValueError> {
    u32::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in u32"))
}

fn u16_from_unsigned(field_name: &'static str, value: u64) -> Result<u16, FieldValueError> {
    u16::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in u16"))
}

fn u8_from_unsigned(field_name: &'static str, value: u64) -> Result<u8, FieldValueError> {
    u8::try_from(value).map_err(|_| invalid_value(field_name, "value does not fit in u8"))
}

fn u64_from_unsigned(_: &'static str, value: u64) -> Result<u64, FieldValueError> {
    Ok(value)
}

fn bool_from_unsigned(field_name: &'static str, value: u64) -> Result<bool, FieldValueError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(invalid_value(field_name, "value must be 0 or 1")),
    }
}

fn unsigned_from_bool(value: bool) -> u64 {
    u64::from(u8::from(value))
}

fn bytes_to_fourcc(field_name: &'static str, bytes: Vec<u8>) -> Result<FourCc, FieldValueError> {
    let bytes: [u8; 4] = bytes
        .try_into()
        .map_err(|_| invalid_value(field_name, "value must be exactly 4 bytes"))?;
    Ok(FourCc::from_bytes(bytes))
}

fn quote_fourcc(value: FourCc) -> String {
    format!("\"{value}\"")
}

fn quote_bytes(bytes: &[u8]) -> String {
    format!("\"{}\"", escape_bytes(bytes))
}

fn escape_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| escape_char(char::from(*byte)))
        .collect::<String>()
}

fn escape_char(value: char) -> char {
    if value.is_control() || (!value.is_ascii_graphic() && value != ' ') {
        '.'
    } else {
        value
    }
}

fn render_array<I>(values: I) -> String
where
    I: IntoIterator<Item = String>,
{
    format!("[{}]", values.into_iter().collect::<Vec<_>>().join(", "))
}

fn encode_data_payload(data: &Data) -> Vec<u8> {
    let mut payload = Vec::with_capacity(8 + data.data.len());
    payload.extend_from_slice(&data.data_type.to_be_bytes());
    payload.extend_from_slice(&data.data_lang.to_be_bytes());
    payload.extend_from_slice(&data.data);
    payload
}

fn decode_data_payload(
    field_name: &'static str,
    payload: Vec<u8>,
) -> Result<Data, FieldValueError> {
    if payload.len() < 8 {
        return Err(invalid_value(
            field_name,
            "data payload is shorter than the fixed metadata header",
        ));
    }

    Ok(Data {
        data_type: u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]),
        data_lang: u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]),
        data: payload[8..].to_vec(),
    })
}

fn encode_keys_entries(entries: &[Key]) -> Result<Vec<u8>, FieldValueError> {
    let mut payload = Vec::new();
    for entry in entries {
        let expected_size = entry
            .key_value
            .len()
            .checked_add(8)
            .ok_or_else(|| invalid_value("Entries", "key size overflows the entry header"))?;
        if expected_size != entry.key_size as usize {
            return Err(invalid_value(
                "Entries",
                "entry key size does not match the encoded key payload",
            ));
        }

        payload.extend_from_slice(&entry.key_size.to_be_bytes());
        payload.extend_from_slice(entry.key_namespace.as_bytes());
        payload.extend_from_slice(&entry.key_value);
    }
    Ok(payload)
}

fn decode_keys_entries(
    field_name: &'static str,
    payload: Vec<u8>,
    entry_count: u32,
) -> Result<Vec<Key>, FieldValueError> {
    let mut entries = Vec::with_capacity(untrusted_prealloc_hint(entry_count as usize));
    let mut offset = 0_usize;

    for _ in 0..entry_count {
        if offset + 8 > payload.len() {
            return Err(invalid_value(
                field_name,
                "entry payload length does not match the entry count",
            ));
        }

        let key_size = u32::from_be_bytes([
            payload[offset],
            payload[offset + 1],
            payload[offset + 2],
            payload[offset + 3],
        ]);
        if key_size < 8 {
            return Err(invalid_value(
                field_name,
                "entry key size is smaller than the fixed key header",
            ));
        }

        let value_len = (key_size - 8) as usize;
        let value_start = offset + 8;
        let value_end = value_start + value_len;
        if value_end > payload.len() {
            return Err(invalid_value(
                field_name,
                "entry payload length does not match the entry count",
            ));
        }

        let key_namespace = FourCc::from_bytes([
            payload[offset + 4],
            payload[offset + 5],
            payload[offset + 6],
            payload[offset + 7],
        ]);
        let key_value = payload[value_start..value_end].to_vec();
        entries.push(Key {
            key_size,
            key_namespace,
            key_value,
        });
        offset = value_end;
    }

    if offset != payload.len() {
        return Err(invalid_value(
            field_name,
            "entry payload length does not match the entry count",
        ));
    }

    Ok(entries)
}

fn render_key(key: &Key) -> String {
    format!(
        "{{KeySize={} KeyNamespace={} KeyValue={}}}",
        key.key_size,
        quote_fourcc(key.key_namespace),
        quote_bytes(&key.key_value)
    )
}

fn render_nested_data(data: &Data) -> Option<String> {
    stringify(data, None)
        .ok()
        .map(|rendered| format!("{{{rendered}}}"))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum NumberedMetadataLayout {
    #[default]
    InlineFields,
    NestedDataBox,
}

/// Returns `true` when `box_type` is one of the currently landed item-list metadata containers.
pub(crate) fn is_ilst_meta_box_type(box_type: FourCc) -> bool {
    ILST_META_BOX_TYPES.contains(&box_type)
}

/// Returns `true` when `box_type` falls into the numbered item range learned from `keys`.
#[allow(dead_code)]
pub(crate) fn is_numbered_metadata_item_type(
    box_type: FourCc,
    quicktime_keys_meta_entry_count: usize,
) -> bool {
    if quicktime_keys_meta_entry_count == 0 {
        return false;
    }

    let type_id = u32::from_be_bytes(box_type.into_bytes());
    type_id >= 1 && (type_id as usize) <= quicktime_keys_meta_entry_count
}

fn is_under_ilst_meta(context: BoxLookupContext) -> bool {
    context.under_ilst_meta
}

fn is_under_ilst_item(context: BoxLookupContext, item_type: FourCc) -> bool {
    context.under_ilst_meta && context.ilst_meta_item == Some(item_type)
}

fn is_under_track_number_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, TRACK_NUMBER_METADATA_ITEM_TYPE)
}

fn is_under_album_artist_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, ALBUM_ARTIST_METADATA_ITEM_TYPE)
}

fn is_under_account_kind_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, ACCOUNT_KIND_METADATA_ITEM_TYPE)
}

fn is_under_apple_id_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, APPLE_ID_METADATA_ITEM_TYPE)
}

fn is_under_artist_id_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, ARTIST_ID_METADATA_ITEM_TYPE)
}

fn is_under_cmid_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, CMID_METADATA_ITEM_TYPE)
}

fn is_under_cnid_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, CNID_METADATA_ITEM_TYPE)
}

fn is_under_description_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, DESCRIPTION_METADATA_ITEM_TYPE)
}

fn is_under_disk_number_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, DISK_NUMBER_METADATA_ITEM_TYPE)
}

fn is_under_episode_guid_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, EPISODE_GUID_METADATA_ITEM_TYPE)
}

fn is_under_genre_id_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, GENRE_ID_METADATA_ITEM_TYPE)
}

fn is_under_tempo_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, TEMPO_METADATA_ITEM_TYPE)
}

fn is_under_media_type_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, MEDIA_TYPE_METADATA_ITEM_TYPE)
}

fn is_under_compilation_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, COMPILATION_METADATA_ITEM_TYPE)
}

fn is_under_podcast_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, PODCAST_METADATA_ITEM_TYPE)
}

fn is_under_gapless_playback_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, GAPLESS_PLAYBACK_METADATA_ITEM_TYPE)
}

fn is_under_playlist_id_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, PLAYLIST_ID_METADATA_ITEM_TYPE)
}

fn is_under_purchase_date_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, PURCHASE_DATE_METADATA_ITEM_TYPE)
}

fn is_under_podcast_url_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, PODCAST_URL_METADATA_ITEM_TYPE)
}

fn is_under_rating_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, RATING_METADATA_ITEM_TYPE)
}

fn is_under_sfid_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, SFID_METADATA_ITEM_TYPE)
}

fn is_under_album_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, ALBUM_METADATA_ITEM_TYPE)
}

fn is_under_artist_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, ARTIST_METADATA_ITEM_TYPE)
}

fn is_under_comment_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, COMMENT_METADATA_ITEM_TYPE)
}

fn is_under_composer_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, COMPOSER_METADATA_ITEM_TYPE)
}

fn is_under_copyright_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, COPYRIGHT_METADATA_ITEM_TYPE)
}

fn is_under_day_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, DAY_METADATA_ITEM_TYPE)
}

fn is_under_genre_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, GENRE_METADATA_ITEM_TYPE)
}

fn is_under_legacy_genre_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, LEGACY_GENRE_METADATA_ITEM_TYPE)
}

fn is_under_grouping_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, GROUPING_METADATA_ITEM_TYPE)
}

fn is_under_name_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, NAME_METADATA_ITEM_TYPE)
}

fn is_under_tool_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, TOOL_METADATA_ITEM_TYPE)
}

fn is_under_sort_album_artist_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, SORT_ALBUM_ARTIST_METADATA_ITEM_TYPE)
}

fn is_under_sort_album_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, SORT_ALBUM_METADATA_ITEM_TYPE)
}

fn is_under_sort_artist_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, SORT_ARTIST_METADATA_ITEM_TYPE)
}

fn is_under_sort_composer_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, SORT_COMPOSER_METADATA_ITEM_TYPE)
}

fn is_under_sort_name_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, SORT_NAME_METADATA_ITEM_TYPE)
}

fn is_under_sort_show_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, SORT_SHOW_METADATA_ITEM_TYPE)
}

fn is_under_tv_episode_id_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, TV_EPISODE_ID_METADATA_ITEM_TYPE)
}

fn is_under_tv_episode_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, TV_EPISODE_METADATA_ITEM_TYPE)
}

fn is_under_tv_network_name_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, TV_NETWORK_NAME_METADATA_ITEM_TYPE)
}

fn is_under_tv_show_name_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, TV_SHOW_NAME_METADATA_ITEM_TYPE)
}

fn is_under_tv_season_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, TV_SEASON_METADATA_ITEM_TYPE)
}

fn is_under_writer_meta(context: BoxLookupContext) -> bool {
    is_under_ilst_item(context, WRITER_METADATA_ITEM_TYPE)
}

fn is_under_ilst_free_meta(context: BoxLookupContext) -> bool {
    context.under_ilst_free_meta
}

fn is_ilst_meta_container_context(context: BoxLookupContext) -> bool {
    context.under_ilst && !context.under_ilst_meta
}

fn is_numbered_ilst_item_context(box_type: FourCc, context: BoxLookupContext) -> bool {
    context.under_ilst
        && is_numbered_metadata_item_type(box_type, context.quicktime_keys_meta_entry_count)
}

/// Item-list container box carried under metadata roots.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ilst;

impl FieldHooks for Ilst {}

impl ImmutableBox for Ilst {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"ilst")
    }
}

impl MutableBox for Ilst {}

impl FieldValueRead for Ilst {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        Err(missing_field(field_name))
    }
}

impl FieldValueWrite for Ilst {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        Err(unexpected_field(field_name, value))
    }
}

impl CodecBox for Ilst {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[]);
}

/// Item-list metadata container whose runtime type comes from the parent `ilst` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IlstMetaContainer {
    box_type: FourCc,
}

impl Default for IlstMetaContainer {
    fn default() -> Self {
        Self {
            box_type: FREE_FORM_METADATA_ITEM_TYPE,
        }
    }
}

impl FieldHooks for IlstMetaContainer {}

impl ImmutableBox for IlstMetaContainer {
    fn box_type(&self) -> FourCc {
        self.box_type
    }
}

impl MutableBox for IlstMetaContainer {}

impl AnyTypeBox for IlstMetaContainer {
    fn set_box_type(&mut self, box_type: FourCc) {
        self.box_type = box_type;
    }
}

impl FieldValueRead for IlstMetaContainer {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        Err(missing_field(field_name))
    }
}

impl FieldValueWrite for IlstMetaContainer {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        Err(unexpected_field(field_name, value))
    }
}

impl CodecBox for IlstMetaContainer {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[]);
}

/// Metadata value box carried under item-list entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Data {
    pub data_type: u32,
    pub data_lang: u32,
    pub data: Vec<u8>,
}

/// Binary metadata payload type.
pub const DATA_TYPE_BINARY: u32 = 0;
/// UTF-8 string metadata payload type.
pub const DATA_TYPE_STRING_UTF8: u32 = 1;
/// UTF-16 string metadata payload type.
pub const DATA_TYPE_STRING_UTF16: u32 = 2;
/// Classic Mac string metadata payload type.
pub const DATA_TYPE_STRING_MAC: u32 = 3;
/// JPEG image metadata payload type.
pub const DATA_TYPE_STRING_JPEG: u32 = 14;
/// Big-endian signed integer metadata payload type.
pub const DATA_TYPE_SIGNED_INT_BIG_ENDIAN: u32 = 21;
/// Big-endian 32-bit float metadata payload type.
pub const DATA_TYPE_FLOAT32_BIG_ENDIAN: u32 = 22;
/// Big-endian 64-bit float metadata payload type.
pub const DATA_TYPE_FLOAT64_BIG_ENDIAN: u32 = 23;

impl FieldHooks for Data {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "DataType" => data_type_label(self.data_type).map(String::from),
            "Data" => display_utf8_bytes(self.data_type, &self.data),
            _ => None,
        }
    }
}

impl ImmutableBox for Data {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"data")
    }
}

impl MutableBox for Data {}

impl FieldValueRead for Data {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DataType" => Ok(FieldValue::Unsigned(u64::from(self.data_type))),
            "DataLang" => Ok(FieldValue::Unsigned(u64::from(self.data_lang))),
            "Data" => Ok(FieldValue::Bytes(self.data.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Data {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DataType", FieldValue::Unsigned(value)) => {
                self.data_type = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DataLang", FieldValue::Unsigned(value)) => {
                self.data_lang = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Data", FieldValue::Bytes(value)) => {
                self.data = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Data {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("DataType", 0, with_bit_width(32)),
        codec_field!("DataLang", 1, with_bit_width(32)),
        codec_field!("Data", 2, with_bit_width(8), as_bytes()),
    ]);
}

/// Track-number metadata value box carried under the `trkn` item container.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrackNumberData {
    pub data_type: u32,
    pub data_lang: u32,
    pub leading_reserved: u16,
    pub track_number: u16,
    pub total_tracks: u16,
    pub trailing_reserved: u16,
}

impl FieldHooks for TrackNumberData {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "DataType" => data_type_label(self.data_type).map(String::from),
            _ => None,
        }
    }
}

impl ImmutableBox for TrackNumberData {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"data")
    }
}

impl MutableBox for TrackNumberData {}

impl FieldValueRead for TrackNumberData {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DataType" => Ok(FieldValue::Unsigned(u64::from(self.data_type))),
            "DataLang" => Ok(FieldValue::Unsigned(u64::from(self.data_lang))),
            "LeadingReserved" => Ok(FieldValue::Unsigned(u64::from(self.leading_reserved))),
            "TrackNumber" => Ok(FieldValue::Unsigned(u64::from(self.track_number))),
            "TotalTracks" => Ok(FieldValue::Unsigned(u64::from(self.total_tracks))),
            "TrailingReserved" => Ok(FieldValue::Unsigned(u64::from(self.trailing_reserved))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for TrackNumberData {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DataType", FieldValue::Unsigned(value)) => {
                self.data_type = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DataLang", FieldValue::Unsigned(value)) => {
                self.data_lang = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("LeadingReserved", FieldValue::Unsigned(value)) => {
                self.leading_reserved = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("TrackNumber", FieldValue::Unsigned(value)) => {
                self.track_number = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("TotalTracks", FieldValue::Unsigned(value)) => {
                self.total_tracks = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("TrailingReserved", FieldValue::Unsigned(value)) => {
                self.trailing_reserved = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for TrackNumberData {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("DataType", 0, with_bit_width(32)),
        codec_field!("DataLang", 1, with_bit_width(32)),
        codec_field!(
            "LeadingReserved",
            2,
            with_bit_width(16),
            with_display_order(4)
        ),
        codec_field!("TrackNumber", 3, with_bit_width(16), with_display_order(2)),
        codec_field!("TotalTracks", 4, with_bit_width(16), with_display_order(3)),
        codec_field!(
            "TrailingReserved",
            5,
            with_bit_width(16),
            with_display_order(5)
        ),
    ]);
}

/// Disc-number metadata value box carried under the `disk` item container.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiskNumberData {
    pub data_type: u32,
    pub data_lang: u32,
    pub leading_reserved: u16,
    pub disk_number: u16,
    pub total_disks: u16,
    pub trailing_reserved: u16,
}

impl FieldHooks for DiskNumberData {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "DataType" => data_type_label(self.data_type).map(String::from),
            _ => None,
        }
    }
}

impl ImmutableBox for DiskNumberData {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"data")
    }
}

impl MutableBox for DiskNumberData {}

impl FieldValueRead for DiskNumberData {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DataType" => Ok(FieldValue::Unsigned(u64::from(self.data_type))),
            "DataLang" => Ok(FieldValue::Unsigned(u64::from(self.data_lang))),
            "LeadingReserved" => Ok(FieldValue::Unsigned(u64::from(self.leading_reserved))),
            "DiskNumber" => Ok(FieldValue::Unsigned(u64::from(self.disk_number))),
            "TotalDisks" => Ok(FieldValue::Unsigned(u64::from(self.total_disks))),
            "TrailingReserved" => Ok(FieldValue::Unsigned(u64::from(self.trailing_reserved))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for DiskNumberData {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DataType", FieldValue::Unsigned(value)) => {
                self.data_type = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DataLang", FieldValue::Unsigned(value)) => {
                self.data_lang = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("LeadingReserved", FieldValue::Unsigned(value)) => {
                self.leading_reserved = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DiskNumber", FieldValue::Unsigned(value)) => {
                self.disk_number = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("TotalDisks", FieldValue::Unsigned(value)) => {
                self.total_disks = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("TrailingReserved", FieldValue::Unsigned(value)) => {
                self.trailing_reserved = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for DiskNumberData {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("DataType", 0, with_bit_width(32)),
        codec_field!("DataLang", 1, with_bit_width(32)),
        codec_field!(
            "LeadingReserved",
            2,
            with_bit_width(16),
            with_display_order(4)
        ),
        codec_field!("DiskNumber", 3, with_bit_width(16), with_display_order(2)),
        codec_field!("TotalDisks", 4, with_bit_width(16), with_display_order(3)),
        codec_field!(
            "TrailingReserved",
            5,
            with_bit_width(16),
            with_display_order(5)
        ),
    ]);
}

/// Tempo metadata value box carried under the `tmpo` item container.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TempoData {
    pub data_type: u32,
    pub data_lang: u32,
    pub tempo: u16,
}

impl FieldHooks for TempoData {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "DataType" => data_type_label(self.data_type).map(String::from),
            _ => None,
        }
    }
}

impl ImmutableBox for TempoData {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"data")
    }
}

impl MutableBox for TempoData {}

impl FieldValueRead for TempoData {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DataType" => Ok(FieldValue::Unsigned(u64::from(self.data_type))),
            "DataLang" => Ok(FieldValue::Unsigned(u64::from(self.data_lang))),
            "Tempo" => Ok(FieldValue::Unsigned(u64::from(self.tempo))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for TempoData {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DataType", FieldValue::Unsigned(value)) => {
                self.data_type = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DataLang", FieldValue::Unsigned(value)) => {
                self.data_lang = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Tempo", FieldValue::Unsigned(value)) => {
                self.tempo = u16_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for TempoData {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("DataType", 0, with_bit_width(32)),
        codec_field!("DataLang", 1, with_bit_width(32)),
        codec_field!("Tempo", 2, with_bit_width(16)),
    ]);
}

/// Media-type metadata value box carried under the `stik` item container.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MediaTypeData {
    pub data_type: u32,
    pub data_lang: u32,
    pub media_type: u8,
}

impl FieldHooks for MediaTypeData {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "DataType" => data_type_label(self.data_type).map(String::from),
            _ => None,
        }
    }
}

impl ImmutableBox for MediaTypeData {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"data")
    }
}

impl MutableBox for MediaTypeData {}

impl FieldValueRead for MediaTypeData {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DataType" => Ok(FieldValue::Unsigned(u64::from(self.data_type))),
            "DataLang" => Ok(FieldValue::Unsigned(u64::from(self.data_lang))),
            "MediaType" => Ok(FieldValue::Unsigned(u64::from(self.media_type))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for MediaTypeData {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DataType", FieldValue::Unsigned(value)) => {
                self.data_type = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DataLang", FieldValue::Unsigned(value)) => {
                self.data_lang = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("MediaType", FieldValue::Unsigned(value)) => {
                self.media_type = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for MediaTypeData {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("DataType", 0, with_bit_width(32)),
        codec_field!("DataLang", 1, with_bit_width(32)),
        codec_field!("MediaType", 2, with_bit_width(8)),
    ]);
}

/// Compilation-flag metadata value box carried under the `cpil` item container.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompilationData {
    pub data_type: u32,
    pub data_lang: u32,
    pub is_compilation: bool,
}

impl FieldHooks for CompilationData {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "DataType" => data_type_label(self.data_type).map(String::from),
            "Compilation" => Some(self.is_compilation.to_string()),
            _ => None,
        }
    }
}

impl ImmutableBox for CompilationData {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"data")
    }
}

impl MutableBox for CompilationData {}

impl FieldValueRead for CompilationData {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DataType" => Ok(FieldValue::Unsigned(u64::from(self.data_type))),
            "DataLang" => Ok(FieldValue::Unsigned(u64::from(self.data_lang))),
            "Compilation" => Ok(FieldValue::Unsigned(unsigned_from_bool(
                self.is_compilation,
            ))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for CompilationData {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DataType", FieldValue::Unsigned(value)) => {
                self.data_type = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DataLang", FieldValue::Unsigned(value)) => {
                self.data_lang = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Compilation", FieldValue::Unsigned(value)) => {
                self.is_compilation = bool_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for CompilationData {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("DataType", 0, with_bit_width(32)),
        codec_field!("DataLang", 1, with_bit_width(32)),
        codec_field!("Compilation", 2, with_bit_width(8)),
    ]);
}

/// Podcast-flag metadata value box carried under the `pcst` item container.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PodcastData {
    pub data_type: u32,
    pub data_lang: u32,
    pub is_podcast: bool,
}

impl FieldHooks for PodcastData {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "DataType" => data_type_label(self.data_type).map(String::from),
            "Podcast" => Some(self.is_podcast.to_string()),
            _ => None,
        }
    }
}

impl ImmutableBox for PodcastData {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"data")
    }
}

impl MutableBox for PodcastData {}

impl FieldValueRead for PodcastData {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DataType" => Ok(FieldValue::Unsigned(u64::from(self.data_type))),
            "DataLang" => Ok(FieldValue::Unsigned(u64::from(self.data_lang))),
            "Podcast" => Ok(FieldValue::Unsigned(unsigned_from_bool(self.is_podcast))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for PodcastData {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DataType", FieldValue::Unsigned(value)) => {
                self.data_type = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DataLang", FieldValue::Unsigned(value)) => {
                self.data_lang = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Podcast", FieldValue::Unsigned(value)) => {
                self.is_podcast = bool_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for PodcastData {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("DataType", 0, with_bit_width(32)),
        codec_field!("DataLang", 1, with_bit_width(32)),
        codec_field!("Podcast", 2, with_bit_width(8)),
    ]);
}

/// Gapless-playback metadata value box carried under the `pgap` item container.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GaplessPlaybackData {
    pub data_type: u32,
    pub data_lang: u32,
    pub is_gapless_playback: bool,
}

impl FieldHooks for GaplessPlaybackData {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "DataType" => data_type_label(self.data_type).map(String::from),
            "GaplessPlayback" => Some(self.is_gapless_playback.to_string()),
            _ => None,
        }
    }
}

impl ImmutableBox for GaplessPlaybackData {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"data")
    }
}

impl MutableBox for GaplessPlaybackData {}

impl FieldValueRead for GaplessPlaybackData {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DataType" => Ok(FieldValue::Unsigned(u64::from(self.data_type))),
            "DataLang" => Ok(FieldValue::Unsigned(u64::from(self.data_lang))),
            "GaplessPlayback" => Ok(FieldValue::Unsigned(unsigned_from_bool(
                self.is_gapless_playback,
            ))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for GaplessPlaybackData {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DataType", FieldValue::Unsigned(value)) => {
                self.data_type = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DataLang", FieldValue::Unsigned(value)) => {
                self.data_lang = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("GaplessPlayback", FieldValue::Unsigned(value)) => {
                self.is_gapless_playback = bool_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for GaplessPlaybackData {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("DataType", 0, with_bit_width(32)),
        codec_field!("DataLang", 1, with_bit_width(32)),
        codec_field!("GaplessPlayback", 2, with_bit_width(8)),
    ]);
}

/// Rating metadata value box carried under the `rtng` item container.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RatingData {
    pub data_type: u32,
    pub data_lang: u32,
    pub rating: u8,
}

impl FieldHooks for RatingData {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "DataType" => data_type_label(self.data_type).map(String::from),
            _ => None,
        }
    }
}

impl ImmutableBox for RatingData {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"data")
    }
}

impl MutableBox for RatingData {}

impl FieldValueRead for RatingData {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DataType" => Ok(FieldValue::Unsigned(u64::from(self.data_type))),
            "DataLang" => Ok(FieldValue::Unsigned(u64::from(self.data_lang))),
            "Rating" => Ok(FieldValue::Unsigned(u64::from(self.rating))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for RatingData {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DataType", FieldValue::Unsigned(value)) => {
                self.data_type = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DataLang", FieldValue::Unsigned(value)) => {
                self.data_lang = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Rating", FieldValue::Unsigned(value)) => {
                self.rating = u8_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for RatingData {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("DataType", 0, with_bit_width(32)),
        codec_field!("DataLang", 1, with_bit_width(32)),
        codec_field!("Rating", 2, with_bit_width(8)),
    ]);
}

macro_rules! define_integer_metadata_data_box {
    ($(#[$doc:meta])* $type_name:ident, $field_name:literal, $field_ident:ident, $field_type:ty, $convert_fn:ident, $bit_width:expr) => {
        $(#[$doc])*
        #[derive(Clone, Debug, Default, PartialEq, Eq)]
        pub struct $type_name {
            pub data_type: u32,
            pub data_lang: u32,
            pub $field_ident: $field_type,
        }

        impl FieldHooks for $type_name {
            fn display_field(&self, name: &'static str) -> Option<String> {
                match name {
                    "DataType" => data_type_label(self.data_type).map(String::from),
                    _ => None,
                }
            }
        }

        impl ImmutableBox for $type_name {
            fn box_type(&self) -> FourCc {
                FourCc::from_bytes(*b"data")
            }
        }

        impl MutableBox for $type_name {}

        impl FieldValueRead for $type_name {
            fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
                match field_name {
                    "DataType" => Ok(FieldValue::Unsigned(u64::from(self.data_type))),
                    "DataLang" => Ok(FieldValue::Unsigned(u64::from(self.data_lang))),
                    $field_name => Ok(FieldValue::Unsigned(self.$field_ident.into())),
                    _ => Err(missing_field(field_name)),
                }
            }
        }

        impl FieldValueWrite for $type_name {
            fn set_field_value(
                &mut self,
                field_name: &'static str,
                value: FieldValue,
            ) -> Result<(), FieldValueError> {
                match (field_name, value) {
                    ("DataType", FieldValue::Unsigned(value)) => {
                        self.data_type = u32_from_unsigned(field_name, value)?;
                        Ok(())
                    }
                    ("DataLang", FieldValue::Unsigned(value)) => {
                        self.data_lang = u32_from_unsigned(field_name, value)?;
                        Ok(())
                    }
                    ($field_name, FieldValue::Unsigned(value)) => {
                        self.$field_ident = $convert_fn(field_name, value)?;
                        Ok(())
                    }
                    (field_name, value) => Err(unexpected_field(field_name, value)),
                }
            }
        }

        impl CodecBox for $type_name {
            const FIELD_TABLE: FieldTable = FieldTable::new(&[
                codec_field!("DataType", 0, with_bit_width(32)),
                codec_field!("DataLang", 1, with_bit_width(32)),
                codec_field!($field_name, 2, with_bit_width($bit_width)),
            ]);
        }
    };
}

macro_rules! define_utf8_metadata_data_box {
    ($(#[$doc:meta])* $type_name:ident, $field_name:literal, $field_ident:ident) => {
        $(#[$doc])*
        #[derive(Clone, Debug, Default, PartialEq, Eq)]
        pub struct $type_name {
            pub data_type: u32,
            pub data_lang: u32,
            pub $field_ident: Vec<u8>,
        }

        impl FieldHooks for $type_name {
            fn display_field(&self, name: &'static str) -> Option<String> {
                match name {
                    "DataType" => data_type_label(self.data_type).map(String::from),
                    $field_name => display_utf8_bytes(self.data_type, &self.$field_ident),
                    _ => None,
                }
            }
        }

        impl ImmutableBox for $type_name {
            fn box_type(&self) -> FourCc {
                FourCc::from_bytes(*b"data")
            }
        }

        impl MutableBox for $type_name {}

        impl FieldValueRead for $type_name {
            fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
                match field_name {
                    "DataType" => Ok(FieldValue::Unsigned(u64::from(self.data_type))),
                    "DataLang" => Ok(FieldValue::Unsigned(u64::from(self.data_lang))),
                    $field_name => Ok(FieldValue::Bytes(self.$field_ident.clone())),
                    _ => Err(missing_field(field_name)),
                }
            }
        }

        impl FieldValueWrite for $type_name {
            fn set_field_value(
                &mut self,
                field_name: &'static str,
                value: FieldValue,
            ) -> Result<(), FieldValueError> {
                match (field_name, value) {
                    ("DataType", FieldValue::Unsigned(value)) => {
                        self.data_type = u32_from_unsigned(field_name, value)?;
                        Ok(())
                    }
                    ("DataLang", FieldValue::Unsigned(value)) => {
                        self.data_lang = u32_from_unsigned(field_name, value)?;
                        Ok(())
                    }
                    ($field_name, FieldValue::Bytes(value)) => {
                        self.$field_ident = value;
                        Ok(())
                    }
                    (field_name, value) => Err(unexpected_field(field_name, value)),
                }
            }
        }

        impl CodecBox for $type_name {
            const FIELD_TABLE: FieldTable = FieldTable::new(&[
                codec_field!("DataType", 0, with_bit_width(32)),
                codec_field!("DataLang", 1, with_bit_width(32)),
                codec_field!($field_name, 2, with_bit_width(8), as_bytes()),
            ]);
        }
    };
}

define_integer_metadata_data_box!(
    /// Account-kind metadata value box carried under the `akID` item container.
    AccountKindData,
    "AccountKind",
    account_kind,
    u8,
    u8_from_unsigned,
    8
);

define_utf8_metadata_data_box!(
    /// Apple-ID metadata value box carried under the `apID` item container.
    AppleIdData,
    "AppleId",
    apple_id
);

define_integer_metadata_data_box!(
    /// Artist-ID metadata value box carried under the `atID` item container.
    ArtistIdData,
    "ArtistId",
    artist_id,
    u32,
    u32_from_unsigned,
    32
);

define_integer_metadata_data_box!(
    /// `cmID` metadata value box carried under the `cmID` item container.
    CmIdData,
    "CmId",
    cmid,
    u32,
    u32_from_unsigned,
    32
);

define_integer_metadata_data_box!(
    /// `cnID` metadata value box carried under the `cnID` item container.
    CnIdData,
    "CnId",
    cnid,
    u32,
    u32_from_unsigned,
    32
);

define_utf8_metadata_data_box!(
    /// Album-artist metadata value box carried under the `aART` item container.
    AlbumArtistData,
    "AlbumArtist",
    album_artist
);

define_utf8_metadata_data_box!(
    /// Description metadata value box carried under the `desc` item container.
    DescriptionData,
    "Description",
    description
);

define_utf8_metadata_data_box!(
    /// Episode-GUID metadata value box carried under the `egid` item container.
    EpisodeGuidData,
    "EpisodeGuid",
    episode_guid
);

define_integer_metadata_data_box!(
    /// Genre-ID metadata value box carried under the `geID` item container.
    GenreIdData,
    "GenreId",
    genre_id,
    u32,
    u32_from_unsigned,
    32
);

define_utf8_metadata_data_box!(
    /// Album metadata value box carried under the `(c)alb` item container.
    AlbumData,
    "Album",
    album
);

define_utf8_metadata_data_box!(
    /// Artist metadata value box carried under the `(c)ART` item container.
    ArtistData,
    "Artist",
    artist
);

define_utf8_metadata_data_box!(
    /// Comment metadata value box carried under the `(c)cmt` item container.
    CommentData,
    "Comment",
    comment
);

define_utf8_metadata_data_box!(
    /// Composer metadata value box carried under the `(c)com` item container.
    ComposerData,
    "Composer",
    composer
);

define_utf8_metadata_data_box!(
    /// Copyright metadata value box carried under the `cprt` item container.
    CopyrightData,
    "Copyright",
    copyright
);

define_utf8_metadata_data_box!(
    /// Date metadata value box carried under the `(c)day` item container.
    DateData,
    "Date",
    date
);

define_utf8_metadata_data_box!(
    /// Genre metadata value box carried under the `(c)gen` item container.
    GenreData,
    "Genre",
    genre
);

define_integer_metadata_data_box!(
    /// Legacy-genre metadata value box carried under the `gnre` item container.
    LegacyGenreData,
    "LegacyGenre",
    legacy_genre,
    u16,
    u16_from_unsigned,
    16
);

define_utf8_metadata_data_box!(
    /// Grouping metadata value box carried under the `(c)grp` item container.
    GroupingData,
    "Grouping",
    grouping
);

define_utf8_metadata_data_box!(
    /// Name metadata value box carried under the `(c)nam` item container.
    NameData,
    "Name",
    name
);

define_utf8_metadata_data_box!(
    /// Encoding-tool metadata value box carried under the `(c)too` item container.
    EncodingToolData,
    "EncodingTool",
    encoding_tool
);

define_utf8_metadata_data_box!(
    /// Writer metadata value box carried under the `(c)wrt` item container.
    WriterData,
    "Writer",
    writer
);

define_integer_metadata_data_box!(
    /// Playlist-ID metadata value box carried under the `plID` item container.
    PlaylistIdData,
    "PlaylistId",
    playlist_id,
    u64,
    u64_from_unsigned,
    64
);

define_utf8_metadata_data_box!(
    /// Purchase-date metadata value box carried under the `purd` item container.
    PurchaseDateData,
    "PurchaseDate",
    purchase_date
);

define_utf8_metadata_data_box!(
    /// Podcast-URL metadata value box carried under the `purl` item container.
    PodcastUrlData,
    "PodcastUrl",
    podcast_url
);

define_integer_metadata_data_box!(
    /// `sfID` metadata value box carried under the `sfID` item container.
    SfIdData,
    "SfId",
    sfid,
    u32,
    u32_from_unsigned,
    32
);

define_utf8_metadata_data_box!(
    /// Sort-album-artist metadata value box carried under the `soaa` item container.
    SortAlbumArtistData,
    "SortAlbumArtist",
    sort_album_artist
);

define_utf8_metadata_data_box!(
    /// Sort-album metadata value box carried under the `soal` item container.
    SortAlbumData,
    "SortAlbum",
    sort_album
);

define_utf8_metadata_data_box!(
    /// Sort-artist metadata value box carried under the `soar` item container.
    SortArtistData,
    "SortArtist",
    sort_artist
);

define_utf8_metadata_data_box!(
    /// Sort-composer metadata value box carried under the `soco` item container.
    SortComposerData,
    "SortComposer",
    sort_composer
);

define_utf8_metadata_data_box!(
    /// Sort-name metadata value box carried under the `sonm` item container.
    SortNameData,
    "SortName",
    sort_name
);

define_utf8_metadata_data_box!(
    /// Sort-show metadata value box carried under the `sosn` item container.
    SortShowData,
    "SortShow",
    sort_show
);

/// Television-episode identifier metadata value box carried under the `tven` item container.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TvEpisodeIdData {
    pub data_type: u32,
    pub data_lang: u32,
    pub tv_episode_id: Vec<u8>,
}

impl FieldHooks for TvEpisodeIdData {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "DataType" => data_type_label(self.data_type).map(String::from),
            "TvEpisodeId" => display_utf8_bytes(self.data_type, &self.tv_episode_id),
            _ => None,
        }
    }
}

impl ImmutableBox for TvEpisodeIdData {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"data")
    }
}

impl MutableBox for TvEpisodeIdData {}

impl FieldValueRead for TvEpisodeIdData {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DataType" => Ok(FieldValue::Unsigned(u64::from(self.data_type))),
            "DataLang" => Ok(FieldValue::Unsigned(u64::from(self.data_lang))),
            "TvEpisodeId" => Ok(FieldValue::Bytes(self.tv_episode_id.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for TvEpisodeIdData {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DataType", FieldValue::Unsigned(value)) => {
                self.data_type = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DataLang", FieldValue::Unsigned(value)) => {
                self.data_lang = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("TvEpisodeId", FieldValue::Bytes(value)) => {
                self.tv_episode_id = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for TvEpisodeIdData {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("DataType", 0, with_bit_width(32)),
        codec_field!("DataLang", 1, with_bit_width(32)),
        codec_field!("TvEpisodeId", 2, with_bit_width(8), as_bytes()),
    ]);
}

/// Television-episode metadata value box carried under the `tves` item container.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TvEpisodeData {
    pub data_type: u32,
    pub data_lang: u32,
    pub tv_episode: u32,
}

impl FieldHooks for TvEpisodeData {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "DataType" => data_type_label(self.data_type).map(String::from),
            _ => None,
        }
    }
}

impl ImmutableBox for TvEpisodeData {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"data")
    }
}

impl MutableBox for TvEpisodeData {}

impl FieldValueRead for TvEpisodeData {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DataType" => Ok(FieldValue::Unsigned(u64::from(self.data_type))),
            "DataLang" => Ok(FieldValue::Unsigned(u64::from(self.data_lang))),
            "TvEpisode" => Ok(FieldValue::Unsigned(u64::from(self.tv_episode))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for TvEpisodeData {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DataType", FieldValue::Unsigned(value)) => {
                self.data_type = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DataLang", FieldValue::Unsigned(value)) => {
                self.data_lang = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("TvEpisode", FieldValue::Unsigned(value)) => {
                self.tv_episode = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for TvEpisodeData {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("DataType", 0, with_bit_width(32)),
        codec_field!("DataLang", 1, with_bit_width(32)),
        codec_field!("TvEpisode", 2, with_bit_width(32)),
    ]);
}

/// Television-network-name metadata value box carried under the `tvnn` item container.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TvNetworkNameData {
    pub data_type: u32,
    pub data_lang: u32,
    pub tv_network_name: Vec<u8>,
}

impl FieldHooks for TvNetworkNameData {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "DataType" => data_type_label(self.data_type).map(String::from),
            "TvNetworkName" => display_utf8_bytes(self.data_type, &self.tv_network_name),
            _ => None,
        }
    }
}

impl ImmutableBox for TvNetworkNameData {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"data")
    }
}

impl MutableBox for TvNetworkNameData {}

impl FieldValueRead for TvNetworkNameData {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DataType" => Ok(FieldValue::Unsigned(u64::from(self.data_type))),
            "DataLang" => Ok(FieldValue::Unsigned(u64::from(self.data_lang))),
            "TvNetworkName" => Ok(FieldValue::Bytes(self.tv_network_name.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for TvNetworkNameData {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DataType", FieldValue::Unsigned(value)) => {
                self.data_type = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DataLang", FieldValue::Unsigned(value)) => {
                self.data_lang = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("TvNetworkName", FieldValue::Bytes(value)) => {
                self.tv_network_name = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for TvNetworkNameData {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("DataType", 0, with_bit_width(32)),
        codec_field!("DataLang", 1, with_bit_width(32)),
        codec_field!("TvNetworkName", 2, with_bit_width(8), as_bytes()),
    ]);
}

/// Television-show-name metadata value box carried under the `tvsh` item container.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TvShowNameData {
    pub data_type: u32,
    pub data_lang: u32,
    pub tv_show_name: Vec<u8>,
}

impl FieldHooks for TvShowNameData {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "DataType" => data_type_label(self.data_type).map(String::from),
            "TvShowName" => display_utf8_bytes(self.data_type, &self.tv_show_name),
            _ => None,
        }
    }
}

impl ImmutableBox for TvShowNameData {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"data")
    }
}

impl MutableBox for TvShowNameData {}

impl FieldValueRead for TvShowNameData {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DataType" => Ok(FieldValue::Unsigned(u64::from(self.data_type))),
            "DataLang" => Ok(FieldValue::Unsigned(u64::from(self.data_lang))),
            "TvShowName" => Ok(FieldValue::Bytes(self.tv_show_name.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for TvShowNameData {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DataType", FieldValue::Unsigned(value)) => {
                self.data_type = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DataLang", FieldValue::Unsigned(value)) => {
                self.data_lang = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("TvShowName", FieldValue::Bytes(value)) => {
                self.tv_show_name = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for TvShowNameData {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("DataType", 0, with_bit_width(32)),
        codec_field!("DataLang", 1, with_bit_width(32)),
        codec_field!("TvShowName", 2, with_bit_width(8), as_bytes()),
    ]);
}

/// Television-season metadata value box carried under the `tvsn` item container.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TvSeasonData {
    pub data_type: u32,
    pub data_lang: u32,
    pub tv_season: u32,
}

impl FieldHooks for TvSeasonData {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "DataType" => data_type_label(self.data_type).map(String::from),
            _ => None,
        }
    }
}

impl ImmutableBox for TvSeasonData {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"data")
    }
}

impl MutableBox for TvSeasonData {}

impl FieldValueRead for TvSeasonData {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "DataType" => Ok(FieldValue::Unsigned(u64::from(self.data_type))),
            "DataLang" => Ok(FieldValue::Unsigned(u64::from(self.data_lang))),
            "TvSeason" => Ok(FieldValue::Unsigned(u64::from(self.tv_season))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for TvSeasonData {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("DataType", FieldValue::Unsigned(value)) => {
                self.data_type = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("DataLang", FieldValue::Unsigned(value)) => {
                self.data_lang = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("TvSeason", FieldValue::Unsigned(value)) => {
                self.tv_season = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for TvSeasonData {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("DataType", 0, with_bit_width(32)),
        codec_field!("DataLang", 1, with_bit_width(32)),
        codec_field!("TvSeason", 2, with_bit_width(32)),
    ]);
}

/// Free-form metadata string leaf used by `mean` and `name`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StringData {
    box_type: FourCc,
    pub data: Vec<u8>,
}

impl Default for StringData {
    fn default() -> Self {
        Self {
            box_type: FourCc::from_bytes(*b"mean"),
            data: Vec::new(),
        }
    }
}

impl FieldHooks for StringData {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Data" => Some(quote_bytes(&self.data)),
            _ => None,
        }
    }
}

impl ImmutableBox for StringData {
    fn box_type(&self) -> FourCc {
        self.box_type
    }
}

impl MutableBox for StringData {}

impl AnyTypeBox for StringData {
    fn set_box_type(&mut self, box_type: FourCc) {
        self.box_type = box_type;
    }
}

impl FieldValueRead for StringData {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Data" => Ok(FieldValue::Bytes(self.data.clone())),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for StringData {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Data", FieldValue::Bytes(value)) => {
                self.data = value;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for StringData {
    const FIELD_TABLE: FieldTable =
        FieldTable::new(&[codec_field!("Data", 0, with_bit_width(8), as_bytes())]);
}

/// Numbered item-list metadata entry selected from the `keys` table.
#[derive(Clone, Debug)]
pub struct NumberedMetadataItem {
    box_type: FourCc,
    full_box: FullBoxState,
    layout: NumberedMetadataLayout,
    pub item_name: FourCc,
    pub data: Data,
}

impl PartialEq for NumberedMetadataItem {
    fn eq(&self, other: &Self) -> bool {
        self.box_type == other.box_type
            && self.full_box == other.full_box
            && self.item_name == other.item_name
            && self.data == other.data
    }
}

impl Eq for NumberedMetadataItem {}

impl Default for NumberedMetadataItem {
    fn default() -> Self {
        Self {
            box_type: FourCc::ANY,
            full_box: FullBoxState::default(),
            layout: NumberedMetadataLayout::InlineFields,
            item_name: FourCc::from_bytes(*b"data"),
            data: Data::default(),
        }
    }
}

impl FieldHooks for NumberedMetadataItem {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "ItemName" => Some(quote_fourcc(self.item_name)),
            "Data" => render_nested_data(&self.data),
            _ => None,
        }
    }
}

impl ImmutableBox for NumberedMetadataItem {
    fn box_type(&self) -> FourCc {
        self.box_type
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for NumberedMetadataItem {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }

    fn before_unmarshal(
        &mut self,
        _reader: &mut dyn ReadSeek,
        _payload_size: u64,
    ) -> Result<(), crate::codec::CodecError> {
        self.layout = NumberedMetadataLayout::InlineFields;
        Ok(())
    }
}

impl AnyTypeBox for NumberedMetadataItem {
    fn set_box_type(&mut self, box_type: FourCc) {
        self.box_type = box_type;
    }
}

impl FieldValueRead for NumberedMetadataItem {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "ItemName" => Ok(FieldValue::Bytes(self.item_name.as_bytes().to_vec())),
            "Data" => Ok(FieldValue::Bytes(encode_data_payload(&self.data))),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for NumberedMetadataItem {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("ItemName", FieldValue::Bytes(value)) => {
                self.item_name = bytes_to_fourcc(field_name, value)?;
                Ok(())
            }
            ("Data", FieldValue::Bytes(value)) => {
                self.data = decode_data_payload(field_name, value)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for NumberedMetadataItem {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("ItemName", 2, with_bit_width(8), with_length(4), as_bytes()),
        codec_field!("Data", 3, with_bit_width(8), as_bytes()),
    ]);

    fn custom_marshal(
        &self,
        writer: &mut dyn Write,
    ) -> Result<Option<u64>, crate::codec::CodecError> {
        if self.layout != NumberedMetadataLayout::NestedDataBox {
            return Ok(None);
        }

        let payload = encode_data_payload(&self.data);
        let size = 8 + payload.len() as u64;
        writer.write_all(&BoxInfo::new(self.item_name, size).encode())?;
        writer.write_all(&payload)?;
        Ok(Some(size))
    }

    fn custom_unmarshal(
        &mut self,
        reader: &mut dyn ReadSeek,
        payload_size: u64,
    ) -> Result<Option<u64>, crate::codec::CodecError> {
        if payload_size < 16 {
            return Ok(None);
        }

        let start = reader.stream_position()?;
        let mut header = [0_u8; 8];
        reader.read_exact(&mut header)?;

        let nested_size = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as u64;
        let item_name = FourCc::from_bytes([header[4], header[5], header[6], header[7]]);

        if nested_size != payload_size || item_name != FourCc::from_bytes(*b"data") {
            reader.seek(SeekFrom::Start(start))?;
            return Ok(None);
        }

        let data_payload = read_exact_vec_untrusted(reader, (payload_size - 8) as usize)?;

        self.full_box = FullBoxState::default();
        self.layout = NumberedMetadataLayout::NestedDataBox;
        self.item_name = item_name;
        self.data = decode_data_payload("Data", data_payload)?;
        Ok(Some(payload_size))
    }
}

/// Metadata key table that describes numbered item-list entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Keys {
    full_box: FullBoxState,
    pub entry_count: u32,
    pub entries: Vec<Key>,
}

/// One metadata-key record carried inside a `keys` box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    pub key_size: u32,
    pub key_namespace: FourCc,
    pub key_value: Vec<u8>,
}

impl Default for Key {
    fn default() -> Self {
        Self {
            key_size: 0,
            key_namespace: FourCc::ANY,
            key_value: Vec::new(),
        }
    }
}

impl FieldHooks for Keys {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Entries" => Some(render_array(self.entries.iter().map(render_key))),
            _ => None,
        }
    }
}

impl ImmutableBox for Keys {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"keys")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for Keys {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }
}

impl FieldValueRead for Keys {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "EntryCount" => Ok(FieldValue::Unsigned(u64::from(self.entry_count))),
            "Entries" => Ok(FieldValue::Bytes(encode_keys_entries(&self.entries)?)),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Keys {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("EntryCount", FieldValue::Unsigned(value)) => {
                self.entry_count = u32_from_unsigned(field_name, value)?;
                Ok(())
            }
            ("Entries", FieldValue::Bytes(value)) => {
                self.entries = decode_keys_entries(field_name, value, self.entry_count)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Keys {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("EntryCount", 2, with_bit_width(32)),
        codec_field!("Entries", 3, with_bit_width(8), as_bytes()),
    ]);
}

/// Registers the currently landed metadata boxes in `registry`.
pub fn register_boxes(registry: &mut BoxRegistry) {
    registry.register::<Ilst>(FourCc::from_bytes(*b"ilst"));
    registry.register::<Keys>(FourCc::from_bytes(*b"keys"));
    registry.register_contextual::<AccountKindData>(
        FourCc::from_bytes(*b"data"),
        is_under_account_kind_meta,
    );
    registry
        .register_contextual::<AppleIdData>(FourCc::from_bytes(*b"data"), is_under_apple_id_meta);
    registry.register_contextual::<AlbumArtistData>(
        FourCc::from_bytes(*b"data"),
        is_under_album_artist_meta,
    );
    registry
        .register_contextual::<ArtistIdData>(FourCc::from_bytes(*b"data"), is_under_artist_id_meta);
    registry.register_contextual::<CmIdData>(FourCc::from_bytes(*b"data"), is_under_cmid_meta);
    registry.register_contextual::<CnIdData>(FourCc::from_bytes(*b"data"), is_under_cnid_meta);
    registry.register_contextual::<DescriptionData>(
        FourCc::from_bytes(*b"data"),
        is_under_description_meta,
    );
    registry.register_contextual::<TrackNumberData>(
        FourCc::from_bytes(*b"data"),
        is_under_track_number_meta,
    );
    registry.register_contextual::<DiskNumberData>(
        FourCc::from_bytes(*b"data"),
        is_under_disk_number_meta,
    );
    registry.register_contextual::<EpisodeGuidData>(
        FourCc::from_bytes(*b"data"),
        is_under_episode_guid_meta,
    );
    registry
        .register_contextual::<GenreIdData>(FourCc::from_bytes(*b"data"), is_under_genre_id_meta);
    registry.register_contextual::<TempoData>(FourCc::from_bytes(*b"data"), is_under_tempo_meta);
    registry.register_contextual::<MediaTypeData>(
        FourCc::from_bytes(*b"data"),
        is_under_media_type_meta,
    );
    registry.register_contextual::<CompilationData>(
        FourCc::from_bytes(*b"data"),
        is_under_compilation_meta,
    );
    registry
        .register_contextual::<PodcastData>(FourCc::from_bytes(*b"data"), is_under_podcast_meta);
    registry.register_contextual::<GaplessPlaybackData>(
        FourCc::from_bytes(*b"data"),
        is_under_gapless_playback_meta,
    );
    registry.register_contextual::<PlaylistIdData>(
        FourCc::from_bytes(*b"data"),
        is_under_playlist_id_meta,
    );
    registry.register_contextual::<PurchaseDateData>(
        FourCc::from_bytes(*b"data"),
        is_under_purchase_date_meta,
    );
    registry.register_contextual::<PodcastUrlData>(
        FourCc::from_bytes(*b"data"),
        is_under_podcast_url_meta,
    );
    registry.register_contextual::<RatingData>(FourCc::from_bytes(*b"data"), is_under_rating_meta);
    registry.register_contextual::<SfIdData>(FourCc::from_bytes(*b"data"), is_under_sfid_meta);
    registry.register_contextual::<AlbumData>(FourCc::from_bytes(*b"data"), is_under_album_meta);
    registry.register_contextual::<ArtistData>(FourCc::from_bytes(*b"data"), is_under_artist_meta);
    registry
        .register_contextual::<CommentData>(FourCc::from_bytes(*b"data"), is_under_comment_meta);
    registry
        .register_contextual::<ComposerData>(FourCc::from_bytes(*b"data"), is_under_composer_meta);
    registry.register_contextual::<CopyrightData>(
        FourCc::from_bytes(*b"data"),
        is_under_copyright_meta,
    );
    registry.register_contextual::<DateData>(FourCc::from_bytes(*b"data"), is_under_day_meta);
    registry.register_contextual::<GenreData>(FourCc::from_bytes(*b"data"), is_under_genre_meta);
    registry.register_contextual::<LegacyGenreData>(
        FourCc::from_bytes(*b"data"),
        is_under_legacy_genre_meta,
    );
    registry
        .register_contextual::<GroupingData>(FourCc::from_bytes(*b"data"), is_under_grouping_meta);
    registry.register_contextual::<NameData>(FourCc::from_bytes(*b"data"), is_under_name_meta);
    registry
        .register_contextual::<EncodingToolData>(FourCc::from_bytes(*b"data"), is_under_tool_meta);
    registry.register_contextual::<SortAlbumArtistData>(
        FourCc::from_bytes(*b"data"),
        is_under_sort_album_artist_meta,
    );
    registry.register_contextual::<SortAlbumData>(
        FourCc::from_bytes(*b"data"),
        is_under_sort_album_meta,
    );
    registry.register_contextual::<SortArtistData>(
        FourCc::from_bytes(*b"data"),
        is_under_sort_artist_meta,
    );
    registry.register_contextual::<SortComposerData>(
        FourCc::from_bytes(*b"data"),
        is_under_sort_composer_meta,
    );
    registry
        .register_contextual::<SortNameData>(FourCc::from_bytes(*b"data"), is_under_sort_name_meta);
    registry
        .register_contextual::<SortShowData>(FourCc::from_bytes(*b"data"), is_under_sort_show_meta);
    registry.register_contextual::<TvEpisodeIdData>(
        FourCc::from_bytes(*b"data"),
        is_under_tv_episode_id_meta,
    );
    registry.register_contextual::<TvEpisodeData>(
        FourCc::from_bytes(*b"data"),
        is_under_tv_episode_meta,
    );
    registry.register_contextual::<TvNetworkNameData>(
        FourCc::from_bytes(*b"data"),
        is_under_tv_network_name_meta,
    );
    registry.register_contextual::<TvShowNameData>(
        FourCc::from_bytes(*b"data"),
        is_under_tv_show_name_meta,
    );
    registry
        .register_contextual::<TvSeasonData>(FourCc::from_bytes(*b"data"), is_under_tv_season_meta);
    registry.register_contextual::<WriterData>(FourCc::from_bytes(*b"data"), is_under_writer_meta);
    registry.register_contextual::<Data>(FourCc::from_bytes(*b"data"), is_under_ilst_meta);
    for box_type in ILST_META_BOX_TYPES.iter().copied() {
        registry
            .register_contextual_any::<IlstMetaContainer>(box_type, is_ilst_meta_container_context);
    }
    registry.register_contextual_any::<StringData>(
        FourCc::from_bytes(*b"mean"),
        is_under_ilst_free_meta,
    );
    registry.register_contextual_any::<StringData>(
        FourCc::from_bytes(*b"name"),
        is_under_ilst_free_meta,
    );
    registry.register_dynamic_any::<NumberedMetadataItem>(is_numbered_ilst_item_context);
}
