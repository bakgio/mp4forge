use crate::FourCc;
use crate::boxes::iso14496_12::{Colr, Pasp, SampleEntry, VisualSampleEntry};

use super::super::MuxError;

pub(super) const SAMPLE_ENTRY_UNCV: FourCc = FourCc::from_bytes(*b"uncv");
const CMPD: FourCc = FourCc::from_bytes(*b"cmpd");
const COLR_NCLC: FourCc = FourCc::from_bytes(*b"nclc");
const COLR_NCLX: FourCc = FourCc::from_bytes(*b"nclx");
const MJP2: FourCc = FourCc::from_bytes(*b"mjp2");
const UNCC: FourCc = FourCc::from_bytes(*b"uncC");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum UncvPixelLayout {
    Grey8,
    AlphaGrey8,
    GreyAlpha8,
    Rgb332,
    Rgb444,
    Rgb555,
    Rgb565,
    Rgb24,
    Bgr24,
    Rgbx32,
    Bgrx32,
    Xrgb32,
    Xbgr32,
    Argb32,
    Rgba32,
    Bgra32,
    Abgr32,
    Rgbd32,
    Rgbds32,
    Yuv420p8,
    Yvu420p8,
    Yuva420p8,
    Yuvd420p8,
    Yuv420p10,
    Yuv422p8,
    Yuv422p10,
    Yuv444p8,
    Yuv444p10,
    Yuva444p8,
    Nv12p8,
    Nv21p8,
    Nv12p10,
    Nv21p10,
    Uyvy422p8,
    Vyuy422p8,
    Yuyv422p8,
    Yvyu422p8,
    Uyvy422p10,
    Vyuy422p10,
    Yuyv422p10,
    Yvyu422p10,
    Yuv444Packed8,
    Vyu444Packed8,
    Yuva444Packed8,
    Uyva444Packed8,
    Yuv444Packed10,
    V210,
}

