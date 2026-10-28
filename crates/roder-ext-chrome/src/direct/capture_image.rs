//! Convert native screenshot pixels to input coordinates without resizing Chrome.
use std::io::Cursor;

use anyhow::Context;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::{ImageFormat, ImageReader, imageops::FilterType};

/// Enough to read a page, a fraction of a PNG's size.
const JPEG_QUALITY: u8 = 70;

/// Convert a base64 PNG of the native viewport to a base64 JPEG with the
/// supplied CSS dimensions. Call on a blocking worker, after capture and any
/// page-mask cleanup. Shared by direct CDP tools and Jev's own screenshot path.
pub fn viewport_jpeg(data: &str, width: u32, height: u32) -> anyhow::Result<String> {
    anyhow::ensure!(width > 0 && height > 0, "Screenshot viewport is empty");
    let bytes = STANDARD.decode(data).context("Invalid screenshot base64")?;
    let image = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png)
        .decode()
        .context("Could not decode Chrome screenshot")?;
    // One image pixel per CSS pixel, including Retina displays and page zoom.
    // Resize from actual image dimensions: DPR alone need not describe the
    // captured surface's scale when device emulation or browser zoom is active.
    let image = if image.width() == width && image.height() == height {
        image
    } else {
        image.resize_exact(width, height, FilterType::Triangle)
    };
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY)
        .encode_image(&image.to_rgb8())
        .context("Could not encode viewport screenshot")?;
    Ok(STANDARD.encode(jpeg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_pixels_become_css_pixels_and_keep_masked_regions() {
        for scale in [1, 2, 3] {
            let source = image::RgbImage::from_fn(80 * scale, 60 * scale, |x, _| {
                image::Rgb(if x < 40 * scale {
                    [0, 0, 0]
                } else {
                    [255, 255, 255]
                })
            });
            let mut png = Cursor::new(Vec::new());
            source.write_to(&mut png, ImageFormat::Png).unwrap();
            let encoded = viewport_jpeg(&STANDARD.encode(png.into_inner()), 80, 60).unwrap();
            let bytes = STANDARD.decode(encoded).unwrap();
            let jpeg = image::load_from_memory_with_format(&bytes, ImageFormat::Jpeg)
                .unwrap()
                .to_rgb8();
            assert_eq!(jpeg.dimensions(), (80, 60));
            assert!(jpeg.get_pixel(20, 30).0.iter().all(|channel| *channel < 5));
            assert!(
                jpeg.get_pixel(60, 30)
                    .0
                    .iter()
                    .all(|channel| *channel > 250)
            );
        }
    }

    #[test]
    fn invalid_capture_data_is_reported() {
        assert!(viewport_jpeg("not base64!", 80, 60).is_err());
        assert!(viewport_jpeg(&STANDARD.encode(b"not a PNG"), 80, 60).is_err());
        assert!(viewport_jpeg("", 0, 60).is_err());
    }
}
