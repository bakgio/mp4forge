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
/// Descriptor tag used by the IPMP descriptor-pointer record.
pub const IPMP_DESCRIPTOR_POINTER_TAG: u8 = 0x0A;
/// Descriptor tag used by the IPMP descriptor record.
pub const IPMP_DESCRIPTOR_TAG: u8 = 0x0B;
/// Descriptor tag used by the ES-ID-increment descriptor record.
pub const ES_ID_INC_DESCRIPTOR_TAG: u8 = 0x0E;
/// Descriptor tag used by the ES-ID-reference descriptor record.
pub const ES_ID_REF_DESCRIPTOR_TAG: u8 = 0x0F;
/// Descriptor tag used by the MP4 initial-object descriptor record.
pub const MP4_INITIAL_OBJECT_DESCRIPTOR_TAG: u8 = 0x10;
/// Descriptor tag used by the MP4 object-descriptor record.
pub const MP4_OBJECT_DESCRIPTOR_TAG: u8 = 0x11;
/// Command tag used by the object-descriptor-update command record.
pub const OBJECT_DESCRIPTOR_UPDATE_COMMAND_TAG: u8 = 0x01;
/// Command tag used by the IPMP-descriptor-update command record.
pub const IPMP_DESCRIPTOR_UPDATE_COMMAND_TAG: u8 = 0x05;

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
        MP4_OBJECT_DESCRIPTOR_TAG => Some("MP4ObjectDescr"),
        MP4_INITIAL_OBJECT_DESCRIPTOR_TAG => Some("MP4InitialObjectDescr"),
        ES_DESCRIPTOR_TAG => Some("ESDescr"),
        DECODER_CONFIG_DESCRIPTOR_TAG => Some("DecoderConfigDescr"),
        DECODER_SPECIFIC_INFO_TAG => Some("DecSpecificInfo"),
        SL_CONFIG_DESCRIPTOR_TAG => Some("SLConfigDescr"),
        IPMP_DESCRIPTOR_POINTER_TAG => Some("IPMPDescrPointer"),
        IPMP_DESCRIPTOR_TAG => Some("IPMPDescr"),
        ES_ID_INC_DESCRIPTOR_TAG => Some("ES_ID_Inc"),
        ES_ID_REF_DESCRIPTOR_TAG => Some("ES_ID_Ref"),
        _ => None,
    }
}

fn render_descriptor_tag(tag: u8) -> String {
    descriptor_tag_name(tag)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("0x{tag:x}"))
}

fn command_tag_name(tag: u8) -> Option<&'static str> {
    match tag {
        OBJECT_DESCRIPTOR_UPDATE_COMMAND_TAG => Some("ObjectDescriptorUpdate"),
        IPMP_DESCRIPTOR_UPDATE_COMMAND_TAG => Some("IPMPDescriptorUpdate"),
        _ => None,
    }
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

fn encode_object_descriptor(
    field_name: &'static str,
    descriptor: &ObjectDescriptor,
) -> Result<Vec<u8>, FieldValueError> {
    if descriptor.object_descriptor_id > 0x03ff {
        return Err(invalid_value(
            field_name,
            "object descriptor id must fit in 10 bits",
        ));
    }
    if descriptor.url_flag && usize::from(descriptor.url_length) != descriptor.url_string.len() {
        return Err(invalid_value(
            "URLString",
            "value length does not match URLLength",
        ));
    }

    let mut buffer = Vec::new();
    write_u16(
        &mut buffer,
        (descriptor.object_descriptor_id << 6) | (u16::from(descriptor.url_flag) << 5) | 0x001f,
    );
    if descriptor.url_flag {
        buffer.push(descriptor.url_length);
        buffer.extend_from_slice(&descriptor.url_string);
    }
    buffer.extend_from_slice(&encode_descriptor_stream(&descriptor.sub_descriptors)?);
    Ok(buffer)
}

