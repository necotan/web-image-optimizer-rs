use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use fast_image_resize::images::Image;
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use img_parts::Bytes;
use zune_jpeg::JpegDecoder;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

use crate::encode::{Encoder, Frame, MozjpegEncoder, PixelFormat, Segments};
use crate::error::ProcessError;
use crate::metadata;
use crate::orient;
use crate::preset::Settings;

/// JPEG の仕様上の上限
// zune-jpeg の既定値はこれより小さい
const MAX_DIMENSION: usize = 65535;

/// メモリ確保の失敗は unwind せずプロセスごと止まるので、確保の前に断る
const MAX_PIXELS: u64 = 500_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Warning {
    /// EXIF を解析できず、EXIF なしで出力した
    ExifRemoved,
    /// メタデータのセグメントを読めず、EXIF も ICC もなしで出力した
    MetadataUnreadable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReport {
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub warnings: Vec<Warning>,
}

/// 8 ビット RGB
// グレースケールも RGB に広げる
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    /// 縮小と回転を済ませた、エンコード前の画像
    pub reference: RgbImage,
    /// 出力する JPEG を読み直した画像
    pub output: RgbImage,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub warnings: Vec<Warning>,
}

struct Converted {
    jpeg: Vec<u8>,
    warnings: Vec<Warning>,
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    format: PixelFormat,
}

pub fn process_file(input: &Path, output: &Path, s: &Settings) -> Result<FileReport, ProcessError> {
    s.validate()?;
    let data = fs::read(input).map_err(ProcessError::Read)?;
    let input_bytes = data.len() as u64;
    // 書き込み中に縮小後の画素を持ち続けないよう、ここで捨てる
    let Converted { jpeg, warnings, .. } = convert(Bytes::from(data), s, &MozjpegEncoder)?;
    write_atomically(output, &jpeg).map_err(ProcessError::Write)?;
    Ok(FileReport {
        input_bytes,
        output_bytes: jpeg.len() as u64,
        warnings,
    })
}

pub fn process_to_memory(input: &Path, s: &Settings) -> Result<Vec<u8>, ProcessError> {
    s.validate()?;
    let data = fs::read(input).map_err(ProcessError::Read)?;
    Ok(convert(Bytes::from(data), s, &MozjpegEncoder)?.jpeg)
}

pub fn preview(input: &Path, s: &Settings) -> Result<Preview, ProcessError> {
    s.validate()?;
    let data = fs::read(input).map_err(ProcessError::Read)?;
    let input_bytes = data.len() as u64;
    let converted = convert(Bytes::from(data), s, &MozjpegEncoder)?;
    let reference = to_rgb(
        converted.pixels,
        converted.width,
        converted.height,
        converted.format,
    );
    let output_bytes = converted.jpeg.len() as u64;
    let decoded = decode(&converted.jpeg)?;
    drop(converted.jpeg);
    let output = to_rgb(
        decoded.pixels,
        decoded.width,
        decoded.height,
        decoded.format,
    );
    Ok(Preview {
        reference,
        output,
        input_bytes,
        output_bytes,
        warnings: converted.warnings,
    })
}

fn to_rgb(pixels: Vec<u8>, width: u32, height: u32, format: PixelFormat) -> RgbImage {
    let pixels = match format {
        PixelFormat::Rgb => pixels,
        PixelFormat::Gray => pixels.iter().flat_map(|&v| [v, v, v]).collect(),
    };
    RgbImage {
        width,
        height,
        pixels,
    }
}

fn convert(input: Bytes, s: &Settings, encoder: &impl Encoder) -> Result<Converted, ProcessError> {
    let mut extracted = metadata::extract(input.clone());
    let mut warnings = Vec::new();
    if extracted.unreadable {
        warnings.push(Warning::MetadataUnreadable);
    }
    if extracted.exif_failed {
        warnings.push(Warning::ExifRemoved);
    }

    let decoded = decode(&input)?;
    drop(input);

    let orientation = extracted
        .exif
        .as_ref()
        .map_or(1, metadata::Exif::orientation);
    let channels = decoded.format.channels();
    let (pixels, width, height) = orient::apply(
        decoded.pixels,
        decoded.width,
        decoded.height,
        channels,
        orientation,
    );

    let (out_width, out_height) = fit_inside(width, height, s.long_edge);
    let pixels = if (out_width, out_height) == (width, height) {
        pixels
    } else {
        resize(pixels, width, height, out_width, out_height, decoded.format)?
    };

    let exif = match extracted.exif.take() {
        Some(exif) => match exif.rebuild(out_width, out_height) {
            Ok(tiff) => Some(tiff),
            Err(_) => {
                warnings.push(Warning::ExifRemoved);
                None
            }
        },
        None => None,
    };

    let frame = Frame {
        pixels: &pixels,
        width: out_width,
        height: out_height,
        format: decoded.format,
    };
    let segments = Segments {
        exif: exif.as_deref(),
        icc: extracted.icc.as_deref(),
    };
    let jpeg = encoder.encode(&frame, s.quality, &segments)?;
    Ok(Converted {
        jpeg,
        warnings,
        pixels,
        width: out_width,
        height: out_height,
        format: decoded.format,
    })
}