impl UncvPixelLayout {
    fn cmpd_component_ids(self) -> &'static [u16] {
        match self {
            Self::Grey8 => &[0],
            Self::AlphaGrey8 => &[7, 0],
            Self::GreyAlpha8 => &[0, 7],
            Self::Rgb332 | Self::Rgb444 | Self::Rgb555 | Self::Rgb565 | Self::Rgb24 => &[4, 5, 6],
            Self::Rgbd32 => &[4, 5, 6, 8],
            Self::Bgr24 => &[6, 5, 4],
            Self::Rgbds32 => &[4, 5, 6, 8, 7],
            Self::Rgbx32 => &[4, 5, 6, 12],
            Self::Bgrx32 => &[6, 5, 4, 12],
            Self::Xrgb32 => &[12, 4, 5, 6],
            Self::Xbgr32 => &[12, 6, 5, 4],
            Self::Argb32 => &[7, 4, 5, 6],
            Self::Rgba32 => &[4, 5, 6, 7],
            Self::Bgra32 => &[6, 5, 4, 7],
            Self::Abgr32 => &[7, 6, 5, 4],
            Self::Yuv420p8
            | Self::Yuv420p10
            | Self::Yuv422p8
            | Self::Yuv422p10
            | Self::Yuv444p8
            | Self::Yuv444p10
            | Self::Nv12p8
            | Self::Nv12p10
            | Self::Yuv444Packed8 => &[1, 2, 3],
            Self::Yvu420p8 | Self::Nv21p8 | Self::Nv21p10 => &[1, 3, 2],
            Self::Yuva420p8 | Self::Yuva444p8 | Self::Yuva444Packed8 => &[1, 2, 3, 7],
            Self::Yuvd420p8 => &[1, 2, 3, 8],
            Self::Uyvy422p8 | Self::Uyvy422p10 | Self::V210 => &[2, 1, 3, 1],
            Self::Vyuy422p8 | Self::Vyuy422p10 => &[3, 1, 2, 1],
            Self::Yuyv422p8 | Self::Yuyv422p10 => &[1, 2, 1, 3],
            Self::Yvyu422p8 | Self::Yvyu422p10 => &[1, 3, 1, 2],
            Self::Vyu444Packed8 => &[3, 1, 2],
            Self::Uyva444Packed8 => &[2, 1, 3, 7],
            Self::Yuv444Packed10 => &[2, 1, 3],
        }
    }

    fn component_bit_depths(self) -> &'static [u8] {
        match self {
            Self::Rgb332 => &[3, 3, 2],
            Self::Rgb444 => &[4, 4, 4],
            Self::Rgb555 => &[5, 5, 5],
            Self::Rgb565 => &[5, 6, 5],
            Self::Rgbds32 => &[8, 8, 8, 7, 1],
            Self::Yuv420p10 | Self::Yuv422p10 | Self::Yuv444p10 | Self::Nv12p10 | Self::Nv21p10 => {
                &[10, 10, 10]
            }
            Self::Uyvy422p10 | Self::Vyuy422p10 | Self::Yuyv422p10 | Self::Yvyu422p10 => {
                &[10, 10, 10, 10]
            }
            Self::V210 => &[10, 10, 10, 8],
            Self::Yuv444Packed10 => &[10, 10, 10],
            Self::Grey8 => &[8],
            Self::AlphaGrey8 | Self::GreyAlpha8 => &[8, 8],
            Self::Rgb24
            | Self::Bgr24
            | Self::Yuv420p8
            | Self::Yvu420p8
            | Self::Yuv422p8
            | Self::Yuv444p8
            | Self::Nv12p8
            | Self::Nv21p8
            | Self::Yuv444Packed8
            | Self::Vyu444Packed8 => &[8, 8, 8],
            Self::Rgbx32
            | Self::Bgrx32
            | Self::Xrgb32
            | Self::Xbgr32
            | Self::Argb32
            | Self::Rgba32
            | Self::Bgra32
            | Self::Abgr32
            | Self::Rgbd32
            | Self::Yuva420p8
            | Self::Yuvd420p8
            | Self::Yuva444p8
            | Self::Uyvy422p8
            | Self::Vyuy422p8
            | Self::Yuyv422p8
            | Self::Yvyu422p8
            | Self::Yuva444Packed8
            | Self::Uyva444Packed8 => &[8, 8, 8, 8],
        }
    }

    fn component_format(self) -> u8 {
        match self {
            Self::Yuv420p10
            | Self::Yuv422p10
            | Self::Yuv444p10
            | Self::Nv12p10
            | Self::Nv21p10
            | Self::Uyvy422p10
            | Self::Vyuy422p10
            | Self::Yuyv422p10
            | Self::Yvyu422p10 => 2,
            _ => 0,
        }
    }

    fn sampling(self) -> u8 {
        match self {
            Self::Yuv420p8
            | Self::Yvu420p8
            | Self::Yuva420p8
            | Self::Yuvd420p8
            | Self::Yuv420p10
            | Self::Nv12p8
            | Self::Nv21p8
            | Self::Nv12p10
            | Self::Nv21p10 => 2,
            Self::Yuv422p8
            | Self::Yuv422p10
            | Self::Uyvy422p8
            | Self::Vyuy422p8
            | Self::Yuyv422p8
            | Self::Yvyu422p8
            | Self::Uyvy422p10
            | Self::Vyuy422p10
            | Self::Yuyv422p10
            | Self::Yvyu422p10
            | Self::V210 => 1,
            _ => 0,
        }
    }

    fn interleave(self) -> u8 {
        match self {
            Self::Yuv420p8
            | Self::Yvu420p8
            | Self::Yuva420p8
            | Self::Yuvd420p8
            | Self::Yuv420p10
            | Self::Yuv422p8
            | Self::Yuv422p10
            | Self::Yuv444p8
            | Self::Yuv444p10
            | Self::Yuva444p8 => 0,
            Self::Nv12p8 | Self::Nv21p8 | Self::Nv12p10 | Self::Nv21p10 => 2,
            Self::Uyvy422p8
            | Self::Vyuy422p8
            | Self::Yuyv422p8
            | Self::Yvyu422p8
            | Self::Uyvy422p10
            | Self::Vyuy422p10
            | Self::Yuyv422p10
            | Self::Yvyu422p10 => 5,
            Self::Grey8
            | Self::AlphaGrey8
            | Self::GreyAlpha8
            | Self::Rgb332
            | Self::Rgb444
            | Self::Rgb555
            | Self::Rgb565
            | Self::Rgb24
            | Self::Bgr24
            | Self::Rgbx32
            | Self::Bgrx32
            | Self::Xrgb32
            | Self::Xbgr32
            | Self::Argb32
            | Self::Rgba32
            | Self::Bgra32
            | Self::Abgr32
            | Self::Rgbd32
            | Self::Rgbds32
            | Self::Yuv444Packed8
            | Self::Vyu444Packed8
            | Self::Yuva444Packed8
            | Self::Uyva444Packed8
            | Self::Yuv444Packed10
            | Self::V210 => 1,
        }
    }

    fn block_size(self) -> u8 {
        match self {
            Self::Yuv444Packed10 | Self::V210 => 4,
            _ => 0,
        }
    }

    fn block_flags(self) -> u8 {
        match self {
            Self::Yuv444Packed10 => return 0x78,
            Self::V210 => return 0x38,
            _ => {}
        }
        let is_ten_bit = matches!(
            self,
            Self::Yuv420p10
                | Self::Yuv422p10
                | Self::Yuv444p10
                | Self::Nv12p10
                | Self::Nv21p10
                | Self::Uyvy422p10
                | Self::Vyuy422p10
                | Self::Yuyv422p10
                | Self::Yvyu422p10
        );
        let block_pad_lsb = false;
        let block_little_endian = false;
        let block_reversed = false;
        ((u8::from(is_ten_bit)) << 7)
            | ((u8::from(block_pad_lsb)) << 6)
            | ((u8::from(block_little_endian)) << 5)
            | ((u8::from(block_reversed)) << 4)
            | 0x08
    }

    fn uncc_profile(self) -> Option<FourCc> {
        match self {
            Self::Rgb24 => Some(FourCc::from_bytes(*b"rgb3")),
            Self::Abgr32 => Some(FourCc::from_bytes(*b"abgr")),
            Self::Rgba32 => Some(FourCc::from_bytes(*b"rgba")),
            Self::Yuv420p8 => Some(FourCc::from_bytes(*b"i420")),
            Self::Nv12p8 => Some(FourCc::from_bytes(*b"nv12")),
            Self::Nv21p8 => Some(FourCc::from_bytes(*b"nv21")),
            Self::Uyvy422p8 | Self::Uyvy422p10 => Some(FourCc::from_bytes(*b"2vuy")),
            Self::Vyuy422p8 | Self::Vyuy422p10 => Some(FourCc::from_bytes(*b"vyuy")),
            Self::Yuyv422p8 | Self::Yuyv422p10 => Some(FourCc::from_bytes(*b"yuv2")),
            Self::Yvyu422p8 | Self::Yvyu422p10 => Some(FourCc::from_bytes(*b"yvyu")),
            Self::Yuv444p8 => Some(FourCc::from_bytes(*b"v308")),
            Self::Vyu444Packed8 => Some(FourCc::from_bytes(*b"v308")),
            Self::Uyva444Packed8 => Some(FourCc::from_bytes(*b"v408")),
            Self::Yuv444Packed10 => Some(FourCc::from_bytes(*b"v410")),
            Self::V210 => Some(FourCc::from_bytes(*b"v210")),
            Self::Grey8
            | Self::AlphaGrey8
            | Self::GreyAlpha8
            | Self::Rgb332
            | Self::Rgb444
            | Self::Rgb555
            | Self::Rgb565
            | Self::Bgr24
            | Self::Rgbx32
            | Self::Bgrx32
            | Self::Xrgb32
            | Self::Xbgr32
            | Self::Argb32
            | Self::Bgra32
            | Self::Rgbd32
            | Self::Rgbds32
            | Self::Yvu420p8
            | Self::Yuva420p8
            | Self::Yuvd420p8
            | Self::Yuv420p10
            | Self::Yuv422p8
            | Self::Yuv422p10
            | Self::Yuv444p10
            | Self::Yuva444p8
            | Self::Nv12p10
            | Self::Nv21p10
            | Self::Yuv444Packed8
            | Self::Yuva444Packed8 => None,
        }
    }
}