fn encode_initial_object_descriptor(
    field_name: &'static str,
    descriptor: &InitialObjectDescriptor,
) -> Result<Vec<u8>, FieldValueError> {
    if descriptor.object_descriptor_id > 0x03ff {
        return Err(invalid_value(
            field_name,
            "object descriptor id must fit in 10 bits",
        ));
    }
    if descriptor.url_flag && usize::from(descriptor.url_length) != descriptor.url_string.len() {
        return Err(invalid_value(
            "URLString",
            "value length does not match URLLength",
        ));
    }

    let mut buffer = Vec::new();
    write_u16(
        &mut buffer,
        (descriptor.object_descriptor_id << 6)
            | (u16::from(descriptor.url_flag) << 5)
            | (u16::from(descriptor.include_inline_profile_level_flag) << 4)
            | 0x000f,
    );
    if descriptor.url_flag {
        buffer.push(descriptor.url_length);
        buffer.extend_from_slice(&descriptor.url_string);
    } else {
        buffer.extend_from_slice(&[
            descriptor.od_profile_level_indication,
            descriptor.scene_profile_level_indication,
            descriptor.audio_profile_level_indication,
            descriptor.visual_profile_level_indication,
            descriptor.graphics_profile_level_indication,
        ]);
    }
    buffer.extend_from_slice(&encode_descriptor_stream(&descriptor.sub_descriptors)?);
    Ok(buffer)
}

fn encode_ipmp_descriptor_pointer(descriptor: &IpmpDescriptorPointer) -> Vec<u8> {
    let mut buffer = Vec::new();
    buffer.push(descriptor.descriptor_id);
    if descriptor.descriptor_id == 0xff {
        write_u16(&mut buffer, descriptor.descriptor_id_ex);
        write_u16(&mut buffer, descriptor.es_id);
    }
    buffer
}

