use std::io::{Cursor, Read, Seek, SeekFrom, Write};

use mp4forge::bitio::{BitReader, BitWriter, INVALID_ALIGNMENT_MESSAGE};

#[test]
fn read_and_write_match_bit_packing_examples() {
    let mut writer = BitWriter::new(Vec::new());
    writer.write_bits(&[0xda], 7).unwrap();
    writer.write_bits(&[0x07, 0x63, 0xd5], 17).unwrap();
    writer.write_all(&[0xa4, 0x6f]).unwrap();
    writer.write_bits(&[0x07, 0x69, 0xe3], 17).unwrap();
    writer.write_bit(true).unwrap();
    writer.write_bit(false).unwrap();
    writer.write_bits(&[0xf7], 5).unwrap();

    let encoded = writer.into_inner().unwrap();
    assert_eq!(encoded, [0xb5, 0x63, 0xd5, 0xa4, 0x6f, 0xb4, 0xf1, 0xd7]);
    let mut cursor = Cursor::new(encoded);

    let mut reader = BitReader::new(&mut cursor);
    assert_eq!(reader.read_bits(7).unwrap(), vec![0x5a]);
    assert_eq!(reader.read_bits(17).unwrap(), vec![0x01, 0x63, 0xd5]);

    let mut aligned = [0_u8; 2];
    reader.read_exact(&mut aligned).unwrap();
    assert_eq!(aligned, [0xa4, 0x6f]);

    assert_eq!(reader.read_bits(17).unwrap(), vec![0x01, 0x69, 0xe3]);
    assert!(reader.read_bit().unwrap());
    assert!(!reader.read_bit().unwrap());
    assert_eq!(reader.read_bits(5).unwrap(), vec![0x17]);
}

#[test]
fn byte_reads_fail_when_reader_is_not_aligned() {
    let mut reader = BitReader::new(Cursor::new(vec![0x6c, 0x82, 0x41, 0x35]));

    let mut buf = [0_u8; 2];
    reader.read_exact(&mut buf).unwrap();
    assert_eq!(buf, [0x6c, 0x82]);

    assert_eq!(reader.read_bits(3).unwrap(), vec![0x02]);

    let error = reader.read(&mut buf).unwrap_err();
    assert_eq!(error.to_string(), INVALID_ALIGNMENT_MESSAGE);
}

#[test]
fn seek_current_requires_byte_alignment() {
    let mut reader = BitReader::new(Cursor::new(vec![0x6c, 0x82, 0x41, 0x35, 0x71]));

    assert_eq!(reader.seek(SeekFrom::Current(2)).unwrap(), 2);
    assert_eq!(reader.read_bits(3).unwrap(), vec![0x02]);

    let error = reader.seek(SeekFrom::Current(2)).unwrap_err();
    assert_eq!(error.to_string(), INVALID_ALIGNMENT_MESSAGE);

    assert_eq!(reader.seek(SeekFrom::Start(0)).unwrap(), 0);
    assert_eq!(reader.read_bits(3).unwrap(), vec![0x03]);
}

#[test]
fn byte_writes_fail_when_writer_is_not_aligned() {
    let mut writer = BitWriter::new(Vec::new());
    writer.write_all(&[0xa4, 0x6f]).unwrap();
    writer.write_bits(&[0xda], 7).unwrap();

    let error = writer.write(&[0xa4, 0x6f]).unwrap_err();
    assert_eq!(error.to_string(), INVALID_ALIGNMENT_MESSAGE);
}
