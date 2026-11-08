use img_parts::jpeg::Jpeg;
use img_parts::{Bytes, ImageEXIF, ImageICC};
use thiserror::Error;

const TAG_ORIENTATION: u16 = 0x0112;
const TAG_EXIF_IFD: u16 = 0x8769;
const TAG_INTEROP_IFD: u16 = 0xA005;
const TAG_PIXEL_X: u16 = 0xA002;
const TAG_PIXEL_Y: u16 = 0xA003;

const TYPE_SHORT: u16 = 3;
const TYPE_LONG: u16 = 4;

/// 出力しないタグ
///
/// 位置情報を含みうるもの（GPS、MakerNote、XMP、IPTC、Photoshop）に加えて、EXIF を組み直すと指す先がずれるオフセット系のタグも落とす
const DROPPED_TAGS: [u16; 15] = [
    0x0111, // StripOffsets
    0x0117, // StripByteCounts
    0x0120, // FreeOffsets
    0x0121, // FreeByteCounts
    0x0144, // TileOffsets
    0x0145, // TileByteCounts
    0x014A, // SubIFDs
    0x0201, // ThumbnailOffset
    0x0202, // ThumbnailLength
    0x02BC, // XMP
    0x83BB, // IPTC
    0x8649, // Photoshop
    0x8825, // GPS IFD
    0x927C, // MakerNote
    0xC634, // DNGPrivateData
];

/// APP1 の最大ペイロードから `Exif\0\0` の 6 バイトを引いた値
const MAX_TIFF_LEN: usize = 65533 - 6;

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum ExifError {
    #[error("invalid TIFF header")]
    Header,
    #[error("IFD is out of bounds")]
    IfdBounds,
    #[error("tag 0x{0:04x} points out of bounds")]
    ValueBounds(u16),
    #[error("rebuilt EXIF does not fit in APP1")]
    TooLarge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Endian {
    Little,
    Big,
}

impl Endian {
    fn read_u16(self, b: [u8; 2]) -> u16 {
        match self {
            Endian::Little => u16::from_le_bytes(b),
            Endian::Big => u16::from_be_bytes(b),
        }
    }

    fn read_u32(self, b: [u8; 4]) -> u32 {
        match self {
            Endian::Little => u32::from_le_bytes(b),
            Endian::Big => u32::from_be_bytes(b),
        }
    }

    fn u16_bytes(self, v: u16) -> [u8; 2] {
        match self {
            Endian::Little => v.to_le_bytes(),
            Endian::Big => v.to_be_bytes(),
        }
    }

    fn u32_bytes(self, v: u32) -> [u8; 4] {
        match self {
            Endian::Little => v.to_le_bytes(),
            Endian::Big => v.to_be_bytes(),
        }
    }
}

#[derive(Debug, Clone)]
struct Entry {
    tag: u16,
    field_type: u16,
    count: u32,
    value: Vec<u8>,
}

impl Entry {
    fn long(tag: u16, value: u32, endian: Endian) -> Entry {
        Entry {
            tag,
            field_type: TYPE_LONG,
            count: 1,
            value: endian.u32_bytes(value).to_vec(),
        }
    }
}

/// 残すタグだけを持つ EXIF
///
/// 元の TIFF のバイト列は保持しない
// ここに読み込まなかったデータは、どの IFD からも参照されていない領域を含めて出力に残らない
#[derive(Debug)]
pub(crate) struct Exif {
    endian: Endian,
    ifd0: Vec<Entry>,
    exif_ifd: Vec<Entry>,
    interop_ifd: Vec<Entry>,
}

struct Reader<'a> {
    data: &'a [u8],
    endian: Endian,
}

