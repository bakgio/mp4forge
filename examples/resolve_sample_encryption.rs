use std::env;
use std::error::Error;

use mp4forge::FourCc;
use mp4forge::boxes::iso14496_12::{Saiz, Sbgp, Sgpd};
use mp4forge::boxes::iso23001_7::{Senc, Tenc};
use mp4forge::encryption::{
    ResolvedSampleEncryptionSource, SampleEncryptionContext, resolve_sample_encryption,
};
use mp4forge::extract::extract_box_as_bytes;
use mp4forge::walk::BoxPath;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let Some(path) = env::args().nth(1) else {
        return Err("usage: cargo run --example resolve_sample_encryption -- <input.mp4>".into());
    };

    let input = std::fs::read(path)?;
    let tenc = extract_box_as_bytes::<Tenc>(
        &input,
        BoxPath::from([
            FourCc::from_bytes(*b"moov"),
            FourCc::from_bytes(*b"trak"),
            FourCc::from_bytes(*b"mdia"),
            FourCc::from_bytes(*b"minf"),
            FourCc::from_bytes(*b"stbl"),
            FourCc::from_bytes(*b"stsd"),
            FourCc::ANY,
            FourCc::from_bytes(*b"sinf"),
            FourCc::from_bytes(*b"schi"),
            FourCc::from_bytes(*b"tenc"),
        ]),
    )?
    .into_iter()
    .next();

    let senc = extract_box_as_bytes::<Senc>(
        &input,
        BoxPath::from([
            FourCc::from_bytes(*b"moof"),
            FourCc::from_bytes(*b"traf"),
            FourCc::from_bytes(*b"senc"),
        ]),
    )?
    .into_iter()
    .next()
    .ok_or("no senc box found")?;

    let sgpd = extract_box_as_bytes::<Sgpd>(
        &input,
        BoxPath::from([
            FourCc::from_bytes(*b"moof"),
            FourCc::from_bytes(*b"traf"),
            FourCc::from_bytes(*b"sgpd"),
        ]),
    )?
    .into_iter()
    .next();
    let sbgp = extract_box_as_bytes::<Sbgp>(
        &input,
        BoxPath::from([
            FourCc::from_bytes(*b"moof"),
            FourCc::from_bytes(*b"traf"),
            FourCc::from_bytes(*b"sbgp"),
        ]),
    )?
    .into_iter()
    .next();
    let saiz = extract_box_as_bytes::<Saiz>(
        &input,
        BoxPath::from([
            FourCc::from_bytes(*b"moof"),
            FourCc::from_bytes(*b"traf"),
            FourCc::from_bytes(*b"saiz"),
        ]),
    )?
    .into_iter()
    .next();

    let resolved = resolve_sample_encryption(
        &senc,
        SampleEncryptionContext {
            tenc: tenc.as_ref(),
            sgpd: sgpd.as_ref(),
            sbgp: sbgp.as_ref(),
            saiz: saiz.as_ref(),
        },
    )?;

    for sample in resolved.samples {
        let source = match sample.metadata_source {
            ResolvedSampleEncryptionSource::TrackEncryptionBox => "tenc".to_string(),
            ResolvedSampleEncryptionSource::SampleGroupDescription {
                group_description_index,
                description_index,
                fragment_local,
            } => format!(
                "sgpd(seig) group_description_index={} description_index={} fragment_local={}",
                group_description_index, description_index, fragment_local
            ),
        };

        println!(
            "sample {} source={} protected={} iv_len={} aux_size={}",
            sample.sample_index,
            source,
            sample.is_protected,
            sample.effective_initialization_vector().len(),
            sample.auxiliary_info_size
        );
    }

    Ok(())
}
