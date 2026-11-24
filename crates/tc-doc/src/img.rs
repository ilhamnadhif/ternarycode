//! Images: PNG, JPEG, GIF (first frame), WebP, BMP, TIFF and ICO, decoded to RGBA.

use crate::Rgba;

/// Longest side kept: bigger images are scaled down (a texture that large is rarely needed,
/// and the GPU may refuse it).
pub const MAX_SIDE: u32 = 4096;
/// Maximum raw pixel count allowed before decoding (100 MP ≈ 400 MB RGBA) to guard against
/// decompression bombs (e.g. tiny PNG claiming 65535×65535).
const MAX_PIXELS: u64 = 100_000_000;

/// The size of an encoded image (read from its header only); None when it is not an image
/// this can decode (EMF and WMF drawings, for instance).
pub fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
        .filter(|(w, h)| *w > 0 && *h > 0)
}

/// Decodes an image, scaled down to at most `max_side` pixels on its longest side.
pub fn decode(bytes: &[u8], max_side: u32) -> Result<Rgba, String> {
    if let Some((w, h)) = dimensions(bytes)
        && u64::from(w).saturating_mul(u64::from(h)) > MAX_PIXELS
    {
        return Err(format!("the image is too big to decode ({w} × {h} px)"));
    }
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let img = if img.width().max(img.height()) > max_side {
        img.resize(max_side, max_side, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let rgba = img.into_rgba8();
    if rgba.width() == 0 || rgba.height() == 0 {
        return Err("the image is empty".into());
    }
    Ok(Rgba {
        width: rgba.width(),
        height: rgba.height(),
        pixels: rgba.into_raw(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A 2×1 PNG: one red, one transparent pixel.
    pub(crate) fn tiny_png() -> Vec<u8> {
        let mut out = Vec::new();
        let img = image::RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 0, 0, 0]).unwrap();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    #[test]
    fn images_decode_and_big_ones_shrink() {
        assert_eq!(dimensions(&tiny_png()), Some((2, 1)));
        assert_eq!(dimensions(b"nope"), None);
        let rgba = decode(&tiny_png(), MAX_SIDE).unwrap();
        assert_eq!((rgba.width, rgba.height), (2, 1));
        assert_eq!(&rgba.pixels[..4], &[255, 0, 0, 255]);
        let small = decode(&tiny_png(), 1).unwrap();
        assert_eq!((small.width, small.height), (1, 1));
        assert!(decode(b"not an image", MAX_SIDE).is_err());
    }
}
