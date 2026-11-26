//! Writes the Xcursor format, as `xcursorgen` does: a header, a table of contents, then one
//! image chunk per image. Every number is a 32-bit little-endian word; a pixel is a word of
//! premultiplied ARGB.

/// Image chunk type.
const IMAGE: u32 = 0xFFFD_0002;
/// Size of the file header and of an image chunk header, in bytes.
const FILE_HEADER: u32 = 16;
const IMAGE_HEADER: u32 = 36;

/// One image of a cursor file.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    /// Nominal size: the size a client asks for.
    pub size: u32,
    pub width: u32,
    pub height: u32,
    pub hotspot: (u32, u32),
    /// Time it is shown, in milliseconds, for an animated cursor. 0 for a still one.
    pub delay: u32,
    /// `width × height` premultiplied ARGB words, row by row.
    pub pixels: Vec<u32>,
}

/// The file of a cursor: every image, in order. The images of an animation share a nominal
/// size and follow each other.
pub fn encode(images: &[Image]) -> Vec<u8> {
    let mut out = Vec::new();
    let word = |out: &mut Vec<u8>, w: u32| out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(b"Xcur");
    word(&mut out, FILE_HEADER);
    word(&mut out, 0x0001_0000);
    word(&mut out, images.len() as u32);
    let mut position = FILE_HEADER + 12 * images.len() as u32;
    for image in images {
        word(&mut out, IMAGE);
        word(&mut out, image.size);
        word(&mut out, position);
        position += IMAGE_HEADER + 4 * image.width * image.height;
    }
    for image in images {
        for w in [
            IMAGE_HEADER,
            IMAGE,
            image.size,
            1,
            image.width,
            image.height,
            image.hotspot.0,
            image.hotspot.1,
            image.delay,
        ] {
            word(&mut out, w);
        }
        for &pixel in &image.pixels {
            word(&mut out, pixel);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_parser_reads_back_what_is_written() {
        let still = Image {
            size: 24,
            width: 2,
            height: 1,
            hotspot: (1, 0),
            delay: 0,
            pixels: vec![0xFF14_233B, 0x8000_0000],
        };
        let large = Image {
            size: 48,
            width: 3,
            height: 2,
            hotspot: (2, 1),
            delay: 33,
            pixels: vec![0xFFF4_EBE3; 6],
        };
        let bytes = encode(&[still.clone(), large]);
        let images = xcursor::parser::parse_xcursor(&bytes).unwrap();
        assert_eq!(images.len(), 2);
        assert_eq!(
            (images[0].size, images[0].width, images[0].xhot),
            (24, 2, 1)
        );
        assert_eq!(
            (images[1].size, images[1].delay, images[1].yhot),
            (48, 33, 1)
        );
        // The bytes of a pixel, in the file's order: B, G, R, A.
        assert_eq!(&images[0].pixels_rgba[..4], &[0x3B, 0x23, 0x14, 0xFF]);
    }
}
