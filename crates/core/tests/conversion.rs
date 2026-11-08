use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use mozjpeg::{ColorSpace as MozColorSpace, Compress};
use wio_core::{
    BatchEvent, BatchSummary, Preset, ProcessError, Settings, SettingsError, Warning, process_file,
    process_to_memory, run_batch,
};
use zune_jpeg::JpegDecoder;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

const SETTINGS: Settings = Settings {
    long_edge: 2400,
    quality: 82,
};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        let id = NEXT_DIR.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("wio-test-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    fn write(&self, name: &str, data: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, data).unwrap();
        path
    }

    fn export(&self) -> PathBuf {
        self.0.join("export")
    }

    fn export_names(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(self.export()) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn encode(
    color_space: MozColorSpace,
    channels: usize,
    w: usize,
    h: usize,
    pixels: &[u8],
) -> Vec<u8> {
    assert_eq!(pixels.len(), w * h * channels);
    let mut compress = Compress::new(color_space);
    compress.set_fastest_defaults();
    compress.set_size(w, h);
    compress.set_quality(95.0);
    let mut started = compress.start_compress(Vec::new()).unwrap();
    started.write_scanlines(pixels).unwrap();
    started.finish().unwrap()
}

/// 左半分が赤、右半分が青の画像
fn two_color_jpeg(w: usize, h: usize) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(w * h * 3);
    for _ in 0..h {
        for x in 0..w {
            pixels.extend_from_slice(if x < w / 2 {
                &[220, 20, 20]
            } else {
                &[20, 20, 220]
            });
        }
    }
    encode(MozColorSpace::JCS_RGB, 3, w, h, &pixels)
}

fn segment(marker: u8, payload: &[u8]) -> Vec<u8> {
    let len = (payload.len() + 2) as u16;
    [&[0xFF, marker], len.to_be_bytes().as_slice(), payload].concat()
}

/// SOI の直後にセグメントを差し込む
fn with_segments(jpeg: &[u8], segments: &[Vec<u8>]) -> Vec<u8> {
    [&jpeg[..2], &segments.concat(), &jpeg[2..]].concat()
}

fn exif_segment(tiff: &[u8]) -> Vec<u8> {
    segment(0xE1, &[b"Exif\0\0".as_slice(), tiff].concat())
}

type RawEntry = (u16, u16, u32, Vec<u8>);

fn append_ifd(out: &mut Vec<u8>, entries: &[RawEntry], next: u32) -> u32 {
    let start = out.len();
    let mut data_at = start + 2 + entries.len() * 12 + 4;
    let mut head = (entries.len() as u16).to_le_bytes().to_vec();
    let mut tail = Vec::new();
    for (tag, field_type, count, value) in entries {
        head.extend(tag.to_le_bytes());
        head.extend(field_type.to_le_bytes());
        head.extend(count.to_le_bytes());
        if value.len() <= 4 {
            let mut inline = value.clone();
            inline.resize(4, 0);
            head.extend(inline);
        } else {
            head.extend((data_at as u32).to_le_bytes());
            data_at += value.len();
            tail.extend_from_slice(value);
        }
    }
    head.extend(next.to_le_bytes());
    out.extend(head);
    out.extend(tail);
    start as u32
}

