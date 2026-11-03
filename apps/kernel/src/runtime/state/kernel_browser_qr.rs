//! Decodes the QR code a page shows (an authenticator enrolment code) from the
//! PNG the browser controller took of that element. The decoded text goes to
//! the Vault only; an error never carries the image or what it holds.
use super::kernel_browser_runtime::host_error;
use crate::error::DaemonError;
use zeroize::Zeroizing;

/// The controller captures one element; a few megabytes of PNG is already a huge QR code.
pub(super) const IMAGE_LIMIT: usize = 4 * 1024 * 1024;
const PIXEL_LIMIT: usize = 16 * 1024 * 1024;

fn unreadable() -> DaemonError {
    host_error(
        "the element shows no readable QR code; point node_ref at the QR image itself, or capture the manual setup key text instead. Nothing was stored".into(),
    )
}

pub(super) fn decode_png_qr(png: &[u8]) -> Result<Zeroizing<String>, DaemonError> {
    if png.len() > IMAGE_LIMIT {
        return Err(unreadable());
    }
    let mut decoder = png::Decoder::new(png);
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|_| unreadable())?;
    let (width, height) = (reader.info().width as usize, reader.info().height as usize);
    if width == 0 || height == 0 || width.saturating_mul(height) > PIXEL_LIMIT {
        return Err(unreadable());
    }
    let mut pixels = vec![0u8; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut pixels).map_err(|_| unreadable())?;
    let channels = frame.color_type.samples();
    let luma = |x: usize, y: usize| -> u8 {
        let at = (y * width + x) * channels;
        let (r, g, b, a) = match frame.color_type {
            png::ColorType::Grayscale => (pixels[at], pixels[at], pixels[at], 255),
            png::ColorType::GrayscaleAlpha => (pixels[at], pixels[at], pixels[at], pixels[at + 1]),
            png::ColorType::Rgba => (pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]),
            _ => (pixels[at], pixels[at + 1], pixels[at + 2], 255),
        };
        // Transparent areas are the page's white.
        let blend = |c: u8| (u32::from(c) * u32::from(a) + 255 * (255 - u32::from(a))) / 255;
        ((blend(r) * 299 + blend(g) * 587 + blend(b) * 114) / 1000) as u8
    };
    // A tight crop leaves no quiet zone, which the finder patterns need.
    let margin = (width.max(height) / 8).max(16);
    let (padded_width, padded_height) = (width + 2 * margin, height + 2 * margin);
    for inverted in [false, true] {
        let mut image =
            rqrr::PreparedImage::prepare_from_greyscale(padded_width, padded_height, |x, y| {
                let inside =
                    x >= margin && y >= margin && x < margin + width && y < margin + height;
                if !inside {
                    return 255;
                }
                let value = luma(x - margin, y - margin);
                if inverted {
                    255 - value
                } else {
                    value
                }
            });
        for grid in image.detect_grids() {
            if let Ok((_, text)) = grid.decode() {
                return Ok(Zeroizing::new(text));
            }
        }
    }
    Err(unreadable())
}

/// Fictional enrolment code, drawn with no quiet zone: otpauth://totp/Fixture:alice?secret=JBSWY3DPEHPK3PXP&issuer=Fixture&digits=8&period=45
#[cfg(test)]
pub(super) const FIXTURE_QR_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAHsAAAB7AQAAAAB8vMLSAAAB00lEQVR42o1Vvc3kIBR8PgIyuwEk2iCjJbsB79KA3RIZbSDRgJ0RIL8bf192l8wK7UorjR7z8wYRsXpm/8SxxiEif+Sfz39/iNfitRvVuhX3UQbipoJBVaxJOlbLQpbs9T08RG91q9VvICHgUpc+PkWvODguIOJA/PdwiqmqT93jJym+K3cxDGpa2tfWXRhIu/uQWOdQZzHfSE25BKa4PfpTFYMIiEl9LNkt6rYOnamLXbbdpT3RzVGWzkBkDU4CRBi7FQmUyClXge8iU1aO/viAgrgtjy3XKQtFvyD2RrPMQQ/KfZm6P+yYBXLJRGWsHdKQk6njyEZB4CB80SdgENSjuGCzZnFiPWJ5UBkzp8pU3lg+sJXLWMr+snrBx2i4jLVTEeOxlDdmyAwz5cEI0bO3s1cOInvwV3QQDTEWCuIg14Q6ithK7DKV5I/iSu5T/PnTgVQsC5Ip+5tP6EyJfGMEYqz+Llhkyv27yGzNndsVyBr3V3itTMWntzApCJJ85pZU1kjSf9vyiS31urx0KtfJQ34CORX0P1mwY3mfMHN38Krc+wIrzWOxnlhqFvINbsHDBCvZKWMO5lZUJeqC5NKOgGoawG6FfMWwMnVBBkKlFPsLQsNA8QWNm7YAAAAASUVORK5CYII=";
#[cfg(test)]
pub(super) const FIXTURE_URI: &str =
    "otpauth://totp/Fixture:alice?secret=JBSWY3DPEHPK3PXP&issuer=Fixture&digits=8&period=45";

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    #[test]
    fn decodes_a_cropped_qr_png_and_refuses_everything_else_without_echoing_it() {
        let png = base64::engine::general_purpose::STANDARD
            .decode(FIXTURE_QR_PNG)
            .unwrap();
        assert_eq!(decode_png_qr(&png).unwrap().as_str(), FIXTURE_URI);
        for bad in [&png[..png.len() / 2], b"not a png".as_slice(), &[]] {
            let error = decode_png_qr(bad).unwrap_err().to_string();
            assert!(
                error.contains("no readable QR code") && !error.contains("JBSWY3DP"),
                "{error}"
            );
        }
        assert!(decode_png_qr(&vec![0u8; IMAGE_LIMIT + 1]).is_err());
    }
}