struct Decoded {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    format: PixelFormat,
}

fn decode(input: &[u8]) -> Result<Decoded, ProcessError> {
    let decode_error = |e: zune_jpeg::errors::DecodeErrors| ProcessError::Decode(e.to_string());
    let options = DecoderOptions::default()
        .set_strict_mode(true)
        .set_max_width(MAX_DIMENSION)
        .set_max_height(MAX_DIMENSION);

    let mut headers = JpegDecoder::new_with_options(ZCursor::new(input), options);
    headers.decode_headers().map_err(decode_error)?;
    let (width, height) = headers
        .dimensions()
        .ok_or_else(|| ProcessError::Decode("missing dimensions".to_string()))?;
    let (width, height) = (width as u32, height as u32);
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(ProcessError::ImageTooLarge { width, height });
    }

    let (format, out_color_space) = match headers.input_colorspace() {
        Some(ColorSpace::Luma) => (PixelFormat::Gray, ColorSpace::Luma),
        Some(ColorSpace::RGB | ColorSpace::YCbCr) => (PixelFormat::Rgb, ColorSpace::RGB),
        // RGB へ単純変換すると色が変わり、CMYK 用の ICC も使えなくなる
        other => {
            return Err(ProcessError::UnsupportedColorSpace(format!("{other:?}")));
        }
    };

    let options = options.jpeg_set_out_colorspace(out_color_space);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(input), options);
    let pixels = decoder.decode().map_err(decode_error)?;
    Ok(Decoded {
        pixels,
        width,
        height,
        format,
    })
}

pub(crate) fn fit_inside(width: u32, height: u32, long_edge: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= long_edge {
        return (width, height);
    }
    let scale = f64::from(long_edge) / f64::from(longest);
    let shrink = |v: u32| ((f64::from(v) * scale).round() as u32).max(1);
    (shrink(width), shrink(height))
}

fn resize(
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    out_width: u32,
    out_height: u32,
    format: PixelFormat,
) -> Result<Vec<u8>, ProcessError> {
    let pixel_type = match format {
        PixelFormat::Rgb => PixelType::U8x3,
        PixelFormat::Gray => PixelType::U8,
    };
    let source = Image::from_vec_u8(width, height, pixels, pixel_type)
        .map_err(|e| ProcessError::Resize(e.to_string()))?;
    let mut target = Image::new(out_width, out_height, pixel_type);
    let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3));
    Resizer::new()
        .resize(&source, &mut target, &options)
        .map_err(|e| ProcessError::Resize(e.to_string()))?;
    Ok(target.into_vec())
}

fn temp_path(output: &Path) -> PathBuf {
    let mut name = output.as_os_str().to_os_string();
    name.push(OsString::from(".tmp"));
    PathBuf::from(name)
}

fn write_atomically(output: &Path, data: &[u8]) -> io::Result<()> {
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = temp_path(output);
    let result = fs::write(&temp, data).and_then(|()| fs::rename(&temp, output));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shrinks_by_the_long_edge() {
        assert_eq!(fit_inside(6000, 4000, 2400), (2400, 1600));
        assert_eq!(fit_inside(4000, 6000, 2400), (1600, 2400));
    }

    #[test]
    fn never_enlarges() {
        assert_eq!(fit_inside(800, 600, 2400), (800, 600));
        assert_eq!(fit_inside(2400, 1600, 2400), (2400, 1600));
    }

    #[test]
    fn rounds_to_nearest() {
        assert_eq!(fit_inside(1000, 333, 500), (500, 167));
    }

    #[test]
    fn keeps_at_least_one_pixel() {
        assert_eq!(fit_inside(10000, 1, 100), (100, 1));
        assert_eq!(fit_inside(1, 10000, 1), (1, 1));
    }

    #[test]
    fn temp_file_sits_next_to_the_output() {
        let output = Path::new("export").join("a.jpg");
        assert_eq!(temp_path(&output), Path::new("export").join("a.jpg.tmp"));
    }
}
