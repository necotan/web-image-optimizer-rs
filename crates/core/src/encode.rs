use std::any::Any;
use std::panic::{self, AssertUnwindSafe};

use mozjpeg::{ColorSpace, Compress, Marker};

use crate::error::ProcessError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PixelFormat {
    Rgb,
    Gray,
}

impl PixelFormat {
    pub(crate) fn channels(self) -> usize {
        match self {
            PixelFormat::Rgb => 3,
            PixelFormat::Gray => 1,
        }
    }
}

pub(crate) struct Frame<'a> {
    pub(crate) pixels: &'a [u8],
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) format: PixelFormat,
}

pub(crate) struct Segments<'a> {
    pub(crate) exif: Option<&'a [u8]>,
    pub(crate) icc: Option<&'a [u8]>,
}

pub(crate) trait Encoder {
    fn encode(
        &self,
        frame: &Frame<'_>,
        quality: u8,
        segments: &Segments<'_>,
    ) -> Result<Vec<u8>, ProcessError>;
}

pub(crate) struct MozjpegEncoder;

impl Encoder for MozjpegEncoder {
    fn encode(
        &self,
        frame: &Frame<'_>,
        quality: u8,
        segments: &Segments<'_>,
    ) -> Result<Vec<u8>, ProcessError> {
        // mozjpeg は Result ではなく unwind でエラーを返す
        panic::catch_unwind(AssertUnwindSafe(|| {
            encode_with_mozjpeg(frame, quality, segments)
        }))
        .map_err(|payload| ProcessError::Encode(panic_message(payload.as_ref())))?
        .map_err(|e| ProcessError::Encode(e.to_string()))
    }
}

fn encode_with_mozjpeg(
    frame: &Frame<'_>,
    quality: u8,
    segments: &Segments<'_>,
) -> std::io::Result<Vec<u8>> {
    let color_space = match frame.format {
        PixelFormat::Rgb => ColorSpace::JCS_RGB,
        PixelFormat::Gray => ColorSpace::JCS_GRAYSCALE,
    };
    let mut compress = Compress::new(color_space);
    // sharp の既定値と同じ出力にする設定。クレートの既定値は小さいが 7〜23 倍遅い (design §11)
    compress.set_fastest_defaults();
    compress.set_optimize_coding(true);
    compress.set_size(frame.width as usize, frame.height as usize);
    compress.set_quality(f32::from(quality));
    if frame.format == PixelFormat::Rgb {
        compress.set_chroma_sampling_pixel_sizes((2, 2), (2, 2));
    }

    let mut started = compress.start_compress(Vec::new())?;
    if let Some(exif) = segments.exif {
        let payload = [b"Exif\0\0".as_slice(), exif].concat();
        started.write_marker(Marker::APP(1), &payload);
    }
    if let Some(icc) = segments.icc {
        // CompressStarted::write_icc_profile は連番を 0 から振るが、仕様は 1 始まり。
        // 0 番のチャンクを持つプロファイルはデコーダに無視され、色が変わってしまう
        let chunks: Vec<&[u8]> = icc.chunks(MAX_ICC_CHUNK).collect();
        for (index, chunk) in chunks.iter().enumerate() {
            let payload = icc_payload(index, chunks.len(), chunk);
            started.write_marker(Marker::APP(2), &payload);
        }
    }
    started.write_scanlines(frame.pixels)?;
    started.finish()
}

fn icc_payload(index: usize, total: usize, chunk: &[u8]) -> Vec<u8> {
    let sequence = [(index + 1) as u8, total as u8];
    [ICC_HEADER, sequence.as_slice(), chunk].concat()
}

const ICC_HEADER: &[u8] = b"ICC_PROFILE\0";

/// APP2 の最大ペイロードから、ヘッダと連番の 14 バイトを引いた値
const MAX_ICC_CHUNK: usize = 65533 - 14;

pub(crate) fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "unknown panic".to_string()
    }
}
