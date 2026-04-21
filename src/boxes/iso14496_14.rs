//! ISO/IEC 14496-14 ES descriptor boxes.

use std::io::{Cursor, Read};

use crate::boxes::BoxRegistry;
use crate::codec::{
    CodecBox, FieldHooks, FieldTable, FieldValue, FieldValueError, FieldValueRead, FieldValueWrite,
    ImmutableBox, MutableBox, read_exact_vec_untrusted,
};
use crate::{FourCc, codec_field};

/// Descriptor tag used by the elementary-stream descriptor record.
pub const ES_DESCRIPTOR_TAG: u8 = 0x03;
/// Descriptor tag used by the decoder-configuration descriptor record.
pub const DECODER_CONFIG_DESCRIPTOR_TAG: u8 = 0x04;
/// Descriptor tag used by raw decoder-specific configuration bytes.
pub const DECODER_SPECIFIC_INFO_TAG: u8 = 0x05;
/// Descriptor tag used by the sync-layer configuration descriptor record.
pub const SL_CONFIG_DESCRIPTOR_TAG: u8 = 0x06;

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

fn render_array(values: impl IntoIterator<Item = String>) -> String {
    let values = values.into_iter().collect::<Vec<_>>();
    format!("[{}]", values.join(", "))
}

fn render_hex_bytes(bytes: &[u8]) -> String {
    render_array(bytes.iter().map(|byte| format!("0x{byte:x}")))
}

fn quote_bytes(bytes: &[u8]) -> String {
    format!("\"{}\"", escape_bytes(bytes))
}

fn escape_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| escape_display_char(char::from(*byte)))
        .collect()
}

fn escape_display_char(value: char) -> char {
    if value.is_control() || (!value.is_ascii_graphic() && value != ' ') {
        '.'
    } else {
        value
    }
}

fn read_u8(reader: &mut Cursor<&[u8]>, field_name: &'static str) -> Result<u8, FieldValueError> {
    let mut buf = [0_u8; 1];
    reader
        .read_exact(&mut buf)
        .map_err(|_| invalid_value(field_name, "descriptor stream is truncated"))?;
    Ok(buf[0])
}

fn read_u16(reader: &mut Cursor<&[u8]>, field_name: &'static str) -> Result<u16, FieldValueError> {
    let mut buf = [0_u8; 2];
    reader
        .read_exact(&mut buf)
        .map_err(|_| invalid_value(field_name, "descriptor stream is truncated"))?;
    Ok(u16::from_be_bytes(buf))
}

fn read_u24(reader: &mut Cursor<&[u8]>, field_name: &'static str) -> Result<u32, FieldValueError> {
    let mut buf = [0_u8; 3];
    reader
        .read_exact(&mut buf)
        .map_err(|_| invalid_value(field_name, "descriptor stream is truncated"))?;
    Ok((u32::from(buf[0]) << 16) | (u32::from(buf[1]) << 8) | u32::from(buf[2]))
}

fn read_u32(reader: &mut Cursor<&[u8]>, field_name: &'static str) -> Result<u32, FieldValueError> {
    let mut buf = [0_u8; 4];
    reader
        .read_exact(&mut buf)
        .map_err(|_| invalid_value(field_name, "descriptor stream is truncated"))?;
    Ok(u32::from_be_bytes(buf))
}

fn read_exact_bytes(
    reader: &mut Cursor<&[u8]>,
    len: usize,
    field_name: &'static str,
) -> Result<Vec<u8>, FieldValueError> {
    read_exact_vec_untrusted(reader, len)
        .map_err(|_| invalid_value(field_name, "descriptor payload is truncated"))
}

fn read_uvarint(
    reader: &mut Cursor<&[u8]>,
    field_name: &'static str,
) -> Result<u32, FieldValueError> {
    let mut value = 0_u64;
    loop {
        let octet = read_u8(reader, field_name)?;
        value = (value << 7) | u64::from(octet & 0x7f);
        if value > u64::from(u32::MAX) {
            return Err(invalid_value(field_name, "value does not fit in u32"));
        }
        if octet & 0x80 == 0 {
            return Ok(value as u32);
        }
    }
}

fn write_u16(buffer: &mut Vec<u8>, value: u16) {
    buffer.extend_from_slice(&value.to_be_bytes());
}

fn write_u24(buffer: &mut Vec<u8>, value: u32) {
    buffer.extend_from_slice(&[
        ((value >> 16) & 0xff) as u8,
        ((value >> 8) & 0xff) as u8,
        (value & 0xff) as u8,
    ]);
}

fn write_u32(buffer: &mut Vec<u8>, value: u32) {
    buffer.extend_from_slice(&value.to_be_bytes());
}