fn tiff_with_gps(orientation: u16) -> Vec<u8> {
    let mut t = b"II\x2a\0\0\0\0\0".to_vec();
    let gps = append_ifd(
        &mut t,
        &[
            (0x0001, 2, 2, b"N\0".to_vec()),
            (0x001B, 7, 15, b"SENTINEL_GPS_ID".to_vec()),
        ],
        0,
    );
    let exif = append_ifd(
        &mut t,
        &[
            (0x9003, 2, 20, b"2026:08:04 12:34:56\0".to_vec()),
            (0x927C, 7, 19, b"SENTINEL_MAKER_NOTE".to_vec()),
            (0xA002, 4, 1, 6000u32.to_le_bytes().to_vec()),
            (0xA003, 4, 1, 4000u32.to_le_bytes().to_vec()),
        ],
        0,
    );
    let thumbnail = t.len() as u32;
    t.extend_from_slice(b"SENTINEL_THUMBNAIL");
    let ifd1 = append_ifd(
        &mut t,
        &[
            (0x0201, 4, 1, thumbnail.to_le_bytes().to_vec()),
            (0x0202, 4, 1, 18u32.to_le_bytes().to_vec()),
        ],
        0,
    );
    let ifd0 = append_ifd(
        &mut t,
        &[
            (0x010F, 2, 5, b"ACME\0".to_vec()),
            (0x0110, 2, 8, b"Model-X\0".to_vec()),
            (0x0112, 3, 1, orientation.to_le_bytes().to_vec()),
            (0x02BC, 1, 18, b"SENTINEL_EXIF_XMP\0".to_vec()),
            (0x8769, 4, 1, exif.to_le_bytes().to_vec()),
            (0x8825, 4, 1, gps.to_le_bytes().to_vec()),
        ],
        ifd1,
    );
    t[4..8].copy_from_slice(&ifd0.to_le_bytes());
    t
}

fn contains(data: &[u8], needle: &[u8]) -> bool {
    data.windows(needle.len()).any(|w| w == needle)
}

fn decode_rgb(jpeg: &[u8]) -> (Vec<u8>, usize, usize) {
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(jpeg), options);
    let pixels = decoder.decode().unwrap();
    let (w, h) = decoder.dimensions().unwrap();
    (pixels, w, h)
}

fn exif_payload(jpeg: &[u8]) -> Option<&[u8]> {
    let mut i = 2;
    while i + 4 <= jpeg.len() && jpeg[i] == 0xFF && jpeg[i + 1] != 0xDA {
        let len = u16::from_be_bytes([jpeg[i + 2], jpeg[i + 3]]) as usize;
        let payload = &jpeg[i + 4..i + 2 + len];
        if jpeg[i + 1] == 0xE1 && payload.starts_with(b"Exif\0\0") {
            return Some(&payload[6..]);
        }
        i += 2 + len;
    }
    None
}

fn convert(dir: &TempDir, name: &str, data: &[u8]) -> Vec<u8> {
    let input = dir.write(name, data);
    let output = dir.export().join(name);
    process_file(&input, &output, &SETTINGS).unwrap();
    fs::read(output).unwrap()
}

#[test]
fn location_data_is_removed_from_every_segment() {
    let dir = TempDir::new();
    let xmp = [
        b"http://ns.adobe.com/xap/1.0/\0".as_slice(),
        b"<exif:GPSLatitude>SENTINEL_XMP</exif:GPSLatitude>",
    ]
    .concat();
    let extended_xmp = [
        b"http://ns.adobe.com/xmp/extension/\0".as_slice(),
        b"0123456789ABCDEF0123456789ABCDEF",
        &21u32.to_be_bytes(),
        &0u32.to_be_bytes(),
        b"SENTINEL_EXTENDED_XMP",
    ]
    .concat();
    let iptc = [b"Photoshop 3.0\08BIM".as_slice(), b"SENTINEL_IPTC"].concat();
    let mpf = [b"MPF\0".as_slice(), b"SENTINEL_MPF"].concat();

    let source = with_segments(
        &two_color_jpeg(64, 32),
        &[
            exif_segment(&tiff_with_gps(1)),
            segment(0xE1, &xmp),
            segment(0xE1, &extended_xmp),
            segment(0xED, &iptc),
            segment(0xE2, &mpf),
            segment(0xFE, b"SENTINEL_COMMENT"),
        ],
    );
    let source = [source.as_slice(), b"SENTINEL_AFTER_EOI"].concat();

    let output = convert(&dir, "a.jpg", &source);

    for sentinel in [
        "SENTINEL_GPS_ID",
        "SENTINEL_MAKER_NOTE",
        "SENTINEL_THUMBNAIL",
        "SENTINEL_EXIF_XMP",
        "SENTINEL_XMP",
        "SENTINEL_EXTENDED_XMP",
        "SENTINEL_IPTC",
        "SENTINEL_MPF",
        "SENTINEL_COMMENT",
        "SENTINEL_AFTER_EOI",
    ] {
        assert!(
            contains(&source, sentinel.as_bytes()),
            "{sentinel} missing from the source"
        );
        assert!(
            !contains(&output, sentinel.as_bytes()),
            "{sentinel} remains"
        );
    }
    assert!(!contains(&output, b"http://ns.adobe.com"));
}