fn encode_ipmp_descriptor(descriptor: &IpmpDescriptor) -> Vec<u8> {
    let mut buffer = Vec::new();
    buffer.push(descriptor.descriptor_id);
    write_u16(&mut buffer, descriptor.ipmps_type);
    if descriptor.descriptor_id == 0xff && descriptor.ipmps_type == 0xffff {
        write_u16(&mut buffer, descriptor.descriptor_id_ex);
        buffer.extend_from_slice(&descriptor.tool_id);
        buffer.push(descriptor.control_point_code);
        if descriptor.control_point_code > 0 {
            buffer.push(descriptor.sequence_code);
        }
        buffer.extend_from_slice(&descriptor.data);
    } else if descriptor.ipmps_type == 0 {
        buffer.extend_from_slice(&descriptor.url_string);
        buffer.push(0);
    } else {
        buffer.extend_from_slice(&descriptor.data);
    }
    buffer
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

fn encode_command_payload(command: &DescriptorCommand) -> Result<(u8, Vec<u8>), FieldValueError> {
    match command {
        DescriptorCommand::DescriptorUpdate(command) => {
            Ok((command.tag, encode_descriptor_stream(&command.descriptors)?))
        }
        DescriptorCommand::Unknown(command) => Ok((command.tag, command.data.clone())),
    }
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

fn parse_object_descriptor_payload(
    field_name: &'static str,
    payload: &[u8],
) -> Result<ObjectDescriptor, FieldValueError> {
    let mut reader = Cursor::new(payload);
    let bits = read_u16(&mut reader, field_name)?;
    let object_descriptor_id = bits >> 6;
    let url_flag = bits & (1 << 5) != 0;
    let (url_length, url_string) = if url_flag {
        let url_length = read_u8(&mut reader, field_name)?;
        let url_string = read_exact_bytes(&mut reader, usize::from(url_length), field_name)?;
        (url_length, url_string)
    } else {
        (0, Vec::new())
    };
    let sub_descriptors =
        parse_descriptor_stream(field_name, &payload[reader.position() as usize..])?;

    Ok(ObjectDescriptor {
        object_descriptor_id,
        url_flag,
        url_length,
        url_string,
        sub_descriptors,
    })
}

fn parse_initial_object_descriptor_payload(
    field_name: &'static str,
    payload: &[u8],
) -> Result<InitialObjectDescriptor, FieldValueError> {
    let mut reader = Cursor::new(payload);
    let bits = read_u16(&mut reader, field_name)?;
    let object_descriptor_id = bits >> 6;
    let url_flag = bits & (1 << 5) != 0;
    let include_inline_profile_level_flag = bits & (1 << 4) != 0;
    let (url_length, url_string) = if url_flag {
        let url_length = read_u8(&mut reader, field_name)?;
        let url_string = read_exact_bytes(&mut reader, usize::from(url_length), field_name)?;
        (url_length, url_string)
    } else {
        (0, Vec::new())
    };
    let (
        od_profile_level_indication,
        scene_profile_level_indication,
        audio_profile_level_indication,
        visual_profile_level_indication,
        graphics_profile_level_indication,
    ) = if url_flag {
        (0, 0, 0, 0, 0)
    } else {
        (
            read_u8(&mut reader, field_name)?,
            read_u8(&mut reader, field_name)?,
            read_u8(&mut reader, field_name)?,
            read_u8(&mut reader, field_name)?,
            read_u8(&mut reader, field_name)?,
        )
    };
    let sub_descriptors =
        parse_descriptor_stream(field_name, &payload[reader.position() as usize..])?;

    Ok(InitialObjectDescriptor {
        object_descriptor_id,
        url_flag,
        include_inline_profile_level_flag,
        url_length,
        url_string,
        od_profile_level_indication,
        scene_profile_level_indication,
        audio_profile_level_indication,
        visual_profile_level_indication,
        graphics_profile_level_indication,
        sub_descriptors,
    })
}

fn parse_ipmp_descriptor_pointer_payload(
    field_name: &'static str,
    payload: &[u8],
) -> Result<IpmpDescriptorPointer, FieldValueError> {
    let mut reader = Cursor::new(payload);
    let descriptor_id = read_u8(&mut reader, field_name)?;
    let (descriptor_id_ex, es_id) = if descriptor_id == 0xff {
        (
            read_u16(&mut reader, field_name)?,
            read_u16(&mut reader, field_name)?,
        )
    } else {
        (0, 0)
    };
    Ok(IpmpDescriptorPointer {
        descriptor_id,
        descriptor_id_ex,
        es_id,
    })
}

fn parse_ipmp_descriptor_payload(
    field_name: &'static str,
    payload: &[u8],
) -> Result<IpmpDescriptor, FieldValueError> {
    let mut reader = Cursor::new(payload);
    let descriptor_id = read_u8(&mut reader, field_name)?;
    let ipmps_type = read_u16(&mut reader, field_name)?;
    let mut tool_id = [0_u8; 16];
    let (descriptor_id_ex, control_point_code, sequence_code, url_string, data) =
        if descriptor_id == 0xff && ipmps_type == 0xffff {
            let descriptor_id_ex = read_u16(&mut reader, field_name)?;
            let tool_id_bytes = read_exact_bytes(&mut reader, 16, field_name)?;
            tool_id.copy_from_slice(&tool_id_bytes);
            let control_point_code = read_u8(&mut reader, field_name)?;
            let sequence_code = if control_point_code > 0 {
                read_u8(&mut reader, field_name)?
            } else {
                0
            };
            (
                descriptor_id_ex,
                control_point_code,
                sequence_code,
                Vec::new(),
                payload[reader.position() as usize..].to_vec(),
            )
        } else if ipmps_type == 0 {
            let mut url_string = payload[reader.position() as usize..].to_vec();
            if url_string.last().copied() == Some(0) {
                url_string.pop();
            }
            (0, 0, 0, url_string, Vec::new())
        } else {
            (
                0,
                0,
                0,
                Vec::new(),
                payload[reader.position() as usize..].to_vec(),
            )
        };

    Ok(IpmpDescriptor {
        descriptor_id,
        ipmps_type,
        descriptor_id_ex,
        tool_id,
        control_point_code,
        sequence_code,
        url_string,
        data,
    })
}

fn parse_es_id_inc_descriptor_payload(
    field_name: &'static str,
    payload: &[u8],
) -> Result<EsIdIncDescriptor, FieldValueError> {
    let mut reader = Cursor::new(payload);
    Ok(EsIdIncDescriptor {
        track_id: read_u32(&mut reader, field_name)?,
    })
}

fn parse_es_id_ref_descriptor_payload(
    field_name: &'static str,
    payload: &[u8],
) -> Result<EsIdRefDescriptor, FieldValueError> {
    let mut reader = Cursor::new(payload);
    Ok(EsIdRefDescriptor {
        ref_index: read_u16(&mut reader, field_name)?,
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
            MP4_OBJECT_DESCRIPTOR_TAG => {
                descriptor.data = read_exact_bytes(&mut reader, size as usize, field_name)?;
                parse_object_descriptor_payload("ObjectDescriptor", &descriptor.data)?;
            }
            MP4_INITIAL_OBJECT_DESCRIPTOR_TAG => {
                descriptor.data = read_exact_bytes(&mut reader, size as usize, field_name)?;
                parse_initial_object_descriptor_payload(
                    "InitialObjectDescriptor",
                    &descriptor.data,
                )?;
            }
            ES_DESCRIPTOR_TAG => {
                descriptor.es_descriptor = Some(parse_es_descriptor("ESDescriptor", &mut reader)?);
            }
            DECODER_CONFIG_DESCRIPTOR_TAG => {
                descriptor.decoder_config_descriptor = Some(parse_decoder_config_descriptor(
                    "DecoderConfigDescriptor",
                    &mut reader,
                )?);
            }
            ES_ID_INC_DESCRIPTOR_TAG => {
                descriptor.data = read_exact_bytes(&mut reader, size as usize, field_name)?;
                parse_es_id_inc_descriptor_payload("EsIdIncDescriptor", &descriptor.data)?;
            }
            ES_ID_REF_DESCRIPTOR_TAG => {
                descriptor.data = read_exact_bytes(&mut reader, size as usize, field_name)?;
                parse_es_id_ref_descriptor_payload("EsIdRefDescriptor", &descriptor.data)?;
            }
            IPMP_DESCRIPTOR_POINTER_TAG => {
                descriptor.data = read_exact_bytes(&mut reader, size as usize, field_name)?;
                parse_ipmp_descriptor_pointer_payload("IpmpDescriptorPointer", &descriptor.data)?;
            }
            IPMP_DESCRIPTOR_TAG => {
                descriptor.data = read_exact_bytes(&mut reader, size as usize, field_name)?;
                parse_ipmp_descriptor_payload("IpmpDescriptor", &descriptor.data)?;
            }
            _ => {
                descriptor.data = read_exact_bytes(&mut reader, size as usize, field_name)?;
            }
        }

        descriptors.push(descriptor);
    }

    Ok(descriptors)
}

/// Decodes one OD-stream command payload into additive typed command records.
///
/// The helper currently recognizes the object-descriptor-update and IPMP-descriptor-update
/// command families and preserves any other command tags as raw payload bytes.
pub fn parse_descriptor_commands(bytes: &[u8]) -> Result<Vec<DescriptorCommand>, FieldValueError> {
    let mut reader = Cursor::new(bytes);
    let mut commands = Vec::new();
    while (reader.position() as usize) < bytes.len() {
        let tag = read_u8(&mut reader, "CommandTag")?;
        let size = read_uvarint(&mut reader, "CommandSize")?;
        let data = read_exact_bytes(&mut reader, size as usize, "CommandData")?;
        match tag {
            OBJECT_DESCRIPTOR_UPDATE_COMMAND_TAG | IPMP_DESCRIPTOR_UPDATE_COMMAND_TAG => {
                commands.push(DescriptorCommand::DescriptorUpdate(
                    DescriptorUpdateCommand {
                        tag,
                        descriptors: parse_descriptor_stream("Descriptors", &data)?,
                    },
                ));
            }
            _ => commands.push(DescriptorCommand::Unknown(UnknownDescriptorCommand {
                tag,
                data,
            })),
        }
    }

    Ok(commands)
}

/// Encodes additive OD-stream command records into one contiguous command payload.
pub fn encode_descriptor_commands(
    commands: &[DescriptorCommand],
) -> Result<Vec<u8>, FieldValueError> {
    let mut buffer = Vec::new();
    for command in commands {
        let (tag, data) = encode_command_payload(command)?;
        buffer.push(tag);
        write_uvarint(&mut buffer, "CommandSize", data.len() as u32)?;
        buffer.extend_from_slice(&data);
    }
    Ok(buffer)
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

fn render_object_descriptor(descriptor: &ObjectDescriptor) -> String {
    let mut fields = vec![
        format!("ObjectDescriptorID={}", descriptor.object_descriptor_id),
        format!("UrlFlag={}", descriptor.url_flag),
    ];
    if descriptor.url_flag {
        fields.push(format!("URLLength=0x{:x}", descriptor.url_length));
        fields.push(format!("URLString={}", quote_bytes(&descriptor.url_string)));
    }
    if !descriptor.sub_descriptors.is_empty() {
        fields.push(format!(
            "SubDescriptors={}",
            render_descriptors(&descriptor.sub_descriptors)
        ));
    }
    fields.join(" ")
}

fn render_initial_object_descriptor(descriptor: &InitialObjectDescriptor) -> String {
    let mut fields = vec![
        format!("ObjectDescriptorID={}", descriptor.object_descriptor_id),
        format!("UrlFlag={}", descriptor.url_flag),
        format!(
            "IncludeInlineProfileLevelFlag={}",
            descriptor.include_inline_profile_level_flag
        ),
    ];
    if descriptor.url_flag {
        fields.push(format!("URLLength=0x{:x}", descriptor.url_length));
        fields.push(format!("URLString={}", quote_bytes(&descriptor.url_string)));
    } else {
        fields.extend([
            format!(
                "ODProfileLevelIndication=0x{:x}",
                descriptor.od_profile_level_indication
            ),
            format!(
                "SceneProfileLevelIndication=0x{:x}",
                descriptor.scene_profile_level_indication
            ),
            format!(
                "AudioProfileLevelIndication=0x{:x}",
                descriptor.audio_profile_level_indication
            ),
            format!(
                "VisualProfileLevelIndication=0x{:x}",
                descriptor.visual_profile_level_indication
            ),
            format!(
                "GraphicsProfileLevelIndication=0x{:x}",
                descriptor.graphics_profile_level_indication
            ),
        ]);
    }
    if !descriptor.sub_descriptors.is_empty() {
        fields.push(format!(
            "SubDescriptors={}",
            render_descriptors(&descriptor.sub_descriptors)
        ));
    }
    fields.join(" ")
}

fn render_ipmp_descriptor_pointer(descriptor: &IpmpDescriptorPointer) -> String {
    let mut fields = vec![format!("DescriptorID=0x{:x}", descriptor.descriptor_id)];
    if descriptor.descriptor_id == 0xff {
        fields.push(format!(
            "DescriptorIDEx=0x{:x}",
            descriptor.descriptor_id_ex
        ));
        fields.push(format!("ESID={}", descriptor.es_id));
    }
    fields.join(" ")
}

fn render_ipmp_descriptor(descriptor: &IpmpDescriptor) -> String {
    let mut fields = vec![
        format!("DescriptorID=0x{:x}", descriptor.descriptor_id),
        format!("IPMPSType=0x{:x}", descriptor.ipmps_type),
    ];
    if descriptor.descriptor_id == 0xff && descriptor.ipmps_type == 0xffff {
        fields.push(format!(
            "DescriptorIDEx=0x{:x}",
            descriptor.descriptor_id_ex
        ));
        fields.push(format!("ToolID={}", render_hex_bytes(&descriptor.tool_id)));
        fields.push(format!(
            "ControlPointCode={}",
            descriptor.control_point_code
        ));
        if descriptor.control_point_code > 0 {
            fields.push(format!("SequenceCode={}", descriptor.sequence_code));
        }
        if !descriptor.data.is_empty() {
            fields.push(format!("Data={}", render_hex_bytes(&descriptor.data)));
        }
    } else if descriptor.ipmps_type == 0 {
        fields.push(format!("URLString={}", quote_bytes(&descriptor.url_string)));
    } else if !descriptor.data.is_empty() {
        fields.push(format!("Data={}", render_hex_bytes(&descriptor.data)));
    }
    fields.join(" ")
}

fn render_descriptor(descriptor: &Descriptor) -> String {
    let mut fields = vec![
        format!("Tag={}", render_descriptor_tag(descriptor.tag)),
        format!("Size={}", descriptor.size),
    ];

    match descriptor.tag {
        MP4_OBJECT_DESCRIPTOR_TAG => {
            if let Some(nested) = descriptor.object_descriptor() {
                fields.push(render_object_descriptor(&nested));
            }
        }
        MP4_INITIAL_OBJECT_DESCRIPTOR_TAG => {
            if let Some(nested) = descriptor.initial_object_descriptor() {
                fields.push(render_initial_object_descriptor(&nested));
            }
        }
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
        ES_ID_INC_DESCRIPTOR_TAG => {
            if let Some(nested) = descriptor.es_id_inc_descriptor() {
                fields.push(format!("TrackID={}", nested.track_id));
            }
        }
        ES_ID_REF_DESCRIPTOR_TAG => {
            if let Some(nested) = descriptor.es_id_ref_descriptor() {
                fields.push(format!("RefIndex={}", nested.ref_index));
            }
        }
        IPMP_DESCRIPTOR_POINTER_TAG => {
            if let Some(nested) = descriptor.ipmp_descriptor_pointer() {
                fields.push(render_ipmp_descriptor_pointer(&nested));
            }
        }
        IPMP_DESCRIPTOR_TAG => {
            if let Some(nested) = descriptor.ipmp_descriptor() {
                fields.push(render_ipmp_descriptor(&nested));
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

/// Initial-object descriptor box carried under `moov`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Iods {
    full_box: FullBoxState,
    pub descriptor: Option<Descriptor>,
}

impl FieldHooks for Iods {
    fn display_field(&self, name: &'static str) -> Option<String> {
        match name {
            "Descriptor" => self.descriptor.as_ref().map(render_descriptor),
            _ => None,
        }
    }
}

impl ImmutableBox for Iods {
    fn box_type(&self) -> FourCc {
        FourCc::from_bytes(*b"iods")
    }

    fn version(&self) -> u8 {
        self.full_box.version
    }

    fn flags(&self) -> u32 {
        self.full_box.flags
    }
}

impl MutableBox for Iods {
    fn set_version(&mut self, version: u8) {
        self.full_box.version = version;
    }

    fn set_flags(&mut self, flags: u32) {
        self.full_box.flags = flags;
    }
}

impl Iods {
    /// Returns the typed descriptor carried by the box when present.
    pub fn descriptor(&self) -> Option<&Descriptor> {
        self.descriptor.as_ref()
    }

    /// Returns the initial-object descriptor payload when the carried descriptor uses that tag.
    pub fn initial_object_descriptor(&self) -> Option<InitialObjectDescriptor> {
        self.descriptor
            .as_ref()
            .and_then(Descriptor::initial_object_descriptor)
    }
}

impl FieldValueRead for Iods {
    fn field_value(&self, field_name: &'static str) -> Result<FieldValue, FieldValueError> {
        match field_name {
            "Descriptor" => {
                let descriptors = self.descriptor.iter().cloned().collect::<Vec<_>>();
                Ok(FieldValue::Bytes(encode_descriptor_stream(&descriptors)?))
            }
            _ => Err(missing_field(field_name)),
        }
    }
}

impl FieldValueWrite for Iods {
    fn set_field_value(
        &mut self,
        field_name: &'static str,
        value: FieldValue,
    ) -> Result<(), FieldValueError> {
        match (field_name, value) {
            ("Descriptor", FieldValue::Bytes(bytes)) => {
                let descriptors = parse_descriptor_stream(field_name, &bytes)?;
                self.descriptor = match descriptors.len() {
                    0 => None,
                    1 => Some(descriptors.into_iter().next().unwrap()),
                    _ => {
                        return Err(invalid_value(
                            field_name,
                            "iods may carry at most one descriptor",
                        ));
                    }
                };
                Ok(())
            }
            (field_name, value) => Err(unexpected_field(field_name, value)),
        }
    }
}

impl CodecBox for Iods {
    const FIELD_TABLE: FieldTable = FieldTable::new(&[
        codec_field!("Version", 0, with_bit_width(8), as_version_field()),
        codec_field!("Flags", 1, with_bit_width(24), as_flags_field()),
        codec_field!("Descriptor", 2, with_bit_width(8), as_bytes()),
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

    /// Builds a typed MP4 object-descriptor record.
    pub fn from_object_descriptor(descriptor: ObjectDescriptor) -> Result<Self, FieldValueError> {
        let data = encode_object_descriptor("ObjectDescriptor", &descriptor)?;
        Ok(Self {
            tag: MP4_OBJECT_DESCRIPTOR_TAG,
            size: data.len() as u32,
            data,
            ..Self::default()
        })
    }

    /// Builds a typed MP4 initial-object-descriptor record.
    pub fn from_initial_object_descriptor(
        descriptor: InitialObjectDescriptor,
    ) -> Result<Self, FieldValueError> {
        let data = encode_initial_object_descriptor("InitialObjectDescriptor", &descriptor)?;
        Ok(Self {
            tag: MP4_INITIAL_OBJECT_DESCRIPTOR_TAG,
            size: data.len() as u32,
            data,
            ..Self::default()
        })
    }

    /// Builds a typed ES-ID-increment descriptor record.
    pub fn from_es_id_inc_descriptor(descriptor: EsIdIncDescriptor) -> Self {
        let mut data = Vec::new();
        write_u32(&mut data, descriptor.track_id);
        Self {
            tag: ES_ID_INC_DESCRIPTOR_TAG,
            size: data.len() as u32,
            data,
            ..Self::default()
        }
    }

    /// Builds a typed ES-ID-reference descriptor record.
    pub fn from_es_id_ref_descriptor(descriptor: EsIdRefDescriptor) -> Self {
        let mut data = Vec::new();
        write_u16(&mut data, descriptor.ref_index);
        Self {
            tag: ES_ID_REF_DESCRIPTOR_TAG,
            size: data.len() as u32,
            data,
            ..Self::default()
        }
    }

    /// Builds a typed IPMP descriptor-pointer record.
    pub fn from_ipmp_descriptor_pointer(descriptor: IpmpDescriptorPointer) -> Self {
        let data = encode_ipmp_descriptor_pointer(&descriptor);
        Self {
            tag: IPMP_DESCRIPTOR_POINTER_TAG,
            size: data.len() as u32,
            data,
            ..Self::default()
        }
    }

    /// Builds a typed IPMP descriptor record.
    pub fn from_ipmp_descriptor(descriptor: IpmpDescriptor) -> Self {
        let data = encode_ipmp_descriptor(&descriptor);
        Self {
            tag: IPMP_DESCRIPTOR_TAG,
            size: data.len() as u32,
            data,
            ..Self::default()
        }
    }

    /// Returns the typed MP4 object-descriptor payload when the tag matches.
    pub fn object_descriptor(&self) -> Option<ObjectDescriptor> {
        (self.tag == MP4_OBJECT_DESCRIPTOR_TAG)
            .then(|| parse_object_descriptor_payload("ObjectDescriptor", &self.data))
            .and_then(Result::ok)
    }

    /// Returns the typed MP4 initial-object-descriptor payload when the tag matches.
    pub fn initial_object_descriptor(&self) -> Option<InitialObjectDescriptor> {
        (self.tag == MP4_INITIAL_OBJECT_DESCRIPTOR_TAG)
            .then(|| parse_initial_object_descriptor_payload("InitialObjectDescriptor", &self.data))
            .and_then(Result::ok)
    }

    /// Returns the typed ES-ID-increment payload when the tag matches.
    pub fn es_id_inc_descriptor(&self) -> Option<EsIdIncDescriptor> {
        (self.tag == ES_ID_INC_DESCRIPTOR_TAG)
            .then(|| parse_es_id_inc_descriptor_payload("EsIdIncDescriptor", &self.data))
            .and_then(Result::ok)
    }

    /// Returns the typed ES-ID-reference payload when the tag matches.
    pub fn es_id_ref_descriptor(&self) -> Option<EsIdRefDescriptor> {
        (self.tag == ES_ID_REF_DESCRIPTOR_TAG)
            .then(|| parse_es_id_ref_descriptor_payload("EsIdRefDescriptor", &self.data))
            .and_then(Result::ok)
    }

    /// Returns the typed IPMP descriptor-pointer payload when the tag matches.
    pub fn ipmp_descriptor_pointer(&self) -> Option<IpmpDescriptorPointer> {
        (self.tag == IPMP_DESCRIPTOR_POINTER_TAG)
            .then(|| parse_ipmp_descriptor_pointer_payload("IpmpDescriptorPointer", &self.data))
            .and_then(Result::ok)
    }

    /// Returns the typed IPMP descriptor payload when the tag matches.
    pub fn ipmp_descriptor(&self) -> Option<IpmpDescriptor> {
        (self.tag == IPMP_DESCRIPTOR_TAG)
            .then(|| parse_ipmp_descriptor_payload("IpmpDescriptor", &self.data))
            .and_then(Result::ok)
    }
}

/// One parsed OD-stream command record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DescriptorCommand {
    /// A typed object-descriptor-update or IPMP-descriptor-update command.
    DescriptorUpdate(DescriptorUpdateCommand),
    /// A command tag that the current crate does not model yet.
    Unknown(UnknownDescriptorCommand),
}

impl DescriptorCommand {
    /// Returns the raw command tag.
    pub fn tag(&self) -> u8 {
        match self {
            Self::DescriptorUpdate(command) => command.tag,
            Self::Unknown(command) => command.tag,
        }
    }

    /// Returns the standard command name for the current tag when one is known.
    pub fn tag_name(&self) -> Option<&'static str> {
        command_tag_name(self.tag())
    }

    /// Returns the typed descriptor-update payload when this command carries one.
    pub fn descriptor_update(&self) -> Option<&DescriptorUpdateCommand> {
        match self {
            Self::DescriptorUpdate(command) => Some(command),
            Self::Unknown(_) => None,
        }
    }

    /// Returns the raw unknown-command payload when this command is not yet modeled.
    pub fn unknown(&self) -> Option<&UnknownDescriptorCommand> {
        match self {
            Self::DescriptorUpdate(_) => None,
            Self::Unknown(command) => Some(command),
        }
    }
}

/// A typed object-descriptor-update or IPMP-descriptor-update command payload.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DescriptorUpdateCommand {
    pub tag: u8,
    pub descriptors: Vec<Descriptor>,
}

impl DescriptorUpdateCommand {
    /// Builds an object-descriptor-update command.
    pub fn object_descriptor_update(descriptors: Vec<Descriptor>) -> Self {
        Self {
            tag: OBJECT_DESCRIPTOR_UPDATE_COMMAND_TAG,
            descriptors,
        }
    }

    /// Builds an IPMP-descriptor-update command.
    pub fn ipmp_descriptor_update(descriptors: Vec<Descriptor>) -> Self {
        Self {
            tag: IPMP_DESCRIPTOR_UPDATE_COMMAND_TAG,
            descriptors,
        }
    }

    /// Returns the standard command name for the current tag when one is known.
    pub fn tag_name(&self) -> Option<&'static str> {
        command_tag_name(self.tag)
    }
}

/// One raw OD-stream command tag that the current crate does not model yet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnknownDescriptorCommand {
    pub tag: u8,
    pub data: Vec<u8>,
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

/// MP4 object descriptor payload selected by tag `0x11`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ObjectDescriptor {
    pub object_descriptor_id: u16,
    pub url_flag: bool,
    pub url_length: u8,
    pub url_string: Vec<u8>,
    pub sub_descriptors: Vec<Descriptor>,
}

/// MP4 initial-object descriptor payload selected by tag `0x10`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InitialObjectDescriptor {
    pub object_descriptor_id: u16,
    pub url_flag: bool,
    pub include_inline_profile_level_flag: bool,
    pub url_length: u8,
    pub url_string: Vec<u8>,
    pub od_profile_level_indication: u8,
    pub scene_profile_level_indication: u8,
    pub audio_profile_level_indication: u8,
    pub visual_profile_level_indication: u8,
    pub graphics_profile_level_indication: u8,
    pub sub_descriptors: Vec<Descriptor>,
}

/// ES-ID-increment descriptor payload selected by tag `0x0e`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EsIdIncDescriptor {
    pub track_id: u32,
}

/// ES-ID-reference descriptor payload selected by tag `0x0f`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EsIdRefDescriptor {
    pub ref_index: u16,
}

/// IPMP descriptor-pointer payload selected by tag `0x0a`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IpmpDescriptorPointer {
    pub descriptor_id: u8,
    pub descriptor_id_ex: u16,
    pub es_id: u16,
}

/// IPMP descriptor payload selected by tag `0x0b`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IpmpDescriptor {
    pub descriptor_id: u8,
    pub ipmps_type: u16,
    pub descriptor_id_ex: u16,
    pub tool_id: [u8; 16],
    pub control_point_code: u8,
    pub sequence_code: u8,
    pub url_string: Vec<u8>,
    pub data: Vec<u8>,
}

/// Registers the currently implemented ISO/IEC 14496-14 boxes in `registry`.
pub fn register_boxes(registry: &mut BoxRegistry) {
    registry.register::<Esds>(FourCc::from_bytes(*b"esds"));
    registry.register::<Iods>(FourCc::from_bytes(*b"iods"));
}
