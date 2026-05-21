use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::{self, File};
#[cfg(feature = "async")]
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, BufReader as AsyncBufReader};

use super::super::import::StagedSample;
use super::super::{MuxError, MuxRawVideoParams, MuxRawVideoPixelFormat};
use super::raw_visual::{UncvPixelLayout, build_uncv_sample_entry_box};

pub(in crate::mux) struct ParsedRawVideoTrack {
    pub(in crate::mux) width: u16,
    pub(in crate::mux) height: u16,
    pub(in crate::mux) timescale: u32,
    pub(in crate::mux) sample_entry_box: Vec<u8>,
    pub(in crate::mux) samples: Vec<StagedSample>,
}

struct ParsedY4mHeader {
    width: u16,
    height: u16,
    timescale: u32,
    frame_duration: u32,
    frame_size: u64,
    sample_entry_box: Vec<u8>,
}

pub(in crate::mux) fn scan_y4m_file_sync(
    path: &Path,
    spec: &str,
) -> Result<ParsedRawVideoTrack, MuxError> {
    let file_size = std::fs::metadata(path)?.len();
    let file = std::fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    parse_y4m_reader_sync(spec, &mut reader, file_size)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_y4m_file_async(
    path: &Path,
    spec: &str,
) -> Result<ParsedRawVideoTrack, MuxError> {
    let file_size = fs::metadata(path).await?.len();
    let file = File::open(path).await?;
    let mut reader = AsyncBufReader::new(file);
    parse_y4m_reader_async(spec, &mut reader, file_size).await
}

pub(in crate::mux) fn scan_raw_video_file_sync(
    path: &Path,
    spec: &str,
    params: &MuxRawVideoParams,
) -> Result<ParsedRawVideoTrack, MuxError> {
    let file_size = std::fs::metadata(path)?.len();
    parse_raw_video_size(spec, file_size, params)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_raw_video_file_async(
    path: &Path,
    spec: &str,
    params: &MuxRawVideoParams,
) -> Result<ParsedRawVideoTrack, MuxError> {
    let file_size = fs::metadata(path).await?.len();
    parse_raw_video_size(spec, file_size, params)
}

fn parse_y4m_reader_sync<R>(
    spec: &str,
    reader: &mut R,
    file_size: u64,
) -> Result<ParsedRawVideoTrack, MuxError>
where
    R: BufRead + Seek,
{
    let mut header_bytes = Vec::new();
    if reader.read_until(b'\n', &mut header_bytes)? == 0 {
        return Err(invalid_y4m(
            spec,
            "Y4M input did not terminate its stream header line",
        ));
    }
    let header =
        std::str::from_utf8(header_bytes.strip_suffix(b"\n").ok_or_else(|| {
            invalid_y4m(spec, "Y4M input did not terminate its stream header line")
        })?)
        .map_err(|_| invalid_y4m(spec, "Y4M stream header is not valid UTF-8 text"))?;
    let parsed_header = parse_y4m_stream_header(spec, header)?;
    let mut physical_offset = u64::try_from(header_bytes.len())
        .map_err(|_| MuxError::LayoutOverflow("Y4M header length"))?;
    let mut retained_sample_offset = 0_u64;
    let mut samples = Vec::new();
    while physical_offset < file_size {
        let mut frame_header = Vec::new();
        if reader.read_until(b'\n', &mut frame_header)? == 0 {
            break;
        }
        let frame_header = std::str::from_utf8(
            frame_header
                .strip_suffix(b"\n")
                .ok_or_else(|| invalid_y4m(spec, "Y4M frame header is truncated"))?,
        )
        .map_err(|_| invalid_y4m(spec, "Y4M frame header is not valid UTF-8 text"))?;
        if !frame_header.starts_with("FRAME") {
            return Err(invalid_y4m(
                spec,
                "Y4M payload did not begin its frame headers with the FRAME marker",
            ));
        }
        physical_offset = physical_offset
            .checked_add(u64::try_from(frame_header.len() + 1).unwrap())
            .ok_or(MuxError::LayoutOverflow("Y4M frame header range"))?;
        if physical_offset
            .checked_add(parsed_header.frame_size)
            .is_none_or(|frame_end| frame_end > file_size)
        {
            return Err(invalid_y4m(
                spec,
                "Y4M frame payload overruns the input length",
            ));
        }
        samples.push(StagedSample {
            data_offset: retained_sample_offset,
            data_size: u32::try_from(parsed_header.frame_size).map_err(|_| {
                MuxError::LayoutOverflow("Y4M frame size exceeds MP4 sample limits")
            })?,
            duration: parsed_header.frame_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        retained_sample_offset = retained_sample_offset
            .checked_add(parsed_header.frame_size)
            .ok_or(MuxError::LayoutOverflow("Y4M retained sample offset"))?;
        physical_offset = physical_offset
            .checked_add(parsed_header.frame_size)
            .ok_or(MuxError::LayoutOverflow("Y4M frame payload range"))?;
        reader.seek(SeekFrom::Start(physical_offset))?;
    }
    finalize_y4m_track(
        spec,
        parsed_header.width,
        parsed_header.height,
        parsed_header.timescale,
        parsed_header.sample_entry_box,
        samples,
    )
}

#[cfg(feature = "async")]
async fn parse_y4m_reader_async<R>(
    spec: &str,
    reader: &mut R,
    file_size: u64,
) -> Result<ParsedRawVideoTrack, MuxError>
where
    R: tokio::io::AsyncBufRead + tokio::io::AsyncSeek + Unpin,
{
    let mut header_bytes = Vec::new();
    if reader.read_until(b'\n', &mut header_bytes).await? == 0 {
        return Err(invalid_y4m(
            spec,
            "Y4M input did not terminate its stream header line",
        ));
    }
    let header =
        std::str::from_utf8(header_bytes.strip_suffix(b"\n").ok_or_else(|| {
            invalid_y4m(spec, "Y4M input did not terminate its stream header line")
        })?)
        .map_err(|_| invalid_y4m(spec, "Y4M stream header is not valid UTF-8 text"))?;
    let parsed_header = parse_y4m_stream_header(spec, header)?;
    let mut physical_offset = u64::try_from(header_bytes.len())
        .map_err(|_| MuxError::LayoutOverflow("Y4M header length"))?;
    let mut retained_sample_offset = 0_u64;
    let mut samples = Vec::new();
    while physical_offset < file_size {
        let mut frame_header = Vec::new();
        if reader.read_until(b'\n', &mut frame_header).await? == 0 {
            break;
        }
        let frame_header = std::str::from_utf8(
            frame_header
                .strip_suffix(b"\n")
                .ok_or_else(|| invalid_y4m(spec, "Y4M frame header is truncated"))?,
        )
        .map_err(|_| invalid_y4m(spec, "Y4M frame header is not valid UTF-8 text"))?;
        if !frame_header.starts_with("FRAME") {
            return Err(invalid_y4m(
                spec,
                "Y4M payload did not begin its frame headers with the FRAME marker",
            ));
        }
        physical_offset = physical_offset
            .checked_add(u64::try_from(frame_header.len() + 1).unwrap())
            .ok_or(MuxError::LayoutOverflow("Y4M frame header range"))?;
        if physical_offset
            .checked_add(parsed_header.frame_size)
            .is_none_or(|frame_end| frame_end > file_size)
        {
            return Err(invalid_y4m(
                spec,
                "Y4M frame payload overruns the input length",
            ));
        }
        samples.push(StagedSample {
            data_offset: retained_sample_offset,
            data_size: u32::try_from(parsed_header.frame_size).map_err(|_| {
                MuxError::LayoutOverflow("Y4M frame size exceeds MP4 sample limits")
            })?,
            duration: parsed_header.frame_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        retained_sample_offset = retained_sample_offset
            .checked_add(parsed_header.frame_size)
            .ok_or(MuxError::LayoutOverflow("Y4M retained sample offset"))?;
        physical_offset = physical_offset
            .checked_add(parsed_header.frame_size)
            .ok_or(MuxError::LayoutOverflow("Y4M frame payload range"))?;
        reader.seek(SeekFrom::Start(physical_offset)).await?;
    }
    finalize_y4m_track(
        spec,
        parsed_header.width,
        parsed_header.height,
        parsed_header.timescale,
        parsed_header.sample_entry_box,
        samples,
    )
}

fn parse_y4m_stream_header(spec: &str, header: &str) -> Result<ParsedY4mHeader, MuxError> {
    if !header.starts_with("YUV4MPEG2 ") {
        return Err(invalid_y4m(
            spec,
            "input does not start with the YUV4MPEG2 stream signature",
        ));
    }

    let mut width = None::<u32>;
    let mut height = None::<u32>;
    let mut fps_num = None::<u32>;
    let mut fps_den = None::<u32>;
    let mut layout = None::<UncvPixelLayout>;
    for token in header.split_ascii_whitespace().skip(1) {
        if token.len() < 2 {
            continue;
        }
        match token.as_bytes()[0] {
            b'W' => {
                width = Some(token[1..].parse::<u32>().map_err(|_| {
                    invalid_y4m(spec, "Y4M stream header carried an invalid width token")
                })?);
            }
            b'H' => {
                height = Some(token[1..].parse::<u32>().map_err(|_| {
                    invalid_y4m(spec, "Y4M stream header carried an invalid height token")
                })?);
            }
            b'F' => {
                let (num, den) = token[1..].split_once(':').ok_or_else(|| {
                    invalid_y4m(
                        spec,
                        "Y4M stream header carried an invalid frame-rate token",
                    )
                })?;
                fps_num = Some(num.parse::<u32>().map_err(|_| {
                    invalid_y4m(
                        spec,
                        "Y4M stream header carried an invalid frame-rate numerator",
                    )
                })?);
                fps_den = Some(den.parse::<u32>().map_err(|_| {
                    invalid_y4m(
                        spec,
                        "Y4M stream header carried an invalid frame-rate denominator",
                    )
                })?);
            }
            b'C' => {
                layout = Some(match &token[1..] {
                    "420jpeg" | "420mpeg2" | "420paldv" | "420" => UncvPixelLayout::Yuv420p8,
                    "422" => UncvPixelLayout::Yuv422p8,
                    "444" => UncvPixelLayout::Yuv444p8,
                    "444alpha" => UncvPixelLayout::Yuva444p8,
                    "mono" => UncvPixelLayout::Grey8,
                    _ => {
                        return Err(invalid_y4m(
                            spec,
                            "Y4M stream header carried an unsupported chroma layout token",
                        ));
                    }
                });
            }
            _ => {}
        }
    }

    let width =
        width.ok_or_else(|| invalid_y4m(spec, "Y4M stream header did not declare width"))?;
    let height =
        height.ok_or_else(|| invalid_y4m(spec, "Y4M stream header did not declare height"))?;
    if width == 0 || height == 0 {
        return Err(invalid_y4m(
            spec,
            "Y4M stream header declared zero width or zero height",
        ));
    }
    let fps_num =
        fps_num.ok_or_else(|| invalid_y4m(spec, "Y4M stream header did not declare frame rate"))?;
    let fps_den = fps_den.ok_or_else(|| {
        invalid_y4m(
            spec,
            "Y4M stream header did not declare a complete frame-rate denominator",
        )
    })?;
    if fps_num == 0 || fps_den == 0 {
        return Err(invalid_y4m(
            spec,
            "Y4M stream header declared a zero frame-rate numerator or denominator",
        ));
    }
    let layout = layout.unwrap_or(UncvPixelLayout::Yuv420p8);
    validate_y4m_dimensions(spec, width, height, layout)?;
    let frame_size = y4m_frame_size(width, height, layout)?;
    let width_u16 = u16::try_from(width)
        .map_err(|_| invalid_y4m(spec, "Y4M width does not fit in an MP4 visual sample entry"))?;
    let height_u16 = u16::try_from(height).map_err(|_| {
        invalid_y4m(
            spec,
            "Y4M height does not fit in an MP4 visual sample entry",
        )
    })?;

    let sample_entry_box = build_uncv_sample_entry_box(
        width_u16,
        height_u16,
        layout,
        true,
        matches!(layout, UncvPixelLayout::Yuv420p8),
    )?;
    Ok(ParsedY4mHeader {
        width: width_u16,
        height: height_u16,
        timescale: fps_num,
        frame_duration: fps_den,
        frame_size,
        sample_entry_box,
    })
}

fn finalize_y4m_track(
    spec: &str,
    width: u16,
    height: u16,
    timescale: u32,
    sample_entry_box: Vec<u8>,
    samples: Vec<StagedSample>,
) -> Result<ParsedRawVideoTrack, MuxError> {
    if samples.is_empty() {
        return Err(invalid_y4m(
            spec,
            "Y4M input did not carry any FRAME payloads",
        ));
    }
    Ok(ParsedRawVideoTrack {
        width,
        height,
        timescale,
        sample_entry_box,
        samples,
    })
}

fn parse_raw_video_size(
    spec: &str,
    file_size: u64,
    params: &MuxRawVideoParams,
) -> Result<ParsedRawVideoTrack, MuxError> {
    let layout = layout_from_raw_video_pixel_format(params.pixel_format());
    validate_raw_video_dimensions(spec, params.width(), params.height(), layout)?;
    let frame_size = y4m_frame_size(params.width(), params.height(), layout)?;
    let frame_size_u32 = u32::try_from(frame_size)
        .map_err(|_| MuxError::LayoutOverflow("rawvideo frame size exceeds MP4 sample limits"))?;
    let width_u16 = u16::try_from(params.width()).map_err(|_| {
        invalid_raw_video(
            spec,
            "rawvideo width does not fit in an MP4 visual sample entry",
        )
    })?;
    let height_u16 = u16::try_from(params.height()).map_err(|_| {
        invalid_raw_video(
            spec,
            "rawvideo height does not fit in an MP4 visual sample entry",
        )
    })?;

    if file_size == 0 {
        return Err(invalid_raw_video(
            spec,
            "rawvideo input did not carry any frame payload bytes",
        ));
    }
    if !file_size.is_multiple_of(frame_size) {
        return Err(invalid_raw_video(
            spec,
            "rawvideo input length is not an exact multiple of the declared frame size",
        ));
    }
    let frame_count = file_size / frame_size;
    let sample_count = usize::try_from(frame_count)
        .map_err(|_| MuxError::LayoutOverflow("rawvideo sample count"))?;
    let mut samples = Vec::with_capacity(sample_count);
    let mut offset = 0_u64;
    for _ in 0..sample_count {
        samples.push(StagedSample {
            data_offset: offset,
            data_size: frame_size_u32,
            duration: params.fps_den(),
            composition_time_offset: 0,
            is_sync_sample: true,
        });
        offset = offset
            .checked_add(frame_size)
            .ok_or(MuxError::LayoutOverflow("rawvideo sample offset"))?;
    }

    let include_qt_raw_yuv_defaults = rawvideo_uses_qt_yuv_defaults(layout);
    let sample_entry_box = build_uncv_sample_entry_box(
        width_u16,
        height_u16,
        layout,
        include_qt_raw_yuv_defaults,
        include_qt_raw_yuv_defaults,
    )?;
    Ok(ParsedRawVideoTrack {
        width: width_u16,
        height: height_u16,
        timescale: params.fps_num(),
        sample_entry_box,
        samples,
    })
}

fn rawvideo_uses_qt_yuv_defaults(layout: UncvPixelLayout) -> bool {
    matches!(
        layout,
        UncvPixelLayout::Yuv420p8
            | UncvPixelLayout::Uyvy422p8
            | UncvPixelLayout::Yuyv422p8
            | UncvPixelLayout::Yvyu422p8
            | UncvPixelLayout::Uyvy422p10
            | UncvPixelLayout::Vyu444Packed8
            | UncvPixelLayout::Uyva444Packed8
            | UncvPixelLayout::Yuv444Packed10
            | UncvPixelLayout::V210
    )
}

fn layout_from_raw_video_pixel_format(pixel_format: MuxRawVideoPixelFormat) -> UncvPixelLayout {
    match pixel_format {
        MuxRawVideoPixelFormat::Yuv420p8 => UncvPixelLayout::Yuv420p8,
        MuxRawVideoPixelFormat::Yvu420p8 => UncvPixelLayout::Yvu420p8,
        MuxRawVideoPixelFormat::Yuv420p10 => UncvPixelLayout::Yuv420p10,
        MuxRawVideoPixelFormat::Yuv422p8 => UncvPixelLayout::Yuv422p8,
        MuxRawVideoPixelFormat::Yuv422p10 => UncvPixelLayout::Yuv422p10,
        MuxRawVideoPixelFormat::Yuv444p8 => UncvPixelLayout::Yuv444p8,
        MuxRawVideoPixelFormat::Yuv444p10 => UncvPixelLayout::Yuv444p10,
        MuxRawVideoPixelFormat::Yuva420p8 => UncvPixelLayout::Yuva420p8,
        MuxRawVideoPixelFormat::Yuvd420p8 => UncvPixelLayout::Yuvd420p8,
        MuxRawVideoPixelFormat::Yuva444p8 => UncvPixelLayout::Yuva444p8,
        MuxRawVideoPixelFormat::Nv12p8 => UncvPixelLayout::Nv12p8,
        MuxRawVideoPixelFormat::Nv21p8 => UncvPixelLayout::Nv21p8,
        MuxRawVideoPixelFormat::Nv12p10 => UncvPixelLayout::Nv12p10,
        MuxRawVideoPixelFormat::Nv21p10 => UncvPixelLayout::Nv21p10,
        MuxRawVideoPixelFormat::Uyvy422p8 => UncvPixelLayout::Uyvy422p8,
        MuxRawVideoPixelFormat::Vyuy422p8 => UncvPixelLayout::Vyuy422p8,
        MuxRawVideoPixelFormat::Yuyv422p8 => UncvPixelLayout::Yuyv422p8,
        MuxRawVideoPixelFormat::Yvyu422p8 => UncvPixelLayout::Yvyu422p8,
        MuxRawVideoPixelFormat::Uyvy422p10 => UncvPixelLayout::Uyvy422p10,
        MuxRawVideoPixelFormat::Vyuy422p10 => UncvPixelLayout::Vyuy422p10,
        MuxRawVideoPixelFormat::Yuyv422p10 => UncvPixelLayout::Yuyv422p10,
        MuxRawVideoPixelFormat::Yvyu422p10 => UncvPixelLayout::Yvyu422p10,
        MuxRawVideoPixelFormat::Yuv444Packed8 => UncvPixelLayout::Yuv444Packed8,
        MuxRawVideoPixelFormat::Vyu444Packed8 => UncvPixelLayout::Vyu444Packed8,
        MuxRawVideoPixelFormat::Yuva444Packed8 => UncvPixelLayout::Yuva444Packed8,
        MuxRawVideoPixelFormat::Uyva444Packed8 => UncvPixelLayout::Uyva444Packed8,
        MuxRawVideoPixelFormat::Yuv444Packed10 => UncvPixelLayout::Yuv444Packed10,
        MuxRawVideoPixelFormat::V210 => UncvPixelLayout::V210,
        MuxRawVideoPixelFormat::Grey8 => UncvPixelLayout::Grey8,
        MuxRawVideoPixelFormat::AlphaGrey8 => UncvPixelLayout::AlphaGrey8,
        MuxRawVideoPixelFormat::GreyAlpha8 => UncvPixelLayout::GreyAlpha8,
        MuxRawVideoPixelFormat::Rgb332 => UncvPixelLayout::Rgb332,
        MuxRawVideoPixelFormat::Rgb444 => UncvPixelLayout::Rgb444,
        MuxRawVideoPixelFormat::Rgb555 => UncvPixelLayout::Rgb555,
        MuxRawVideoPixelFormat::Rgb565 => UncvPixelLayout::Rgb565,
        MuxRawVideoPixelFormat::Rgb24 => UncvPixelLayout::Rgb24,
        MuxRawVideoPixelFormat::Bgr24 => UncvPixelLayout::Bgr24,
        MuxRawVideoPixelFormat::Rgbx32 => UncvPixelLayout::Rgbx32,
        MuxRawVideoPixelFormat::Bgrx32 => UncvPixelLayout::Bgrx32,
        MuxRawVideoPixelFormat::Xrgb32 => UncvPixelLayout::Xrgb32,
        MuxRawVideoPixelFormat::Xbgr32 => UncvPixelLayout::Xbgr32,
        MuxRawVideoPixelFormat::Argb32 => UncvPixelLayout::Argb32,
        MuxRawVideoPixelFormat::Rgba32 => UncvPixelLayout::Rgba32,
        MuxRawVideoPixelFormat::Bgra32 => UncvPixelLayout::Bgra32,
        MuxRawVideoPixelFormat::Abgr32 => UncvPixelLayout::Abgr32,
        MuxRawVideoPixelFormat::Rgbd32 => UncvPixelLayout::Rgbd32,
        MuxRawVideoPixelFormat::Rgbds32 => UncvPixelLayout::Rgbds32,
    }
}

fn validate_y4m_dimensions(
    spec: &str,
    width: u32,
    height: u32,
    layout: UncvPixelLayout,
) -> Result<(), MuxError> {
    match layout {
        UncvPixelLayout::Yuv420p8 => {
            if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
                return Err(invalid_y4m(
                    spec,
                    "Y4M 4:2:0 carriage requires even width and even height",
                ));
            }
        }
        UncvPixelLayout::Yuv422p8 => {
            if !width.is_multiple_of(2) {
                return Err(invalid_y4m(
                    spec,
                    "Y4M 4:2:2 carriage requires an even width",
                ));
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_raw_video_dimensions(
    _spec: &str,
    _width: u32,
    _height: u32,
    _layout: UncvPixelLayout,
) -> Result<(), MuxError> {
    Ok(())
}

fn y4m_frame_size(width: u32, height: u32, layout: UncvPixelLayout) -> Result<u64, MuxError> {
    let width = u64::from(width);
    let height = u64::from(height);
    let luma = checked_mul(width, height, "rawvideo luma plane size")?;
    match layout {
        UncvPixelLayout::Grey8 | UncvPixelLayout::Rgb332 => Ok(luma),
        UncvPixelLayout::AlphaGrey8
        | UncvPixelLayout::GreyAlpha8
        | UncvPixelLayout::Rgb444
        | UncvPixelLayout::Rgb555
        | UncvPixelLayout::Rgb565
        | UncvPixelLayout::Uyvy422p8
        | UncvPixelLayout::Vyuy422p8
        | UncvPixelLayout::Yuyv422p8
        | UncvPixelLayout::Yvyu422p8 => checked_mul(luma, 2, "rawvideo frame size"),
        UncvPixelLayout::Rgb24
        | UncvPixelLayout::Bgr24
        | UncvPixelLayout::Yuv444p8
        | UncvPixelLayout::Vyu444Packed8
        | UncvPixelLayout::Yuv444Packed8 => checked_mul(luma, 3, "rawvideo frame size"),
        UncvPixelLayout::Rgbx32
        | UncvPixelLayout::Bgrx32
        | UncvPixelLayout::Xrgb32
        | UncvPixelLayout::Xbgr32
        | UncvPixelLayout::Argb32
        | UncvPixelLayout::Rgba32
        | UncvPixelLayout::Bgra32
        | UncvPixelLayout::Abgr32
        | UncvPixelLayout::Rgbd32
        | UncvPixelLayout::Rgbds32
        | UncvPixelLayout::Yuva444p8
        | UncvPixelLayout::Yuva444Packed8
        | UncvPixelLayout::Uyva444Packed8
        | UncvPixelLayout::Yuv444Packed10
        | UncvPixelLayout::Uyvy422p10
        | UncvPixelLayout::Vyuy422p10
        | UncvPixelLayout::Yuyv422p10
        | UncvPixelLayout::Yvyu422p10 => checked_mul(luma, 4, "rawvideo frame size"),
        UncvPixelLayout::Yuv420p8 | UncvPixelLayout::Yvu420p8 => {
            let uv_height = height.div_ceil(2);
            let stride_uv = width.div_ceil(2);
            checked_add(
                luma,
                checked_mul(
                    checked_mul(stride_uv, uv_height, "rawvideo chroma plane size")?,
                    2,
                    "rawvideo 4:2:0 chroma size",
                )?,
                "rawvideo frame size",
            )
        }
        UncvPixelLayout::Yuva420p8 | UncvPixelLayout::Yuvd420p8 => {
            let uv_height = height.div_ceil(2);
            let stride_uv = width.div_ceil(2);
            checked_add(
                checked_mul(luma, 2, "rawvideo 4:2:0 alpha or depth luma size")?,
                checked_mul(
                    checked_mul(stride_uv, uv_height, "rawvideo chroma plane size")?,
                    2,
                    "rawvideo 4:2:0 chroma size",
                )?,
                "rawvideo frame size",
            )
        }
        UncvPixelLayout::Yuv420p10 => {
            let stride = checked_mul(width, 2, "rawvideo 10-bit luma stride")?;
            let uv_height = height.div_ceil(2);
            let stride_uv = stride.div_ceil(2);
            checked_add(
                checked_mul(stride, height, "rawvideo 10-bit luma size")?,
                checked_mul(
                    checked_mul(stride_uv, uv_height, "rawvideo 10-bit chroma plane size")?,
                    2,
                    "rawvideo 10-bit chroma size",
                )?,
                "rawvideo frame size",
            )
        }
        UncvPixelLayout::Yuv422p8 => {
            let stride_uv = width.div_ceil(2);
            checked_add(
                luma,
                checked_mul(
                    checked_mul(stride_uv, height, "rawvideo 4:2:2 chroma plane size")?,
                    2,
                    "rawvideo 4:2:2 chroma size",
                )?,
                "rawvideo frame size",
            )
        }
        UncvPixelLayout::Yuv422p10 => {
            let stride = checked_mul(width, 2, "rawvideo 10-bit 4:2:2 luma stride")?;
            let stride_uv = stride.div_ceil(2);
            checked_add(
                checked_mul(stride, height, "rawvideo 10-bit 4:2:2 luma size")?,
                checked_mul(
                    checked_mul(stride_uv, height, "rawvideo 10-bit 4:2:2 chroma plane size")?,
                    2,
                    "rawvideo 10-bit 4:2:2 chroma size",
                )?,
                "rawvideo frame size",
            )
        }
        UncvPixelLayout::Yuv444p10 => checked_mul(
            checked_mul(width, 2, "rawvideo 10-bit 4:4:4 stride")?,
            checked_mul(height, 3, "rawvideo 10-bit 4:4:4 plane factor")?,
            "rawvideo frame size",
        ),
        UncvPixelLayout::Nv12p8 | UncvPixelLayout::Nv21p8 => checked_mul(
            checked_mul(width, height, "rawvideo NV luma size")?,
            3,
            "rawvideo NV size numerator",
        )
        .map(|size| size / 2),
        UncvPixelLayout::Nv12p10 | UncvPixelLayout::Nv21p10 => checked_mul(
            checked_mul(
                checked_mul(width, 2, "rawvideo 10-bit NV stride")?,
                height,
                "rawvideo 10-bit NV luma size",
            )?,
            3,
            "rawvideo 10-bit NV size numerator",
        )
        .map(|size| size / 2),
        UncvPixelLayout::V210 => {
            let mut padded_width = width;
            while !padded_width.is_multiple_of(48) {
                padded_width = padded_width
                    .checked_add(1)
                    .ok_or(MuxError::LayoutOverflow("rawvideo v210 padded width"))?;
            }
            let stride = checked_mul(padded_width, 16, "rawvideo v210 stride numerator")? / 6;
            checked_mul(stride, height, "rawvideo v210 frame size")
        }
    }
}

fn checked_mul(lhs: u64, rhs: u64, label: &'static str) -> Result<u64, MuxError> {
    lhs.checked_mul(rhs).ok_or(MuxError::LayoutOverflow(label))
}

fn checked_add(lhs: u64, rhs: u64, label: &'static str) -> Result<u64, MuxError> {
    lhs.checked_add(rhs).ok_or(MuxError::LayoutOverflow(label))
}

fn invalid_y4m(spec: &str, message: &str) -> MuxError {
    MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: message.to_string(),
    }
}

fn invalid_raw_video(spec: &str, message: &str) -> MuxError {
    MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: message.to_string(),
    }
}