#[test]
fn keeps_exif_other_than_location() {
    let dir = TempDir::new();
    let source = with_segments(&two_color_jpeg(64, 32), &[exif_segment(&tiff_with_gps(1))]);
    let output = convert(&dir, "a.jpg", &source);
    let exif = exif_payload(&output).unwrap();
    for text in ["ACME", "Model-X", "2026:08:04 12:34:56"] {
        assert!(contains(exif, text.as_bytes()), "{text} is missing");
    }
}

#[test]
fn rotates_pixels_and_resets_orientation() {
    let dir = TempDir::new();
    let source = with_segments(&two_color_jpeg(64, 32), &[exif_segment(&tiff_with_gps(6))]);
    let output = convert(&dir, "a.jpg", &source);

    let (pixels, w, h) = decode_rgb(&output);
    assert_eq!((w, h), (32, 64));
    // 右に 90 度回すと、元の左半分（赤）が上半分になる
    let top = &pixels[(8 * w + 16) * 3..][..3];
    let bottom = &pixels[(56 * w + 16) * 3..][..3];
    assert!(top[0] > 150 && top[2] < 100, "top is {top:?}");
    assert!(bottom[2] > 150 && bottom[0] < 100, "bottom is {bottom:?}");

    let exif = exif_payload(&output).unwrap();
    assert!(contains(exif, &[0x12, 0x01, 3, 0, 1, 0, 0, 0, 1, 0]));
}

#[test]
fn shrinks_and_updates_pixel_dimensions() {
    let dir = TempDir::new();
    let source = with_segments(
        &two_color_jpeg(400, 200),
        &[exif_segment(&tiff_with_gps(1))],
    );
    let input = dir.write("a.jpg", &source);
    let output = dir.export().join("a.jpg");
    let settings = Settings {
        long_edge: 100,
        quality: 82,
    };
    process_file(&input, &output, &settings).unwrap();

    let data = fs::read(output).unwrap();
    let (_, w, h) = decode_rgb(&data);
    assert_eq!((w, h), (100, 50));
    let exif = exif_payload(&data).unwrap();
    assert!(contains(
        exif,
        &[0x02, 0xA0, 4, 0, 1, 0, 0, 0, 100, 0, 0, 0]
    ));
    assert!(contains(exif, &[0x03, 0xA0, 4, 0, 1, 0, 0, 0, 50, 0, 0, 0]));
}

#[test]
fn does_not_enlarge_small_images() {
    let dir = TempDir::new();
    let output = convert(&dir, "a.jpg", &two_color_jpeg(64, 32));
    let (_, w, h) = decode_rgb(&output);
    assert_eq!((w, h), (64, 32));
}

#[test]
fn quality_changes_the_output_size() {
    let dir = TempDir::new();
    let mut pixels = Vec::new();
    let mut state = 12345u32;
    for _ in 0..256 * 256 * 3 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        pixels.push((state >> 24) as u8);
    }
    let input = dir.write(
        "a.jpg",
        &encode(MozColorSpace::JCS_RGB, 3, 256, 256, &pixels),
    );
    let size = |quality| {
        let s = Settings {
            long_edge: 1000,
            quality,
        };
        process_to_memory(&input, &s).unwrap().len()
    };
    assert!(size(30) < size(90));
}

#[test]
fn keeps_icc_profile_across_segments() {
    let dir = TempDir::new();
    let profile: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
    let (first, second) = profile.split_at(60_000);
    let chunk =
        |n: u8, data: &[u8]| segment(0xE2, &[b"ICC_PROFILE\0".as_slice(), &[n, 2], data].concat());
    let source = with_segments(
        &two_color_jpeg(64, 32),
        &[chunk(1, first), chunk(2, second)],
    );

    let output = convert(&dir, "a.jpg", &source);

    let mut decoder = JpegDecoder::new(ZCursor::new(output.as_slice()));
    decoder.decode_headers().unwrap();
    assert_eq!(decoder.icc_profile().unwrap(), profile);
}