fn write_uvarint(
    buffer: &mut Vec<u8>,
    field_name: &'static str,
    value: u32,
) -> Result<(), FieldValueError> {
    if value > 0x0fff_ffff {
        return Err(invalid_value(
            field_name,
            "value does not fit in the four-octet descriptor varint",
        ));
    }

    for shift in [21_u32, 14, 7] {
        let octet = (((value >> shift) as u8) & 0x7f) | 0x80;
        buffer.push(octet);
    }
    buffer.push((value & 0x7f) as u8);
    Ok(())
}

fn descriptor_tag_name(tag: u8) -> Option<&'static str> {
    match tag {
        ES_DESCRIPTOR_TAG => Some("ESDescr"),
        DECODER_CONFIG_DESCRIPTOR_TAG => Some("DecoderConfigDescr"),
        DECODER_SPECIFIC_INFO_TAG => Some("DecSpecificInfo"),
        SL_CONFIG_DESCRIPTOR_TAG => Some("SLConfigDescr"),
        _ => None,
    }
}

fn render_descriptor_tag(tag: u8) -> String {
    descriptor_tag_name(tag)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("0x{tag:x}"))
}

fn encode_es_descriptor(
    field_name: &'static str,
    descriptor: &EsDescriptor,
) -> Result<Vec<u8>, FieldValueError> {
    if descriptor.stream_priority > 0x1f {
        return Err(invalid_value(
            field_name,
            "stream priority must fit in 5 bits",
        ));
    }
    if descriptor.url_flag && usize::from(descriptor.url_length) != descriptor.url_string.len() {
        return Err(invalid_value(
            "URLString",
            "value length does not match URLLength",
        ));
    }

    let mut buffer = Vec::new();
    write_u16(&mut buffer, descriptor.es_id);
    let mut packed = descriptor.stream_priority & 0x1f;
    if descriptor.stream_dependence_flag {
        packed |= 0x80;
    }
    if descriptor.url_flag {
        packed |= 0x40;
    }
    if descriptor.ocr_stream_flag {
        packed |= 0x20;
    }
    buffer.push(packed);
    if descriptor.stream_dependence_flag {
        write_u16(&mut buffer, descriptor.depends_on_es_id);
    }
    if descriptor.url_flag {
        buffer.push(descriptor.url_length);
        buffer.extend_from_slice(&descriptor.url_string);
    }
    if descriptor.ocr_stream_flag {
        write_u16(&mut buffer, descriptor.ocr_es_id);
    }
    Ok(buffer)
}

fn encode_decoder_config_descriptor(
    field_name: &'static str,
    descriptor: &DecoderConfigDescriptor,
) -> Result<Vec<u8>, FieldValueError> {
    if descriptor.stream_type > 0x3f {
        return Err(invalid_value(field_name, "stream type must fit in 6 bits"));
    }
    if descriptor.buffer_size_db > 0x00ff_ffff {
        return Err(invalid_value(
            "BufferSizeDB",
            "value does not fit in 24 bits",
        ));
    }

    let mut buffer = Vec::new();
    buffer.push(descriptor.object_type_indication);
    let packed = (descriptor.stream_type << 2)
        | (u8::from(descriptor.up_stream) << 1)
        | u8::from(descriptor.reserved);
    buffer.push(packed);
    write_u24(&mut buffer, descriptor.buffer_size_db);
    write_u32(&mut buffer, descriptor.max_bitrate);
    write_u32(&mut buffer, descriptor.avg_bitrate);
    Ok(buffer)
}

fn encode_descriptor_stream(descriptors: &[Descriptor]) -> Result<Vec<u8>, FieldValueError> {
    let mut buffer = Vec::new();
    for descriptor in descriptors {
        buffer.push(descriptor.tag);
        write_uvarint(&mut buffer, "Size", descriptor.size)?;
        match descriptor.tag {
            ES_DESCRIPTOR_TAG => {
                let nested = descriptor.es_descriptor.as_ref().ok_or_else(|| {
                    invalid_value("ESDescriptor", "descriptor payload is missing")
                })?;
                buffer.extend_from_slice(&encode_es_descriptor("ESDescriptor", nested)?);
            }
            DECODER_CONFIG_DESCRIPTOR_TAG => {
                let nested = descriptor
                    .decoder_config_descriptor
                    .as_ref()
                    .ok_or_else(|| {
                        invalid_value("DecoderConfigDescriptor", "descriptor payload is missing")
                    })?;
                buffer.extend_from_slice(&encode_decoder_config_descriptor(
                    "DecoderConfigDescriptor",
                    nested,
                )?);
            }
            _ => {
                if descriptor.data.len() != descriptor.size as usize {
                    return Err(invalid_value("Data", "value length does not match Size"));
                }
                buffer.extend_from_slice(&descriptor.data);
            }
        }
    }
    Ok(buffer)
}

