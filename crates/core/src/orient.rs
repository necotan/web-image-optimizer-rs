/// EXIF Orientation に従って画素を並べ替え、並べ替え後の幅と高さとともに返す
pub(crate) fn apply(
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    channels: usize,
    orientation: u8,
) -> (Vec<u8>, u32, u32) {
    if !(2..=8).contains(&orientation) {
        return (pixels, width, height);
    }
    let (w, h) = (width as usize, height as usize);
    let (out_w, out_h) = if orientation >= 5 { (h, w) } else { (w, h) };
    let mut out = vec![0u8; pixels.len()];

    for y in 0..h {
        for x in 0..w {
            let (dst_x, dst_y) = match orientation {
                2 => (w - 1 - x, y),
                3 => (w - 1 - x, h - 1 - y),
                4 => (x, h - 1 - y),
                5 => (y, x),
                6 => (h - 1 - y, x),
                7 => (h - 1 - y, w - 1 - x),
                _ => (y, w - 1 - x),
            };
            let src = (y * w + x) * channels;
            let dst = (dst_y * out_w + dst_x) * channels;
            out[dst..dst + channels].copy_from_slice(&pixels[src..src + channels]);
        }
    }
    (out, out_w as u32, out_h as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 3x2 の画像を 1 チャンネルで表す
    //   1 2 3
    //   4 5 6
    const SOURCE: [u8; 6] = [1, 2, 3, 4, 5, 6];

    fn check(orientation: u8, width: u32, height: u32, expected: [u8; 6]) {
        let (out, w, h) = apply(SOURCE.to_vec(), 3, 2, 1, orientation);
        assert_eq!((w, h), (width, height), "orientation {orientation}");
        assert_eq!(out, expected, "orientation {orientation}");
    }

    #[test]
    fn orientation_1_keeps_pixels() {
        check(1, 3, 2, [1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn orientation_2_flips_horizontally() {
        check(2, 3, 2, [3, 2, 1, 6, 5, 4]);
    }

    #[test]
    fn orientation_3_rotates_180() {
        check(3, 3, 2, [6, 5, 4, 3, 2, 1]);
    }

    #[test]
    fn orientation_4_flips_vertically() {
        check(4, 3, 2, [4, 5, 6, 1, 2, 3]);
    }

    #[test]
    fn orientation_5_transposes() {
        check(5, 2, 3, [1, 4, 2, 5, 3, 6]);
    }

    #[test]
    fn orientation_6_rotates_90_clockwise() {
        check(6, 2, 3, [4, 1, 5, 2, 6, 3]);
    }

    #[test]
    fn orientation_7_transverses() {
        check(7, 2, 3, [6, 3, 5, 2, 4, 1]);
    }

    #[test]
    fn orientation_8_rotates_90_counterclockwise() {
        check(8, 2, 3, [3, 6, 2, 5, 1, 4]);
    }

    #[test]
    fn invalid_values_keep_pixels() {
        for orientation in [0, 9, 255] {
            check(orientation, 3, 2, [1, 2, 3, 4, 5, 6]);
        }
    }

    #[test]
    fn moves_whole_pixels() {
        let rgb = vec![1, 2, 3, 4, 5, 6];
        let (out, w, h) = apply(rgb, 2, 1, 3, 2);
        assert_eq!((w, h), (2, 1));
        assert_eq!(out, [4, 5, 6, 1, 2, 3]);
    }
}