impl Reader<'_> {
    fn bytes(&self, offset: usize, len: usize) -> Option<&[u8]> {
        self.data.get(offset..offset.checked_add(len)?)
    }

    fn u16_at(&self, offset: usize) -> Option<u16> {
        let b = self.bytes(offset, 2)?;
        Some(self.endian.read_u16([b[0], b[1]]))
    }

    fn u32_at(&self, offset: usize) -> Option<u32> {
        let b = self.bytes(offset, 4)?;
        Some(self.endian.read_u32([b[0], b[1], b[2], b[3]]))
    }

    /// IFD を読む
    // `next` のリンクは読まないので IFD1 には進まず、ポインタの循環も起きない
    fn read_ifd(
        &self,
        offset: usize,
        pointers: &mut Vec<(u16, u32)>,
    ) -> Result<Vec<Entry>, ExifError> {
        let count = self.u16_at(offset).ok_or(ExifError::IfdBounds)? as usize;
        let mut entries = Vec::with_capacity(count);

        for i in 0..count {
            let at = offset + 2 + i * 12;
            let raw = self.bytes(at, 12).ok_or(ExifError::IfdBounds)?;
            let tag = self.endian.read_u16([raw[0], raw[1]]);
            let field_type = self.endian.read_u16([raw[2], raw[3]]);
            let value_count = self.endian.read_u32([raw[4], raw[5], raw[6], raw[7]]);
            let inline = &raw[8..12];

            if tag == TAG_EXIF_IFD || tag == TAG_INTEROP_IFD {
                pointers.push((
                    tag,
                    self.endian
                        .read_u32([inline[0], inline[1], inline[2], inline[3]]),
                ));
                continue;
            }
            if DROPPED_TAGS.contains(&tag) {
                continue;
            }
            // 未知の型はサイズが分からず値を取り出せない
            let Some(unit) = type_size(field_type) else {
                continue;
            };

            let size = (value_count as usize)
                .checked_mul(unit)
                .ok_or(ExifError::ValueBounds(tag))?;
            let value = if size <= 4 {
                inline[..size].to_vec()
            } else {
                let value_offset = self
                    .endian
                    .read_u32([inline[0], inline[1], inline[2], inline[3]]);
                self.bytes(value_offset as usize, size)
                    .ok_or(ExifError::ValueBounds(tag))?
                    .to_vec()
            };
            entries.push(Entry {
                tag,
                field_type,
                count: value_count,
                value,
            });
        }
        Ok(entries)
    }
}

fn type_size(field_type: u16) -> Option<usize> {
    match field_type {
        1 | 2 | 6 | 7 => Some(1),
        3 | 8 => Some(2),
        4 | 9 | 11 => Some(4),
        5 | 10 | 12 => Some(8),
        _ => None,
    }
}

impl Exif {
    pub(crate) fn parse(tiff: &[u8]) -> Result<Exif, ExifError> {
        let endian = match tiff.get(0..2) {
            Some(b"II") => Endian::Little,
            Some(b"MM") => Endian::Big,
            _ => return Err(ExifError::Header),
        };
        let reader = Reader { data: tiff, endian };
        if reader.u16_at(2) != Some(42) {
            return Err(ExifError::Header);
        }
        let ifd0_offset = reader.u32_at(4).ok_or(ExifError::Header)? as usize;

        let mut pointers = Vec::new();
        let ifd0 = reader.read_ifd(ifd0_offset, &mut pointers)?;

        let mut exif_ifd = Vec::new();
        let mut interop_ifd = Vec::new();
        let exif_pointer = pointers.iter().find(|(tag, _)| *tag == TAG_EXIF_IFD);
        if let Some((_, offset)) = exif_pointer {
            let mut sub_pointers = Vec::new();
            exif_ifd = reader.read_ifd(*offset as usize, &mut sub_pointers)?;
            let interop_pointer = sub_pointers.iter().find(|(tag, _)| *tag == TAG_INTEROP_IFD);
            if let Some((_, offset)) = interop_pointer {
                interop_ifd = reader.read_ifd(*offset as usize, &mut Vec::new())?;
            }
        }

        Ok(Exif {
            endian,
            ifd0,
            exif_ifd,
            interop_ifd,
        })
    }

    /// 不正な値は補正しない扱い（1）にする
    pub(crate) fn orientation(&self) -> u8 {
        let Some(entry) = self.ifd0.iter().find(|e| e.tag == TAG_ORIENTATION) else {
            return 1;
        };
        let value = match (entry.field_type, entry.value.as_slice()) {
            (TYPE_SHORT, [a, b, ..]) => self.endian.read_u16([*a, *b]) as u32,
            (TYPE_LONG, [a, b, c, d, ..]) => self.endian.read_u32([*a, *b, *c, *d]),
            _ => 1,
        };
        if (1..=8).contains(&value) {
            value as u8
        } else {
            1
        }
    }