fn parse_es_descriptor(
    field_name: &'static str,
    reader: &mut Cursor<&[u8]>,
) -> Result<EsDescriptor, FieldValueError> {
    let es_id = read_u16(reader, field_name)?;
    let packed = read_u8(reader, field_name)?;
    let stream_dependence_flag = packed & 0x80 != 0;
    let url_flag = packed & 0x40 != 0;
    let ocr_stream_flag = packed & 0x20 != 0;
    let stream_priority = packed & 0x1f;

    let depends_on_es_id = if stream_dependence_flag {
        read_u16(reader, field_name)?
    } else {
        0
    };
    let (url_length, url_string) = if url_flag {
        let url_length = read_u8(reader, field_name)?;
        let url_string = read_exact_bytes(reader, usize::from(url_length), field_name)?;
        (url_length, url_string)
    } else {
        (0, Vec::new())
    };
    let ocr_es_id = if ocr_stream_flag {
        read_u16(reader, field_name)?
    } else {
        0
    };

    Ok(EsDescriptor {
        es_id,
        stream_dependence_flag,
        url_flag,
        ocr_stream_flag,
        stream_priority,
        depends_on_es_id,
        url_length,
        url_string,
        ocr_es_id,
    })
}

fn parse_decoder_config_descriptor(
    field_name: &'static str,
    reader: &mut Cursor<&[u8]>,
) -> Result<DecoderConfigDescriptor, FieldValueError> {
    let object_type_indication = read_u8(reader, field_name)?;
    let packed = read_u8(reader, field_name)?;
    let stream_type = packed >> 2;
    let up_stream = packed & 0x02 != 0;
    let reserved = packed & 0x01 != 0;
    let buffer_size_db = read_u24(reader, field_name)?;
    let max_bitrate = read_u32(reader, field_name)?;
    let avg_bitrate = read_u32(reader, field_name)?;

    Ok(DecoderConfigDescriptor {
        object_type_indication,
        stream_type,
        up_stream,
        reserved,
        buffer_size_db,
        max_bitrate,
        avg_bitrate,
    })
}

fn parse_descriptor_stream(
    field_name: &'static str,
    bytes: &[u8],
) -> Result<Vec<Descriptor>, FieldValueError> {
    let mut reader = Cursor::new(bytes);
    let mut descriptors = Vec::new();

    while reader.position() < bytes.len() as u64 {
        let tag = read_u8(&mut reader, field_name)?;
        let size = read_uvarint(&mut reader, "Size")?;
        let mut descriptor = Descriptor {
            tag,
            size,
            ..Descriptor::default()
        };

        match tag {
            ES_DESCRIPTOR_TAG => {
                descriptor.es_descriptor = Some(parse_es_descriptor("ESDescriptor", &mut reader)?);
            }
            DECODER_CONFIG_DESCRIPTOR_TAG => {
                descriptor.decoder_config_descriptor = Some(parse_decoder_config_descriptor(
                    "DecoderConfigDescriptor",
                    &mut reader,
                )?);
            }
            _ => {
                descriptor.data = read_exact_bytes(&mut reader, size as usize, field_name)?;
            }
        }

        descriptors.push(descriptor);
    }

    Ok(descriptors)
}

fn render_es_descriptor(descriptor: &EsDescriptor) -> String {
    let mut fields = vec![
        format!("ESID={}", descriptor.es_id),
        format!("StreamDependenceFlag={}", descriptor.stream_dependence_flag),
        format!("UrlFlag={}", descriptor.url_flag),
        format!("OcrStreamFlag={}", descriptor.ocr_stream_flag),
        format!("StreamPriority={}", descriptor.stream_priority),
    ];

    if descriptor.stream_dependence_flag {
        fields.push(format!("DependsOnESID={}", descriptor.depends_on_es_id));
    }
    if descriptor.url_flag {
        fields.push(format!("URLLength=0x{:x}", descriptor.url_length));
        fields.push(format!("URLString={}", quote_bytes(&descriptor.url_string)));
    }
    if descriptor.ocr_stream_flag {
        fields.push(format!("OCRESID={}", descriptor.ocr_es_id));
    }

    fields.join(" ")
}

fn render_decoder_config_descriptor(descriptor: &DecoderConfigDescriptor) -> String {
    [
        format!(
            "ObjectTypeIndication=0x{:x}",
            descriptor.object_type_indication
        ),
        format!("StreamType={}", descriptor.stream_type),
        format!("UpStream={}", descriptor.up_stream),
        format!("Reserved={}", descriptor.reserved),
        format!("BufferSizeDB={}", descriptor.buffer_size_db),
        format!("MaxBitrate={}", descriptor.max_bitrate),
        format!("AvgBitrate={}", descriptor.avg_bitrate),
    ]
    .join(" ")
}