pub(super) fn build_uncv_sample_entry_box(
    width: u16,
    height: u16,
    layout: UncvPixelLayout,
    include_pasp: bool,
    include_nclx_colr: bool,
) -> Result<Vec<u8>, MuxError> {
    let mut child_boxes = vec![build_uncv_cmpd_box(layout)?, build_uncv_uncc_box(layout)?];
    if include_pasp {
        child_boxes.push(build_pasp_box(1, 1)?);
    }
    if include_nclx_colr {
        child_boxes.push(build_nclx_colr_box(1, 1, 1, false)?);
    }
    let mut compressorname = [0_u8; 32];
    compressorname[0] = 8;
    compressorname[1..9].copy_from_slice(b"RawVideo");
    super::super::mp4::encode_typed_box(
        &VisualSampleEntry {
            sample_entry: SampleEntry {
                box_type: SAMPLE_ENTRY_UNCV,
                data_reference_index: 1,
            },
            pre_defined2: [0, 0, 0],
            width,
            height,
            // The retained reference raw-video authoring writes literal 72 here, not 16.16
            // fixed-point.
            horizresolution: 72,
            vertresolution: 72,
            frame_count: 1,
            compressorname,
            depth: 0x0018,
            pre_defined3: -1,
            ..VisualSampleEntry::default()
        },
        &child_boxes.concat(),
    )
}