    /// 出力用の TIFF を組み立てる
    pub(crate) fn rebuild(&self, width: u32, height: u32) -> Result<Vec<u8>, ExifError> {
        let endian = self.endian;

        let mut ifd0 = self.ifd0.clone();
        for entry in ifd0.iter_mut().filter(|e| e.tag == TAG_ORIENTATION) {
            entry.field_type = TYPE_SHORT;
            entry.count = 1;
            entry.value = endian.u16_bytes(1).to_vec();
        }

        let mut exif_ifd = self.exif_ifd.clone();
        for entry in &mut exif_ifd {
            match entry.tag {
                TAG_PIXEL_X => *entry = Entry::long(TAG_PIXEL_X, width, endian),
                TAG_PIXEL_Y => *entry = Entry::long(TAG_PIXEL_Y, height, endian),
                _ => {}
            }
        }

        let interop_ifd = &self.interop_ifd;
        if !interop_ifd.is_empty() {
            exif_ifd.push(Entry::long(TAG_INTEROP_IFD, 0, endian));
        }
        if !exif_ifd.is_empty() {
            ifd0.push(Entry::long(TAG_EXIF_IFD, 0, endian));
        }

        let ifd0_offset = 8;
        let exif_offset = ifd0_offset + ifd_size(&ifd0);
        let interop_offset = exif_offset + ifd_size(&exif_ifd);
        set_pointer(&mut ifd0, TAG_EXIF_IFD, exif_offset, endian);
        set_pointer(&mut exif_ifd, TAG_INTEROP_IFD, interop_offset, endian);

        let mut out = Vec::new();
        out.extend_from_slice(match endian {
            Endian::Little => b"II",
            Endian::Big => b"MM",
        });
        out.extend_from_slice(&endian.u16_bytes(42));
        out.extend_from_slice(&endian.u32_bytes(ifd0_offset as u32));
        write_ifd(&mut out, ifd0, endian);
        if !exif_ifd.is_empty() {
            write_ifd(&mut out, exif_ifd, endian);
        }
        if !interop_ifd.is_empty() {
            write_ifd(&mut out, interop_ifd.clone(), endian);
        }

        if out.len() > MAX_TIFF_LEN {
            return Err(ExifError::TooLarge);
        }
        Ok(out)
    }
}

fn set_pointer(entries: &mut [Entry], tag: u16, offset: usize, endian: Endian) {
    for entry in entries.iter_mut().filter(|e| e.tag == tag) {
        entry.value = endian.u32_bytes(offset as u32).to_vec();
    }
}

fn padded(len: usize) -> usize {
    len + len % 2
}

fn ifd_size(entries: &[Entry]) -> usize {
    if entries.is_empty() {
        return 0;
    }
    let data: usize = entries
        .iter()
        .filter(|e| e.value.len() > 4)
        .map(|e| padded(e.value.len()))
        .sum();
    2 + entries.len() * 12 + 4 + data
}