#[test]
fn icc_chunks_are_numbered_from_1() {
    let dir = TempDir::new();
    let icc = segment(0xE2, b"ICC_PROFILE\0\x01\x01profile");
    let source = with_segments(&two_color_jpeg(64, 32), &[icc]);
    let output = convert(&dir, "a.jpg", &source);
    assert!(contains(&output, b"ICC_PROFILE\0\x01\x01profile"));
}

#[test]
fn broken_exif_is_dropped_with_a_warning() {
    let dir = TempDir::new();
    let mut tiff = tiff_with_gps(6);
    // IFD0 の位置を範囲外にして解析を失敗させる
    tiff[4..8].copy_from_slice(&0xFFFF_0000u32.to_le_bytes());
    let source = with_segments(&two_color_jpeg(64, 32), &[exif_segment(&tiff)]);
    let input = dir.write("a.jpg", &source);
    let output = dir.export().join("a.jpg");

    let report = process_file(&input, &output, &SETTINGS).unwrap();

    assert_eq!(report.warnings, [Warning::ExifRemoved]);
    let data = fs::read(output).unwrap();
    assert!(exif_payload(&data).is_none());
    assert!(!contains(&data, b"SENTINEL_GPS_ID"));
}

#[test]
fn grayscale_stays_grayscale() {
    let dir = TempDir::new();
    let pixels = vec![128u8; 64 * 32];
    let source = encode(MozColorSpace::JCS_GRAYSCALE, 1, 64, 32, &pixels);
    let output = convert(&dir, "a.jpg", &source);

    let mut decoder = JpegDecoder::new(ZCursor::new(output.as_slice()));
    decoder.decode_headers().unwrap();
    assert_eq!(decoder.input_colorspace(), Some(ColorSpace::Luma));
}

#[test]
fn cmyk_is_rejected() {
    let dir = TempDir::new();
    let pixels = vec![100u8; 16 * 16 * 4];
    let input = dir.write(
        "a.jpg",
        &encode(MozColorSpace::JCS_CMYK, 4, 16, 16, &pixels),
    );
    let result = process_file(&input, &dir.export().join("a.jpg"), &SETTINGS);
    assert!(matches!(
        result,
        Err(ProcessError::UnsupportedColorSpace(_))
    ));
}

#[test]
fn invalid_settings_are_rejected() {
    let dir = TempDir::new();
    let input = dir.write("a.jpg", &two_color_jpeg(64, 32));
    let settings = Settings {
        long_edge: 100,
        quality: 0,
    };
    let result = process_file(&input, &dir.export().join("a.jpg"), &settings);
    assert!(matches!(
        result,
        Err(ProcessError::Settings(SettingsError::Quality(0)))
    ));
}

#[test]
fn memory_result_matches_the_written_file() {
    let dir = TempDir::new();
    let source = with_segments(&two_color_jpeg(64, 32), &[exif_segment(&tiff_with_gps(3))]);
    let written = convert(&dir, "a.jpg", &source);
    let memory = process_to_memory(&dir.0.join("a.jpg"), &SETTINGS).unwrap();
    assert_eq!(written, memory);
}

#[test]
fn overwrites_existing_output() {
    let dir = TempDir::new();
    fs::create_dir_all(dir.export()).unwrap();
    fs::write(dir.export().join("a.jpg"), b"old").unwrap();
    let output = convert(&dir, "a.jpg", &two_color_jpeg(64, 32));
    assert_ne!(output, b"old");
    assert_eq!(dir.export_names(), ["a.jpg"]);
}

