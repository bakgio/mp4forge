//! Shared CLI-only heuristics for command wrappers.

use crate::FourCc;

const CTTS: FourCc = FourCc::from_bytes(*b"ctts");
const CO64: FourCc = FourCc::from_bytes(*b"co64");
const ELST: FourCc = FourCc::from_bytes(*b"elst");
const EMSG: FourCc = FourCc::from_bytes(*b"emsg");
const ESDS: FourCc = FourCc::from_bytes(*b"esds");
const FTYP: FourCc = FourCc::from_bytes(*b"ftyp");
const PSSH: FourCc = FourCc::from_bytes(*b"pssh");
const SAIO: FourCc = FourCc::from_bytes(*b"saio");
const SAIZ: FourCc = FourCc::from_bytes(*b"saiz");
const SBGP: FourCc = FourCc::from_bytes(*b"sbgp");
const SDTP: FourCc = FourCc::from_bytes(*b"sdtp");
const SGPD: FourCc = FourCc::from_bytes(*b"sgpd");
const STCO: FourCc = FourCc::from_bytes(*b"stco");
const STSC: FourCc = FourCc::from_bytes(*b"stsc");
const STSS: FourCc = FourCc::from_bytes(*b"stss");
const STSZ: FourCc = FourCc::from_bytes(*b"stsz");
const STTS: FourCc = FourCc::from_bytes(*b"stts");
const TFRA: FourCc = FourCc::from_bytes(*b"tfra");
const TRUN: FourCc = FourCc::from_bytes(*b"trun");

/// Returns `true` when a supported box should be treated as a leaf for CLI expansion heuristics.
pub const fn should_have_no_children(box_type: FourCc) -> bool {
    matches!(
        box_type,
        EMSG | ESDS
            | FTYP
            | PSSH
            | CTTS
            | CO64
            | ELST
            | SAIO
            | SAIZ
            | SBGP
            | SDTP
            | SGPD
            | STCO
            | STSC
            | STSS
            | STSZ
            | STTS
            | TFRA
            | TRUN
    )
}