pub(super) fn build_mjp2_sample_entry_box(
    width: u16,
    height: u16,
    compressor_name: &[u8],
    jp2h_payload: Option<&[u8]>,
) -> Result<Vec<u8>, MuxError> {
    let jp2h_box = super::super::mp4::encode_raw_box(
        FourCc::from_bytes(*b"jp2h"),
        jp2h_payload.unwrap_or(&[]),
    )?;
    super::super::import::build_visual_sample_entry_box_with_compressor_name(
        MJP2,
        width,
        height,
        compressor_name,
        &[jp2h_box],
    )
}

pub(super) fn build_prores_sample_entry_box(
    sample_entry_type: FourCc,
    width: u16,
    height: u16,
    compressor_name: &[u8],
    colour_primaries: u16,
    transfer_characteristics: u16,
    matrix_coefficients: u16,
) -> Result<Vec<u8>, MuxError> {
    let mut compressorname = [0_u8; 32];
    let visible_len = compressor_name.len().min(31);
    compressorname[0] =
        u8::try_from(visible_len).map_err(|_| MuxError::LayoutOverflow("compressor name"))?;
    compressorname[1..1 + visible_len].copy_from_slice(&compressor_name[..visible_len]);
    let child_boxes = [
        build_pasp_box(1, 1)?,
        build_nclc_colr_box(
            colour_primaries,
            transfer_characteristics,
            matrix_coefficients,
        )?,
    ];
    super::super::mp4::encode_typed_box(
        &VisualSampleEntry {
            sample_entry: SampleEntry {
                box_type: sample_entry_type,
                data_reference_index: 1,
            },
            width,
            height,
            // The retained reference QuickTime-style ProRes authoring writes literal 72 here.
            horizresolution: 72,
            vertresolution: 72,
            frame_count: 1,
            compressorname,
            depth: 0x0018,
            pre_defined3: -1,
            ..VisualSampleEntry::default()
        },
        &child_boxes.concat(),
    )
}