fn run(
    dir: &TempDir,
    files: &[PathBuf],
    jobs: usize,
    cancel: &AtomicBool,
    on_started: impl Fn() + Sync,
) -> (BatchSummary, Vec<BatchEvent>) {
    let events = Mutex::new(Vec::new());
    let summary = run_batch(files, &dir.export(), &SETTINGS, jobs, cancel, |event| {
        if matches!(event, BatchEvent::Started { .. }) {
            on_started();
        }
        events.lock().unwrap().push(event);
    });
    (summary, events.into_inner().unwrap())
}

#[test]
fn batch_continues_after_corrupt_files() {
    let dir = TempDir::new();
    let good = two_color_jpeg(64, 32);
    let truncated = good[..good.len() / 2].to_vec();
    let files = vec![
        dir.write("1-good.jpg", &good),
        dir.write("2-truncated.jpg", &truncated),
        dir.write("3-garbage.jpg", b"this is not a jpeg"),
        dir.write("4-empty.jpg", b""),
        dir.write("5-good.jpg", &good),
    ];

    let (summary, events) = run(&dir, &files, 2, &AtomicBool::new(false), || {});

    assert_eq!(summary.succeeded, 2);
    assert_eq!(summary.failed, 3);
    assert_eq!(dir.export_names(), ["1-good.jpg", "5-good.jpg"]);

    let mut failed: Vec<usize> = events
        .iter()
        .filter_map(|e| match e {
            BatchEvent::Failed { index, .. } => Some(*index),
            _ => None,
        })
        .collect();
    failed.sort();
    assert_eq!(failed, [1, 2, 3]);
}

#[test]
fn batch_reports_totals_and_keeps_originals() {
    let dir = TempDir::new();
    let source = with_segments(&two_color_jpeg(64, 32), &[exif_segment(&tiff_with_gps(6))]);
    let files: Vec<PathBuf> = (0..6)
        .map(|i| dir.write(&format!("{i}.jpg"), &source))
        .collect();

    let (summary, _) = run(&dir, &files, 3, &AtomicBool::new(false), || {});

    assert_eq!(summary.succeeded, 6);
    assert_eq!(summary.input_bytes, source.len() as u64 * 6);
    let written: u64 = dir
        .export_names()
        .iter()
        .map(|n| fs::metadata(dir.export().join(n)).unwrap().len())
        .sum();
    assert_eq!(summary.output_bytes, written);
    for file in &files {
        assert_eq!(fs::read(file).unwrap(), source);
    }
}

#[test]
fn cancel_before_start_processes_nothing() {
    let dir = TempDir::new();
    let files = vec![dir.write("a.jpg", &two_color_jpeg(64, 32))];
    let (summary, events) = run(&dir, &files, 2, &AtomicBool::new(true), || {});

    assert_eq!(summary.skipped, 1);
    assert_eq!(summary.succeeded, 0);
    assert!(dir.export_names().is_empty());
    assert!(matches!(events.as_slice(), [BatchEvent::Cancelled]));
}

#[test]
fn cancel_finishes_running_files_and_leaves_no_partial_output() {
    let dir = TempDir::new();
    let source = two_color_jpeg(64, 32);
    let files: Vec<PathBuf> = (0..8)
        .map(|i| dir.write(&format!("{i}.jpg"), &source))
        .collect();
    let cancel = AtomicBool::new(false);

    let (summary, events) = run(&dir, &files, 1, &cancel, || {
        cancel.store(true, Ordering::SeqCst);
    });

    assert_eq!(summary.succeeded, 1);
    assert_eq!(summary.skipped, 7);
    assert_eq!(dir.export_names().len(), 1);
    assert!(!dir.export_names()[0].ends_with(".tmp"));
    assert!(matches!(events.last(), Some(BatchEvent::Cancelled)));
}

#[test]
fn preset_settings_work_end_to_end() {
    let dir = TempDir::new();
    let input = dir.write("a.jpg", &two_color_jpeg(2000, 1000));
    let output: &Path = &dir.export().join("a.jpg");
    process_file(&input, output, &Preset::Low.settings()).unwrap();
    let (_, w, h) = decode_rgb(&fs::read(output).unwrap());
    assert_eq!((w, h), (1600, 800));
}
