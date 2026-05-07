use std::io::Read;

use crate::bitio::BitReader;

use super::super::MuxError;
use super::super::import::{SegmentedMuxSourceSpec, StagedSample};

pub(in crate::mux) struct IndexedAnnexBTrack {
    pub(in crate::mux) segmented_source: SegmentedMuxSourceSpec,
    pub(in crate::mux) track_width: u16,
    pub(in crate::mux) track_height: u16,
    pub(in crate::mux) timescale: u32,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) source_edit_media_time: Option<u64>,
    pub(in crate::mux) samples: Vec<StagedSample>,
}

pub(in crate::mux) struct AnnexBNal {
    pub(in crate::mux) source_offset: u64,
    pub(in crate::mux) bytes: Vec<u8>,
}

#[derive(Default)]
pub(in crate::mux) struct AnnexBNalScanner {
    buffer: Vec<u8>,
    buffer_start_offset: u64,
    next_input_offset: u64,
}

impl AnnexBNalScanner {
    pub(in crate::mux) fn push<F>(&mut self, chunk: &[u8], mut on_nal: F) -> Result<(), MuxError>
    where
        F: FnMut(AnnexBNal) -> Result<(), MuxError>,
    {
        for nal in self.collect(chunk) {
            on_nal(nal)?;
        }
        Ok(())
    }

    pub(in crate::mux) fn finish<F>(&mut self, mut on_nal: F) -> Result<(), MuxError>
    where
        F: FnMut(AnnexBNal) -> Result<(), MuxError>,
    {
        for nal in self.finish_collect() {
            on_nal(nal)?;
        }
        Ok(())
    }

    pub(in crate::mux) fn collect(&mut self, chunk: &[u8]) -> Vec<AnnexBNal> {
        if self.buffer.is_empty() {
            self.buffer_start_offset = self.next_input_offset;
        }
        self.buffer.extend_from_slice(chunk);
        self.next_input_offset = self
            .next_input_offset
            .saturating_add(u64::try_from(chunk.len()).unwrap());
        self.drain_available()
    }

    pub(in crate::mux) fn finish_collect(&mut self) -> Vec<AnnexBNal> {
        let mut nals = self.drain_available();
        if let Some((start, start_len)) = find_annex_b_start_code(&self.buffer) {
            let data_start = start + start_len;
            if data_start < self.buffer.len() {
                let mut data_end = self.buffer.len();
                while data_end > data_start && self.buffer[data_end - 1] == 0 {
                    data_end -= 1;
                }
                if data_end > data_start {
                    nals.push(AnnexBNal {
                        source_offset: self.buffer_start_offset
                            + u64::try_from(data_start).unwrap(),
                        bytes: self.buffer[data_start..data_end].to_vec(),
                    });
                }
            }
        }
        self.buffer.clear();
        nals
    }

    fn drain_available(&mut self) -> Vec<AnnexBNal> {
        let mut nals = Vec::new();
        loop {
            let Some((first_start, first_len)) = find_annex_b_start_code(&self.buffer) else {
                if self.buffer.len() > 3 {
                    let retain_from = self.buffer.len() - 3;
                    self.buffer.drain(..retain_from);
                    self.buffer_start_offset += u64::try_from(retain_from).unwrap();
                }
                break;
            };
            if first_start > 0 {
                self.buffer.drain(..first_start);
                self.buffer_start_offset += u64::try_from(first_start).unwrap();
                continue;
            }
            let Some((next_start, _)) = find_annex_b_start_code(&self.buffer[first_len..])
                .map(|(start, len)| (start + first_len, len))
            else {
                break;
            };
            let data_start = first_len;
            let mut data_end = next_start;
            while data_end > data_start && self.buffer[data_end - 1] == 0 {
                data_end -= 1;
            }
            if data_end > data_start {
                nals.push(AnnexBNal {
                    source_offset: self.buffer_start_offset + u64::try_from(data_start).unwrap(),
                    bytes: self.buffer[data_start..data_end].to_vec(),
                });
            }
            self.buffer.drain(..next_start);
            self.buffer_start_offset += u64::try_from(next_start).unwrap();
        }
        nals
    }
}

pub(in crate::mux) fn find_annex_b_start_code(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut index = 0usize;
    while index + 2 < bytes.len() {
        if index + 3 < bytes.len() && bytes[index..].starts_with(&[0, 0, 0, 1]) {
            return Some((index, 4));
        }
        if bytes[index..].starts_with(&[0, 0, 1]) {
            return Some((index, 3));
        }
        index += 1;
    }
    None
}