fn render_descriptor(descriptor: &Descriptor) -> String {
    let mut fields = vec![
        format!("Tag={}", render_descriptor_tag(descriptor.tag)),
        format!("Size={}", descriptor.size),
    ];

    match descriptor.tag {
        ES_DESCRIPTOR_TAG => {
            if let Some(nested) = descriptor.es_descriptor.as_ref() {
                fields.push(render_es_descriptor(nested));
            }
        }
        DECODER_CONFIG_DESCRIPTOR_TAG => {
            if let Some(nested) = descriptor.decoder_config_descriptor.as_ref() {
                fields.push(render_decoder_config_descriptor(nested));
            }
        }
        _ => {
            fields.push(format!("Data={}", render_hex_bytes(&descriptor.data)));
        }
    }

    format!("{{{}}}", fields.join(" "))
}

fn render_descriptors(descriptors: &[Descriptor]) -> String {
    render_array(descriptors.iter().map(render_descriptor))
}

/// Elementary-stream descriptor box carried under MPEG-4 audio sample entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Esds {
    full_box: FullBoxState,
    pub descriptors: Vec<Descriptor>,
}

impl FieldHooks for Esds {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Descriptors" => Some(render_descriptors(&self.descriptors)),
            _ => None,
        }
    }
}

impl ImmutableBox for Esds {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"esds")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for Esds {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }
}

impl Esds {
    /// Returns the first descriptor carrying `tag`.
    pub fn first_descriptor_with_tag(&self, tag: u8) -> Option<&Descriptor> {
        self.descriptors
            .iter()
            .find(|descriptor| descriptor.tag == tag)
    }

    /// Returns the first elementary-stream descriptor in the stream.
    pub fn es_descriptor(&self) -> Option<&EsDescriptor> {
        self.first_descriptor_with_tag(ES_DESCRIPTOR_TAG)
            .and_then(|descriptor| descriptor.es_descriptor.as_ref())
    }

    /// Returns the first decoder-configuration descriptor in the stream.
    pub fn decoder_config_descriptor(&self) -> Option<&DecoderConfigDescriptor> {
        self.first_descriptor_with_tag(DECODER_CONFIG_DESCRIPTOR_TAG)
            .and_then(|descriptor| descriptor.decoder_config_descriptor.as_ref())
    }

    /// Returns the first decoder-specific payload carried as raw descriptor data.
    pub fn decoder_specific_info(&self) -> Option<&[u8]> {
        self.first_descriptor_with_tag(DECODER_SPECIFIC_INFO_TAG)
            .map(|descriptor| descriptor.data.as_slice())
    }
}

impl FieldValueRead for Esds {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Descriptors" => Ok(FieldValue::Bytes(encode_descriptor_stream(
                &self.descriptors,
            )?)),
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Esds {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Descriptors", FieldValue::Bytes(bytes)) => {
                self.descriptors = parse_descriptor_stream(field_name, &bytes)?;
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Esds {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("Descriptors", 2, with_bit_width(8), as_bytes()),
    ]);
    const SUPPORTED_VERSIONS: &'static [u8] = &[0];
}

/// One tag-sized record within the `esds` descriptor stream.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Descriptor {
    pub tag: u8,
    pub size: u32,
    pub es_descriptor: Option<EsDescriptor>,
    pub decoder_config_descriptor: Option<DecoderConfigDescriptor>,
    pub data: Vec<u8>,
}

impl Descriptor {
    /// Returns the standard name for the current descriptor tag when one is known.
    pub fn tag_name(&self) -> Option<&'static str> {
        descriptor_tag_name(self.tag)
    }
}

/// Elementary-stream descriptor payload selected by tag `0x03`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EsDescriptor {
    pub es_id: u16,
    pub stream_dependence_flag: bool,
    pub url_flag: bool,
    pub ocr_stream_flag: bool,
    pub stream_priority: u8,
    pub depends_on_es_id: u16,
    pub url_length: u8,
    pub url_string: Vec<u8>,
    pub ocr_es_id: u16,
}

/// Decoder-configuration descriptor payload selected by tag `0x04`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecoderConfigDescriptor {
    pub object_type_indication: u8,
    pub stream_type: u8,
    pub up_stream: bool,
    pub reserved: bool,
    pub buffer_size_db: u32,
    pub max_bitrate: u32,
    pub avg_bitrate: u32,
}

/// Registers the currently implemented ISO/IEC 14496-14 boxes in `registry`.
pub fn register_boxes(registry: &mut BoxRegistry) {
    registry.register::<Esds>(FourCc::from_bytes(*b"esds"));
}
