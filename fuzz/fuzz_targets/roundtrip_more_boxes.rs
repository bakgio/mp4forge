#![no_main]

mod support;

use std::io::Cursor;

use libfuzzer_sys::fuzz_target;
use mp4forge::boxes::etsi_ts_102_366::{Dac3, Dec3};
use mp4forge::boxes::etsi_ts_103_190::Dac4;
use mp4forge::boxes::iso14496_12::{
    AVCDecoderConfiguration, Btrt, Clap, CoLL, Colr, Frma, HEVCDecoderConfiguration, Hdlr, Mdhd,
    Mvhd, Pasp, Saio, Saiz, Sbgp, Schm, Sgpd, Sidx, Tfdt, Tfhd, Tkhd, Trex, Trun,
};
use mp4forge::boxes::iso14496_14::Esds;
use mp4forge::boxes::iso14496_15::VVCDecoderConfiguration;
use mp4forge::boxes::iso23001_7::Tenc;
use mp4forge::boxes::opus::DOps;
use mp4forge::codec::{CodecBox, marshal, unmarshal};

use support::FuzzInput;

const MAX_PAYLOAD_LEN: usize = 4096;

fuzz_target!(|data: &[u8]| {
    let mut input = FuzzInput::new(data);
    let payload = input.take_bytes(MAX_PAYLOAD_LEN);

    match input.take_u8() % 30 {
        0 => decode_then_encode::<Mvhd>(&payload),
        1 => decode_then_encode::<Tkhd>(&payload),
        2 => decode_then_encode::<Mdhd>(&payload),
        3 => decode_then_encode::<Hdlr>(&payload),
        4 => decode_then_encode::<Tfdt>(&payload),
        5 => decode_then_encode::<Tfhd>(&payload),
        6 => decode_then_encode::<Trun>(&payload),
        7 => decode_then_encode::<Trex>(&payload),
        8 => decode_then_encode::<Sidx>(&payload),
        9 => decode_then_encode::<Sbgp>(&payload),
        10 => decode_then_encode::<Sgpd>(&payload),
        11 => decode_then_encode::<Saiz>(&payload),
        12 => decode_then_encode::<Saio>(&payload),
        13 => decode_then_encode::<Tenc>(&payload),
        14 => decode_then_encode::<Schm>(&payload),
        15 => decode_then_encode::<Frma>(&payload),
        16 => decode_then_encode::<Esds>(&payload),
        17 => decode_then_encode::<AVCDecoderConfiguration>(&payload),
        18 => decode_then_encode::<HEVCDecoderConfiguration>(&payload),
        19 => decode_then_encode::<VVCDecoderConfiguration>(&payload),
        20 => decode_then_encode::<Btrt>(&payload),
        21 => decode_then_encode::<Clap>(&payload),
        22 => decode_then_encode::<CoLL>(&payload),
        23 => decode_then_encode::<Colr>(&payload),
        24 => decode_then_encode::<Pasp>(&payload),
        25 => decode_then_encode::<Dac3>(&payload),
        26 => decode_then_encode::<Dec3>(&payload),
        27 => decode_then_encode::<Dac4>(&payload),
        28 => decode_then_encode::<DOps>(&payload),
        _ => decode_all_defaults(),
    }
});

fn decode_then_encode<B>(payload: &[u8])
where
    B: CodecBox + Default,
{
    let mut decoded = B::default();
    if let Ok(read) = unmarshal(
        &mut Cursor::new(payload),
        payload.len() as u64,
        &mut decoded,
        None,
    ) {
        assert!(read <= payload.len() as u64);
        let mut encoded = Vec::new();
        let _ = marshal(&mut encoded, &decoded, None);
    }
}

fn decode_all_defaults() {
    encode_default::<Mvhd>();
    encode_default::<Tkhd>();
    encode_default::<Mdhd>();
    encode_default::<Tfdt>();
    encode_default::<Tfhd>();
    encode_default::<Trun>();
    encode_default::<Trex>();
    encode_default::<Sidx>();
    encode_default::<Saiz>();
    encode_default::<Saio>();
    encode_default::<Tenc>();
    encode_default::<Schm>();
    encode_default::<Frma>();
}

fn encode_default<B>()
where
    B: CodecBox + Default,
{
    let mut encoded = Vec::new();
    let _ = marshal(&mut encoded, &B::default(), None);
}