pub(in crate::mux) fn push_unique_nal(existing: &mut Vec<Vec<u8>>, nal: Vec<u8>) {
    if !existing.iter().any(|entry| entry == &nal) {
        existing.push(nal);
    }
}

pub(in crate::mux) fn nal_to_rbsp(nal: &[u8]) -> Vec<u8> {
    let mut rbsp = Vec::with_capacity(nal.len());
    let mut zero_count = 0_u8;
    for &byte in nal {
        if zero_count == 2 && byte == 0x03 {
            zero_count = 0;
            continue;
        }
        rbsp.push(byte);
        if byte == 0 {
            zero_count = zero_count.saturating_add(1);
        } else {
            zero_count = 0;
        }
    }
    rbsp
}

pub(in crate::mux) fn skip_bits_labeled<R>(
    reader: &mut BitReader<R>,
    width: usize,
    spec: &str,
    label: &str,
) -> Result<(), MuxError>
where
    R: Read,
{
    let _ = reader
        .read_bits(width)
        .map_err(|error| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("failed to read {label} bitstream: {error}"),
        })?;
    Ok(())
}

pub(in crate::mux) fn read_bit_labeled<R>(
    reader: &mut BitReader<R>,
    spec: &str,
    label: &str,
) -> Result<bool, MuxError>
where
    R: Read,
{
    reader
        .read_bit()
        .map_err(|error| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("failed to read {label} bitstream: {error}"),
        })
}

pub(in crate::mux) fn read_bits_u8_labeled<R>(
    reader: &mut BitReader<R>,
    width: usize,
    spec: &str,
    label: &str,
) -> Result<u8, MuxError>
where
    R: Read,
{
    let bits = reader
        .read_bits(width)
        .map_err(|error| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("failed to read {label} bitstream: {error}"),
        })?;
    let mut value = 0_u16;
    for byte in bits {
        value = (value << 8) | u16::from(byte);
    }
    u8::try_from(value).map_err(|_| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: format!("{label} bitfield does not fit in u8"),
    })
}

pub(in crate::mux) fn read_bits_u16_labeled<R>(
    reader: &mut BitReader<R>,
    width: usize,
    spec: &str,
    label: &str,
) -> Result<u16, MuxError>
where
    R: Read,
{
    let bits = reader
        .read_bits(width)
        .map_err(|error| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("failed to read {label} bitstream: {error}"),
        })?;
    let mut value = 0_u32;
    for byte in bits {
        value = (value << 8) | u32::from(byte);
    }
    u16::try_from(value).map_err(|_| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: format!("{label} bitfield does not fit in u16"),
    })
}

pub(in crate::mux) fn read_bits_u32_labeled<R>(
    reader: &mut BitReader<R>,
    width: usize,
    spec: &str,
    label: &str,
) -> Result<u32, MuxError>
where
    R: Read,
{
    let bits = reader
        .read_bits(width)
        .map_err(|error| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("failed to read {label} bitstream: {error}"),
        })?;
    let mut value = 0_u64;
    for byte in bits {
        value = (value << 8) | u64::from(byte);
    }
    u32::try_from(value).map_err(|_| MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: format!("{label} bitfield does not fit in u32"),
    })
}

pub(in crate::mux) fn read_ue_labeled<R>(
    reader: &mut BitReader<R>,
    spec: &str,
    label: &str,
) -> Result<u32, MuxError>
where
    R: Read,
{
    let mut leading_zero_bits = 0_u32;
    while !read_bit_labeled(reader, spec, label)? {
        leading_zero_bits = leading_zero_bits
            .checked_add(1)
            .ok_or(MuxError::LayoutOverflow("Exp-Golomb prefix"))?;
        if leading_zero_bits > 31 {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!("{label} Exp-Golomb prefix is too large"),
            });
        }
    }
    if leading_zero_bits == 0 {
        return Ok(0);
    }
    let suffix = read_bits_u32_labeled(reader, leading_zero_bits as usize, spec, label)?;
    Ok((1_u32 << leading_zero_bits) - 1 + suffix)
}

pub(in crate::mux) fn read_se_labeled<R>(
    reader: &mut BitReader<R>,
    spec: &str,
    label: &str,
) -> Result<i32, MuxError>
where
    R: Read,
{
    let code_num = read_ue_labeled(reader, spec, label)?;
    let magnitude =
        i32::try_from(code_num.div_ceil(2)).map_err(|_| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("{label} signed Exp-Golomb value is too large"),
        })?;
    if code_num % 2 == 0 {
        Ok(-magnitude)
    } else {
        Ok(magnitude)
    }
}
