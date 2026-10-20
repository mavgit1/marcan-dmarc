//! An image's size in pixels, read from its header, and the size an email
//! shows it at.
//!
//! Desktop Outlook draws HTML with Word's engine, which ignores CSS sizes on
//! images and uses the image's own pixels unless the `img` tag carries
//! `width` and `height` attributes. So every image in an email gets both,
//! computed here from its pixel size. Only the formats a logo may have are
//! read (PNG, JPEG, WebP; [`dmarc_store::LogoType`]), and only as far as the
//! header that states the size: no image library.

use dmarc_store::PixelSize;

/// The size of the PNG, JPEG or WebP image in `bytes`; `None` for another
/// format or a header that does not state one.
pub fn pixel_size(bytes: &[u8]) -> Option<PixelSize> {
    let size = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        png(bytes)
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        jpeg(bytes)
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        webp(bytes)
    } else {
        None
    }?;
    (size.width > 0 && size.height > 0).then_some(size)
}

fn be16(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from(u16::from_be_bytes(
        b.get(at..at + 2)?.try_into().ok()?,
    )))
}

fn le16(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from(u16::from_le_bytes(
        b.get(at..at + 2)?.try_into().ok()?,
    )))
}

fn le24(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at + 3)?;
    Some(u32::from(s[0]) | u32::from(s[1]) << 8 | u32::from(s[2]) << 16)
}

/// The IHDR chunk, always first: width and height as big-endian u32.
fn png(b: &[u8]) -> Option<PixelSize> {
    if b.get(12..16)? != b"IHDR" {
        return None;
    }
    let u32_at = |at: usize| Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?));
    Some(PixelSize {
        width: u32_at(16)?,
        height: u32_at(20)?,
    })
}

/// The first start-of-frame segment (SOF0 to SOF15, except DHT, JPG and
/// DAC, which share the range): height, then width, big-endian u16.
fn jpeg(b: &[u8]) -> Option<PixelSize> {
    let mut at = 2;
    loop {
        // Fill bytes (0xFF) may precede a marker.
        while *b.get(at)? == 0xFF && *b.get(at + 1)? == 0xFF {
            at += 1;
        }
        if *b.get(at)? != 0xFF {
            return None;
        }
        let marker = *b.get(at + 1)?;
        match marker {
            // Markers without a length: TEM, RST0..7.
            0x01 | 0xD0..=0xD7 => at += 2,
            // Start of scan or end of image before a frame: no size.
            0xDA | 0xD9 => return None,
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => {
                return Some(PixelSize {
                    height: be16(b, at + 5)?,
                    width: be16(b, at + 7)?,
                });
            }
            _ => at += 2 + usize::try_from(be16(b, at + 2)?).ok()?,
        }
    }
}

/// The first chunk: `VP8 ` (lossy), `VP8L` (lossless) or `VP8X` (extended).
fn webp(b: &[u8]) -> Option<PixelSize> {
    match b.get(12..16)? {
        // The key frame's start code, then 14-bit width and height.
        b"VP8 " => {
            if b.get(23..26)? != [0x9D, 0x01, 0x2A] {
                return None;
            }
            Some(PixelSize {
                width: le16(b, 26)? & 0x3FFF,
                height: le16(b, 28)? & 0x3FFF,
            })
        }
        // The signature byte 0x2F, then width - 1 and height - 1 in 14 bits each.
        b"VP8L" => {
            if *b.get(20)? != 0x2F {
                return None;
            }
            let bits = u32::from_le_bytes(b.get(21..25)?.try_into().ok()?);
            Some(PixelSize {
                width: (bits & 0x3FFF) + 1,
                height: ((bits >> 14) & 0x3FFF) + 1,
            })
        }
        // The canvas: width - 1 and height - 1 in 24 bits each.
        b"VP8X" => Some(PixelSize {
            width: le24(b, 24)? + 1,
            height: le24(b, 27)? + 1,
        }),
        _ => None,
    }
}

