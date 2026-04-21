use std::str::FromStr;

use mp4forge::FourCc;

#[test]
fn display_formats_printable_values() {
    assert_eq!(FourCc::from_bytes(*b"1234").to_string(), "1234");
    assert_eq!(FourCc::from_bytes(*b"abcd").to_string(), "abcd");
    assert_eq!(FourCc::from_bytes(*b"xx x").to_string(), "xx x");
    assert_eq!(FourCc::from_bytes(*b"xx~x").to_string(), "xx~x");
    assert_eq!(
        FourCc::from_bytes([b'x', b'x', 0xa9, b'x']).to_string(),
        "xx(c)x"
    );
    assert_eq!(
        FourCc::from_bytes([b'x', b'x', 0xab, b'x']).to_string(),
        "0x7878ab78"
    );
}

#[test]
fn wildcard_matching_accepts_any_marker() {
    let pssh = FourCc::from_str("pssh").unwrap();
    let free = FourCc::from_str("free").unwrap();

    assert!(FourCc::ANY.matches(pssh));
    assert!(pssh.matches(FourCc::ANY));
    assert!(pssh.matches(pssh));
    assert!(!pssh.matches(free));
}

#[test]
fn parse_requires_exactly_four_bytes() {
    let error = FourCc::try_from("abc").unwrap_err();
    assert_eq!(error.len(), 3);
    assert_eq!(
        error.to_string(),
        "fourcc values must be exactly 4 bytes, got 3"
    );
}

#[test]
fn big_endian_u32_conversion_matches_byte_layout() {
    let value = FourCc::from_u32(0x7465_7374);
    assert_eq!(value.as_bytes(), b"test");
}