fn write_ifd(out: &mut Vec<u8>, mut entries: Vec<Entry>, endian: Endian) {
    entries.sort_by_key(|e| e.tag);
    let start = out.len();
    let mut data_offset = start + 2 + entries.len() * 12 + 4;

    out.extend_from_slice(&endian.u16_bytes(entries.len() as u16));
    for entry in &entries {
        out.extend_from_slice(&endian.u16_bytes(entry.tag));
        out.extend_from_slice(&endian.u16_bytes(entry.field_type));
        out.extend_from_slice(&endian.u32_bytes(entry.count));
        if entry.value.len() <= 4 {
            let mut inline = [0u8; 4];
            inline[..entry.value.len()].copy_from_slice(&entry.value);
            out.extend_from_slice(&inline);
        } else {
            out.extend_from_slice(&endian.u32_bytes(data_offset as u32));
            data_offset += padded(entry.value.len());
        }
    }
    out.extend_from_slice(&[0; 4]);

    for entry in entries.iter().filter(|e| e.value.len() > 4) {
        out.extend_from_slice(&entry.value);
        if entry.value.len() % 2 == 1 {
            out.push(0);
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct Metadata {
    pub(crate) exif: Option<Exif>,
    pub(crate) icc: Option<Bytes>,
    pub(crate) exif_failed: bool,
    pub(crate) unreadable: bool,
}

/// 入力の JPEG から EXIF と ICC を取り出す
///
/// XMP や IPTC などのセグメントは読まない
// 出力は新しくエンコードするので、ここで返したもの以外は出力に含まれない
pub(crate) fn extract(input: Bytes) -> Metadata {
    let Ok(jpeg) = Jpeg::from_bytes(input) else {
        return Metadata {
            unreadable: true,
            ..Metadata::default()
        };
    };

    let mut metadata = Metadata {
        icc: jpeg.icc_profile().filter(|p| !p.is_empty()),
        ..Metadata::default()
    };
    if let Some(tiff) = jpeg.exif() {
        match Exif::parse(&tiff) {
            Ok(exif) => metadata.exif = Some(exif),
            Err(_) => metadata.exif_failed = true,
        }
    }
    metadata
}

#[cfg(test)]
mod tests {
    use super::*;

    const THUMBNAIL: &[u8] = b"SENTINEL_THUMBNAIL_BYTES";
    const GPS_TEXT: &[u8] = b"SENTINEL_GPS_METHOD";
    const MAKER_NOTE: &[u8] = b"SENTINEL_MAKER_NOTE_CITY";
    const XMP: &[u8] = b"<rdf>SENTINEL_XMP_LOCATION</rdf>";
    const IPTC: &[u8] = b"SENTINEL_IPTC_CITY";
    const ORPHAN: &[u8] = b"SENTINEL_ORPHAN_DATA";
    const SENTINELS: [&[u8]; 6] = [THUMBNAIL, GPS_TEXT, MAKER_NOTE, XMP, IPTC, ORPHAN];

    struct Tiff {
        endian: Endian,
        out: Vec<u8>,
    }

    type RawEntry = (u16, u16, u32, Vec<u8>);

    impl Tiff {
        fn u16(&self, v: u16) -> Vec<u8> {
            self.endian.u16_bytes(v).to_vec()
        }
        fn u32(&self, v: u32) -> Vec<u8> {
            self.endian.u32_bytes(v).to_vec()
        }
        fn rational(&self, a: u32, b: u32) -> Vec<u8> {
            [self.u32(a), self.u32(b)].concat()
        }

        /// IFD を末尾に追記し、その開始位置を返す
        fn ifd(&mut self, entries: &[RawEntry], next: u32) -> u32 {
            let start = self.out.len();
            let mut data_at = start + 2 + entries.len() * 12 + 4;
            let mut head = self.u16(entries.len() as u16);
            let mut tail = Vec::new();
            for (tag, field_type, count, value) in entries {
                head.extend(self.u16(*tag));
                head.extend(self.u16(*field_type));
                head.extend(self.u32(*count));
                if value.len() <= 4 {
                    let mut inline = value.clone();
                    inline.resize(4, 0);
                    head.extend(inline);
                } else {
                    head.extend(self.u32(data_at as u32));
                    data_at += value.len();
                    tail.extend_from_slice(value);
                }
            }
            head.extend(self.u32(next));
            self.out.extend(head);
            self.out.extend(tail);
            start as u32
        }
    }

    /// GPS、IFD1、MakerNote、XMP、IPTC と、どこからも参照されない領域を持つ TIFF
    fn sample_tiff(endian: Endian) -> Vec<u8> {
        let mut t = Tiff {
            endian,
            out: Vec::new(),
        };
        t.out.extend_from_slice(match endian {
            Endian::Little => b"II",
            Endian::Big => b"MM",
        });
        let magic = t.u16(42);
        t.out.extend(magic);
        let first = t.u32(0);
        t.out.extend(first);
        t.out.extend_from_slice(ORPHAN);

        let gps = t.ifd(
            &[
                (0x0001, 2, 2, b"N\0".to_vec()),
                (0x0002, 5, 1, t.rational(34, 1)),
                (0x001B, 7, GPS_TEXT.len() as u32, GPS_TEXT.to_vec()),
            ],
            0,
        );
        let interop = t.ifd(&[(0x0001, 2, 4, b"R98\0".to_vec())], 0);
        let exif = t.ifd(
            &[
                (0x829A, 5, 1, t.rational(1, 250)),
                (0x8827, 3, 1, t.u16(400)),
                (0x9003, 2, 20, b"2026:08:04 12:34:56\0".to_vec()),
                (0x927C, 7, MAKER_NOTE.len() as u32, MAKER_NOTE.to_vec()),
                (TAG_PIXEL_X, 4, 1, t.u32(6000)),
                (TAG_PIXEL_Y, 3, 1, t.u16(4000)),
                (TAG_INTEROP_IFD, 4, 1, t.u32(interop)),
                (0xA434, 2, 7, b"Lens-Y\0".to_vec()),
            ],
            0,
        );

        let thumbnail_at = t.out.len() as u32;
        t.out.extend_from_slice(THUMBNAIL);
        let ifd1 = t.ifd(
            &[
                (0x0103, 3, 1, t.u16(6)),
                (0x0201, 4, 1, t.u32(thumbnail_at)),
                (0x0202, 4, 1, t.u32(THUMBNAIL.len() as u32)),
            ],
            0,
        );

        let ifd0 = t.ifd(
            &[
                (0x010F, 2, 5, b"ACME\0".to_vec()),
                (0x0110, 2, 8, b"Model-X\0".to_vec()),
                (TAG_ORIENTATION, 3, 1, t.u16(6)),
                (0x011A, 5, 1, t.rational(72, 1)),
                (0x02BC, 1, XMP.len() as u32, XMP.to_vec()),
                (0x83BB, 7, IPTC.len() as u32, IPTC.to_vec()),
                (TAG_EXIF_IFD, 4, 1, t.u32(exif)),
                (0x8825, 4, 1, t.u32(gps)),
            ],
            ifd1,
        );
        let pointer = t.u32(ifd0);
        t.out[4..8].copy_from_slice(&pointer);
        t.out
    }

    /// 本体の読み取り処理に頼らず、出力の構造を調べる
    struct Dump {
        endian: Endian,
        data: Vec<u8>,
    }

    impl Dump {
        fn new(data: Vec<u8>) -> Dump {
            let endian = if data.starts_with(b"II") {
                Endian::Little
            } else {
                Endian::Big
            };
            Dump { endian, data }
        }
        fn u16(&self, at: usize) -> u16 {
            self.endian
                .read_u16(self.data[at..at + 2].try_into().unwrap())
        }
        fn u32(&self, at: usize) -> u32 {
            self.endian
                .read_u32(self.data[at..at + 4].try_into().unwrap())
        }
        fn ifd0(&self) -> usize {
            self.u32(4) as usize
        }
        fn tags(&self, ifd: usize) -> Vec<u16> {
            (0..self.u16(ifd) as usize)
                .map(|i| self.u16(ifd + 2 + i * 12))
                .collect()
        }
        fn next(&self, ifd: usize) -> u32 {
            self.u32(ifd + 2 + self.u16(ifd) as usize * 12)
        }
        fn entry(&self, ifd: usize, tag: u16) -> Option<usize> {
            (0..self.u16(ifd) as usize)
                .map(|i| ifd + 2 + i * 12)
                .find(|at| self.u16(*at) == tag)
        }
        fn sub_ifd(&self, ifd: usize, tag: u16) -> usize {
            self.u32(self.entry(ifd, tag).unwrap() + 8) as usize
        }
        fn text(&self, ifd: usize, tag: u16) -> Vec<u8> {
            let at = self.entry(ifd, tag).unwrap();
            let len = self.u32(at + 4) as usize;
            let offset = self.u32(at + 8) as usize;
            self.data[offset..offset + len].to_vec()
        }
    }

    fn contains(data: &[u8], needle: &[u8]) -> bool {
        data.windows(needle.len()).any(|w| w == needle)
    }

    fn rebuilt(endian: Endian) -> Dump {
        let exif = Exif::parse(&sample_tiff(endian)).unwrap();
        Dump::new(exif.rebuild(2400, 1600).unwrap())
    }

    #[test]
    fn sample_contains_every_sentinel() {
        let tiff = sample_tiff(Endian::Little);
        for sentinel in SENTINELS {
            assert!(contains(&tiff, sentinel));
        }
    }

    #[test]
    fn removes_location_bytes_in_both_endians() {
        for endian in [Endian::Little, Endian::Big] {
            let dump = rebuilt(endian);
            for sentinel in SENTINELS {
                assert!(
                    !contains(&dump.data, sentinel),
                    "{} remains ({endian:?})",
                    String::from_utf8_lossy(sentinel)
                );
            }
        }
    }

    #[test]
    fn removes_gps_pointer_and_ifd1() {
        for endian in [Endian::Little, Endian::Big] {
            let dump = rebuilt(endian);
            let ifd0 = dump.ifd0();
            assert!(!dump.tags(ifd0).contains(&0x8825));
            assert_eq!(dump.next(ifd0), 0);
        }
    }

    #[test]
    fn removes_tags_that_can_carry_location_or_stale_offsets() {
        let dump = rebuilt(Endian::Little);
        let ifd0 = dump.ifd0();
        let exif = dump.sub_ifd(ifd0, TAG_EXIF_IFD);
        let tags = [dump.tags(ifd0), dump.tags(exif)].concat();
        for tag in DROPPED_TAGS {
            assert!(!tags.contains(&tag), "0x{tag:04x} remains");
        }
    }

    #[test]
    fn keeps_endianness() {
        assert!(rebuilt(Endian::Little).data.starts_with(b"II"));
        assert!(rebuilt(Endian::Big).data.starts_with(b"MM"));
    }

    #[test]
    fn keeps_other_values() {
        for endian in [Endian::Little, Endian::Big] {
            let dump = rebuilt(endian);
            let ifd0 = dump.ifd0();
            let exif = dump.sub_ifd(ifd0, TAG_EXIF_IFD);
            let interop = dump.sub_ifd(exif, TAG_INTEROP_IFD);
            assert_eq!(dump.text(ifd0, 0x010F), b"ACME\0");
            assert_eq!(dump.text(ifd0, 0x0110), b"Model-X\0");
            assert_eq!(dump.text(exif, 0x9003), b"2026:08:04 12:34:56\0");
            assert_eq!(dump.text(exif, 0xA434), b"Lens-Y\0");
            assert_eq!(dump.u16(dump.entry(exif, 0x8827).unwrap() + 8), 400);
            assert_eq!(dump.tags(interop), [0x0001]);
        }
    }

    #[test]
    fn tags_are_sorted() {
        let dump = rebuilt(Endian::Big);
        let ifd0 = dump.ifd0();
        for ifd in [ifd0, dump.sub_ifd(ifd0, TAG_EXIF_IFD)] {
            let tags = dump.tags(ifd);
            assert!(tags.windows(2).all(|w| w[0] < w[1]));
        }
    }

    #[test]
    fn rewrites_orientation_to_1() {
        for endian in [Endian::Little, Endian::Big] {
            let exif = Exif::parse(&sample_tiff(endian)).unwrap();
            assert_eq!(exif.orientation(), 6);

            let dump = Dump::new(exif.rebuild(10, 10).unwrap());
            let at = dump.entry(dump.ifd0(), TAG_ORIENTATION).unwrap();
            assert_eq!(dump.u16(at + 2), TYPE_SHORT);
            assert_eq!(dump.u16(at + 8), 1);

            let again = Exif::parse(&dump.data).unwrap();
            assert_eq!(again.orientation(), 1);
        }
    }

    #[test]
    fn rewrites_pixel_dimensions() {
        for endian in [Endian::Little, Endian::Big] {
            let dump = rebuilt(endian);
            let exif = dump.sub_ifd(dump.ifd0(), TAG_EXIF_IFD);
            assert_eq!(dump.u32(dump.entry(exif, TAG_PIXEL_X).unwrap() + 8), 2400);
            assert_eq!(dump.u32(dump.entry(exif, TAG_PIXEL_Y).unwrap() + 8), 1600);
        }
    }

    fn minimal_tiff(entries: &[RawEntry]) -> Vec<u8> {
        let mut t = Tiff {
            endian: Endian::Little,
            out: b"II\x2a\0\x08\0\0\0".to_vec(),
        };
        t.ifd(entries, 0);
        t.out
    }

    #[test]
    fn does_not_add_absent_tags() {
        let tiff = minimal_tiff(&[(0x010F, 2, 5, b"ACME\0".to_vec())]);
        let exif = Exif::parse(&tiff).unwrap();
        assert_eq!(exif.orientation(), 1);
        let dump = Dump::new(exif.rebuild(10, 10).unwrap());
        assert_eq!(dump.tags(dump.ifd0()), [0x010F]);
    }

    #[test]
    fn invalid_orientation_counts_as_upright() {
        for value in [0u16, 9, 300] {
            let tiff = minimal_tiff(&[(TAG_ORIENTATION, 3, 1, value.to_le_bytes().to_vec())]);
            assert_eq!(Exif::parse(&tiff).unwrap().orientation(), 1);
        }
    }

    #[test]
    fn drops_entries_of_unknown_type() {
        let tiff = minimal_tiff(&[
            (0x010F, 2, 5, b"ACME\0".to_vec()),
            (0xBEEF, 99, 1, vec![1, 2, 3, 4]),
        ]);
        let dump = Dump::new(Exif::parse(&tiff).unwrap().rebuild(10, 10).unwrap());
        assert_eq!(dump.tags(dump.ifd0()), [0x010F]);
    }

    #[test]
    fn rejects_broken_headers() {
        assert_eq!(Exif::parse(b"").unwrap_err(), ExifError::Header);
        assert_eq!(
            Exif::parse(b"XX\x2a\0\x08\0\0\0").unwrap_err(),
            ExifError::Header
        );
        assert_eq!(
            Exif::parse(b"II\x2b\0\x08\0\0\0").unwrap_err(),
            ExifError::Header
        );
    }

    #[test]
    fn rejects_values_out_of_bounds() {
        let mut tiff = minimal_tiff(&[(0x010F, 2, 100, vec![0; 100])]);
        tiff.truncate(40);
        assert_eq!(
            Exif::parse(&tiff).unwrap_err(),
            ExifError::ValueBounds(0x010F)
        );
    }

    #[test]
    fn rejects_ifd_out_of_bounds() {
        let tiff = b"II\x2a\0\xff\xff\0\0";
        assert_eq!(Exif::parse(tiff).unwrap_err(), ExifError::IfdBounds);
    }

    #[test]
    fn rejects_output_too_large_for_app1() {
        let tiff = minimal_tiff(&[(0x010E, 2, 70000, vec![b'x'; 70000])]);
        let exif = Exif::parse(&tiff).unwrap();
        assert_eq!(exif.rebuild(10, 10).unwrap_err(), ExifError::TooLarge);
    }

    #[test]
    fn pointer_cycles_end() {
        let mut next_to_self = sample_tiff(Endian::Little);
        let ifd0 = u32::from_le_bytes(next_to_self[4..8].try_into().unwrap()) as usize;
        let count = u16::from_le_bytes(next_to_self[ifd0..ifd0 + 2].try_into().unwrap()) as usize;
        let next_at = ifd0 + 2 + count * 12;
        next_to_self[next_at..next_at + 4].copy_from_slice(&(ifd0 as u32).to_le_bytes());
        assert!(Exif::parse(&next_to_self).is_ok());

        let mut exif_to_ifd0 = sample_tiff(Endian::Little);
        let dump = Dump::new(exif_to_ifd0.clone());
        let at = dump.entry(ifd0, TAG_EXIF_IFD).unwrap();
        exif_to_ifd0[at + 8..at + 12].copy_from_slice(&(ifd0 as u32).to_le_bytes());
        let exif = Exif::parse(&exif_to_ifd0).unwrap();
        for sentinel in SENTINELS {
            assert!(!contains(&exif.rebuild(10, 10).unwrap(), sentinel));
        }
    }

    #[test]
    fn corrupted_data_never_leaks_or_panics() {
        let base = sample_tiff(Endian::Little);
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..50_000 {
            let mut data = base.clone();
            for _ in 0..(1 + next() % 4) {
                let at = next() as usize % data.len();
                data[at] = next() as u8;
            }
            if let Ok(exif) = Exif::parse(&data)
                && let Ok(out) = exif.rebuild(10, 10)
            {
                assert!(Exif::parse(&out).is_ok());
            }
        }
    }
}
