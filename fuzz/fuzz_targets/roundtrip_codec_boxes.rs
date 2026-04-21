#![no_main]

mod support;

use std::fmt::Debug;
use std::io::Cursor;

use libfuzzer_sys::fuzz_target;
use mp4forge::boxes::iso14496_12::Ftyp;
use mp4forge::boxes::iso23001_7::{Pssh, PsshKid};
use mp4forge::boxes::metadata::{
    DATA_TYPE_BINARY, DATA_TYPE_SIGNED_INT_BIG_ENDIAN, DATA_TYPE_STRING_JPEG,
    DATA_TYPE_STRING_UTF8, Data,
};
use mp4forge::codec::{CodecBox, MutableBox, marshal, unmarshal};

use support::FuzzInput;

const DATA_TYPES: [u32; 4] = [
    DATA_TYPE_BINARY,
    DATA_TYPE_STRING_UTF8,
    DATA_TYPE_STRING_JPEG,
    DATA_TYPE_SIGNED_INT_BIG_ENDIAN,
];

fuzz_target!(|data: &[u8]| {
    let mut input = FuzzInput::new(data);
    match input.take_u8() % 3 {
        0 => roundtrip(build_ftyp(&mut input)),
        1 => roundtrip(build_data(&mut input)),
        _ => roundtrip(build_pssh(&mut input)),
    }
});

fn build_ftyp(input: &mut FuzzInput<'_>) -> Ftyp {
    let mut ftyp = Ftyp {
        major_brand: input.take_fourcc(),
        minor_version: input.take_u32(),
        compatible_brands: Vec::new(),
    };

    for _ in 0..input.take_usize(8) {
        ftyp.add_compatible_brand(input.take_fourcc());
    }

    ftyp
}

fn build_data(input: &mut FuzzInput<'_>) -> Data {
    Data {
        data_type: DATA_TYPES[input.take_usize(DATA_TYPES.len() - 1)],
        data_lang: input.take_u32(),
        data: input.take_bytes(64),
    }
}

fn build_pssh(input: &mut FuzzInput<'_>) -> Pssh {
    let version = input.take_u8() & 1;
    let mut pssh = Pssh::default();
    pssh.set_version(version);
    pssh.set_flags(input.take_u32() & 0x00ff_ffff);
    pssh.system_id = input.take_exact();
    pssh.data = input.take_bytes(64);

    if version == 1 {
        for _ in 0..input.take_usize(4) {
            pssh.kids.push(PsshKid {
                kid: input.take_exact(),
            });
        }
    }

    pssh.kid_count = pssh.kids.len() as u32;
    pssh.data_size = pssh.data.len() as u32;
    pssh
}

fn roundtrip<B>(src: B)
where
    B: CodecBox + Default + PartialEq + Debug,
{
    let mut encoded = Vec::new();
    if let Ok(written) = marshal(&mut encoded, &src, None) {
        let mut decoded = B::default();
        let mut cursor = Cursor::new(encoded);
        if let Ok(read) = unmarshal(&mut cursor, written, &mut decoded, None) {
            assert_eq!(read, written);
            assert_eq!(decoded, src);
        }
    }
}