/// The size `natural` is shown at inside `max_width` by `max_height`,
/// keeping its aspect ratio; never larger than its own pixels and at least
/// 1 px each way.
pub fn fit(natural: PixelSize, max_width: u32, max_height: u32) -> PixelSize {
    let (w, h) = (f64::from(natural.width), f64::from(natural.height));
    let scale = (f64::from(max_width) / w)
        .min(f64::from(max_height) / h)
        .min(1.0);
    // In range: the scale is at most 1, so each side is at most its own u32.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let px = |v: f64| ((v * scale).round() as u32).max(1);
    PixelSize {
        width: px(w),
        height: px(h),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(width: u32, height: u32) -> PixelSize {
        PixelSize { width, height }
    }

    /// The signature, the IHDR chunk's length and type, then 640 by 160.
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\x02\x80\0\0\0\xa0\x08\x06\0\0\0";

    /// SOI, an APP0 segment of 4 bytes, a DQT-like segment, then SOF2
    /// (progressive) with 8-bit samples, height 90 and width 300.
    const JPEG: &[u8] = &[
        0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x4A, 0x46, 0xFF, 0xDB, 0x00, 0x03, 0x00, 0xFF, 0xFF,
        0xC2, 0x00, 0x11, 0x08, 0x00, 0x5A, 0x01, 0x2C, 0x03,
    ];

    /// A RIFF header, then a VP8X chunk with a canvas of 1000 by 250.
    const WEBP_EXTENDED: &[u8] = b"RIFF\x24\0\0\0WEBPVP8X\x0a\0\0\0\x10\0\0\0\xe7\x03\0\xf9\0\0";

    #[test]
    fn sizes_are_read_from_the_header() {
        assert_eq!(pixel_size(PNG), Some(size(640, 160)));
        assert_eq!(pixel_size(JPEG), Some(size(300, 90)));
        assert_eq!(pixel_size(WEBP_EXTENDED), Some(size(1000, 250)));
        // Lossless: 0x2F, then (w-1) | (h-1) << 14 = 199 | 49 << 14.
        let bits: u32 = 199 | 49 << 14;
        let mut lossless = b"RIFF\x1a\0\0\0WEBPVP8L\x0d\0\0\0\x2f".to_vec();
        lossless.extend_from_slice(&bits.to_le_bytes());
        assert_eq!(pixel_size(&lossless), Some(size(200, 50)));
        // Lossy: a frame tag, the start code, then 120 by 30.
        let mut lossy = b"RIFF\x1a\0\0\0WEBPVP8 \x0d\0\0\0\0\0\0\x9d\x01\x2a".to_vec();
        lossy.extend_from_slice(&[120, 0, 30, 0]);
        assert_eq!(pixel_size(&lossy), Some(size(120, 30)));
    }

    #[test]
    fn a_header_without_a_size_has_none() {
        assert_eq!(pixel_size(b""), None);
        assert_eq!(pixel_size(b"GIF89a\x10\0\x10\0"), None, "not a logo format");
        assert_eq!(pixel_size(&PNG[..20]), None, "cut short");
        // A JPEG whose scan starts before any frame.
        assert_eq!(pixel_size(&[0xFF, 0xD8, 0xFF, 0xDA, 0, 2]), None);
        assert_eq!(pixel_size(&JPEG[..20]), None, "cut short");
        // A zero-sized PNG.
        let mut zero = PNG.to_vec();
        zero[16..20].copy_from_slice(&[0, 0, 0, 0]);
        assert_eq!(pixel_size(&zero), None);
    }

    #[test]
    fn a_size_fits_its_box_keeping_its_shape() {
        let px = |width, height| PixelSize { width, height };
        // Wide: the width cap decides.
        assert_eq!(fit(px(1200, 300), 200, 64), px(200, 50));
        // Tall: the height cap decides.
        assert_eq!(fit(px(400, 400), 200, 64), px(64, 64));
        assert_eq!(fit(px(1200, 300), 120, 32), px(120, 30));
        // Small images are not enlarged.
        assert_eq!(fit(px(80, 20), 200, 64), px(80, 20));
        // Extreme shapes keep a pixel.
        assert_eq!(fit(px(10_000, 1), 200, 64), px(200, 1));
    }
}