fn build_uncv_cmpd_box(layout: UncvPixelLayout) -> Result<Vec<u8>, MuxError> {
    let component_ids = layout.cmpd_component_ids();
    let mut payload = Vec::with_capacity(4 + component_ids.len() * 2);
    payload.extend_from_slice(
        &u32::try_from(component_ids.len())
            .map_err(|_| MuxError::LayoutOverflow("uncv component count"))?
            .to_be_bytes(),
    );
    for component_id in component_ids {
        payload.extend_from_slice(&component_id.to_be_bytes());
    }
    super::super::mp4::encode_raw_box(CMPD, &payload)
}

fn build_uncv_uncc_box(layout: UncvPixelLayout) -> Result<Vec<u8>, MuxError> {
    let component_ids = layout.cmpd_component_ids();
    let component_bits = layout.component_bit_depths();
    let mut payload = Vec::with_capacity(24 + component_ids.len() * 5 + 20);
    payload.extend_from_slice(&0_u32.to_be_bytes());
    payload.extend_from_slice(
        layout
            .uncc_profile()
            .unwrap_or(FourCc::from_bytes(*b"\0\0\0\0"))
            .as_bytes(),
    );
    payload.extend_from_slice(
        &u32::try_from(component_ids.len())
            .map_err(|_| MuxError::LayoutOverflow("uncv component count"))?
            .to_be_bytes(),
    );
    for (component_index, component_bits) in component_bits.iter().enumerate() {
        payload.extend_from_slice(
            &u16::try_from(component_index)
                .map_err(|_| MuxError::LayoutOverflow("uncv component index"))?
                .to_be_bytes(),
        );
        payload.push(component_bits.saturating_sub(1));
        payload.push(0);
        payload.push(layout.component_format());
    }
    payload.push(layout.sampling());
    payload.push(layout.interleave());
    payload.push(layout.block_size());
    payload.push(layout.block_flags());
    payload.extend_from_slice(&0_u32.to_be_bytes());
    payload.extend_from_slice(&0_u32.to_be_bytes());
    payload.extend_from_slice(&0_u32.to_be_bytes());
    payload.extend_from_slice(&0_u32.to_be_bytes());
    payload.extend_from_slice(&0_u32.to_be_bytes());
    super::super::mp4::encode_raw_box(UNCC, &payload)
}

fn build_pasp_box(h_spacing: u32, v_spacing: u32) -> Result<Vec<u8>, MuxError> {
    super::super::mp4::encode_typed_box(
        &Pasp {
            h_spacing,
            v_spacing,
        },
        &[],
    )
}

fn build_nclx_colr_box(
    colour_primaries: u16,
    transfer_characteristics: u16,
    matrix_coefficients: u16,
    full_range_flag: bool,
) -> Result<Vec<u8>, MuxError> {
    super::super::mp4::encode_typed_box(
        &Colr {
            colour_type: COLR_NCLX,
            colour_primaries,
            transfer_characteristics,
            matrix_coefficients,
            full_range_flag,
            reserved: 0,
            ..Colr::default()
        },
        &[],
    )
}

fn build_nclc_colr_box(
    colour_primaries: u16,
    transfer_characteristics: u16,
    matrix_coefficients: u16,
) -> Result<Vec<u8>, MuxError> {
    let mut unknown = Vec::with_capacity(6);
    unknown.extend_from_slice(&colour_primaries.to_be_bytes());
    unknown.extend_from_slice(&transfer_characteristics.to_be_bytes());
    unknown.extend_from_slice(&matrix_coefficients.to_be_bytes());
    super::super::mp4::encode_typed_box(
        &Colr {
            colour_type: COLR_NCLC,
            unknown,
            ..Colr::default()
        },
        &[],
    )
}
