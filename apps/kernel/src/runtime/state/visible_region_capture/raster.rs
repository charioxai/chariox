use super::capture_error;
use crate::{error::DaemonError, local::ScreenshotRegion};
use std::io::Cursor;

const MAX_PIXELS: u64 = 16 * 1024 * 1024;
const MAX_PNG: usize = 16 * 1024 * 1024;
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CaptureMask {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
pub(super) struct Cropped {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub(super) fn validate_region(r: &ScreenshotRegion) -> Result<(), DaemonError> {
    if r.width == 0
        || r.height == 0
        || r.viewport_width == 0
        || r.viewport_height == 0
        || r.viewport_width > 32768
        || r.viewport_height > 32768
        || u64::from(r.x) + u64::from(r.width) > u64::from(r.viewport_width)
        || u64::from(r.y) + u64::from(r.height) > u64::from(r.viewport_height)
        || r.frame_width == 0
        || r.frame_height == 0
        || u64::from(r.frame_width) * u64::from(r.frame_height) > MAX_PIXELS
    {
        return Err(capture_error("Region must fit the visible painted surface"));
    }
    Ok(())
}
pub(super) fn crop_png(
    bytes: &[u8],
    r: &ScreenshotRegion,
    masks: &[CaptureMask],
) -> Result<Cropped, DaemonError> {
    validate_region(r)?;
    if bytes.len() > MAX_PNG || masks.len() > 1024 {
        return Err(capture_error("Capture exceeds limit"));
    }
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_limits(png::Limits {
        bytes: (MAX_PIXELS * 4) as usize,
    });
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|_| capture_error("Invalid screenshot PNG"))?;
    if reader.info().width != r.frame_width
        || reader.info().height != r.frame_height
        || reader.info().animation_control.is_some()
    {
        return Err(capture_error(
            "Screenshot dimensions changed or animated PNG refused",
        ));
    }
    let mut decoded = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut decoded)
        .map_err(|_| capture_error("Invalid screenshot pixels"))?;
    let channels = match info.color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        _ => return Err(capture_error("Unsupported PNG pixels")),
    };
    // Mask full source before cropping: neither crop nor scaling can reveal
    // protected edge pixels. Never store the unmasked host image.
    for m in masks {
        if [m.x, m.y, m.width, m.height].iter().any(|n| !n.is_finite())
            || m.width < 0.0
            || m.height < 0.0
        {
            return Err(capture_error("Invalid protection region"));
        }
        let x0 = m.x.floor().max(0.0).min(r.frame_width as f64) as usize;
        let y0 = m.y.floor().max(0.0).min(r.frame_height as f64) as usize;
        let x1 = (m.x + m.width).ceil().max(0.0).min(r.frame_width as f64) as usize;
        let y1 = (m.y + m.height).ceil().max(0.0).min(r.frame_height as f64) as usize;
        for y in y0..y1 {
            for x in x0..x1 {
                let at = (y * r.frame_width as usize + x) * channels;
                decoded[at..at + channels].fill(0);
                if channels == 4 || channels == 2 {
                    decoded[at + channels - 1] = 255;
                }
            }
        }
    }
    let floor = |n: u32, pixels: u32, viewport: u32| {
        (u64::from(n) * u64::from(pixels) / u64::from(viewport)) as u32
    };
    let ceil = |n: u32, pixels: u32, viewport: u32| {
        (u64::from(n) * u64::from(pixels)).div_ceil(u64::from(viewport)) as u32
    };
    let x0 = floor(r.x, r.frame_width, r.viewport_width);
    let y0 = floor(r.y, r.frame_height, r.viewport_height);
    let width = ceil(r.x + r.width, r.frame_width, r.viewport_width) - x0;
    let height = ceil(r.y + r.height, r.frame_height, r.viewport_height) - y0;
    let stride = width as usize * channels;
    let mut pixels = Vec::with_capacity(stride * height as usize);
    for y in y0..y0 + height {
        let at = (y as usize * r.frame_width as usize + x0 as usize) * channels;
        pixels.extend_from_slice(&decoded[at..at + stride]);
    }
    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut output, width, height);
        encoder.set_color(info.color_type);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|_| capture_error("PNG encoder unavailable"))?;
        writer
            .write_image_data(&pixels)
            .map_err(|_| capture_error("PNG encoding failed"))?;
    }
    if output.len() > 4 * 1024 * 1024 {
        return Err(capture_error(
            "Selected PNG exceeds attachment capture limit",
        ));
    }
    Ok(Cropped {
        bytes: output,
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn region() -> ScreenshotRegion {
        ScreenshotRegion {
            x: 1,
            y: 1,
            width: 2,
            height: 2,
            viewport_width: 4,
            viewport_height: 4,
            frame_width: 8,
            frame_height: 8,
        }
    }
    fn fixture() -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut e = png::Encoder::new(&mut bytes, 8, 8);
            e.set_color(png::ColorType::Rgb);
            e.write_header()
                .unwrap()
                .write_image_data(&[200; 192])
                .unwrap();
        }
        bytes
    }
    #[test]
    fn maps_scaled_region_and_masks_before_crop() {
        let image = crop_png(
            &fixture(),
            &region(),
            &[CaptureMask {
                x: 2.5,
                y: 2.5,
                width: 1.0,
                height: 1.0,
            }],
        )
        .unwrap();
        assert_eq!((image.width, image.height), (4, 4));
        let mut reader = png::Decoder::new(Cursor::new(image.bytes))
            .read_info()
            .unwrap();
        let mut bytes = vec![0; reader.output_buffer_size()];
        reader.next_frame(&mut bytes).unwrap();
        assert_eq!(&bytes[..6], &[0; 6]);
        assert_eq!(&bytes[6..9], &[200; 3]);
    }
    #[test]
    fn captured_png_uses_existing_inline_prompt_attachment_path() {
        use crate::runtime::agent_actor::prompt_attachment_materialization::{
            inline_prompt_attachment_root, materialize_inline_prompt_attachments,
        };
        use base64::Engine as _;
        let captured = crop_png(&fixture(), &region(), &[]).unwrap();
        let session = format!("screenshot-test-{:016x}", rand::random::<u64>());
        let attachment = crate::session::PromptAttachment::new(
            "chariox-terminal://screenshot/capture-1?source=room-1",
            "image/png",
            Some("chariox-room-1-12.png".into()),
        )
        .with_contents_base64(base64::engine::general_purpose::STANDARD.encode(&captured.bytes));
        let materialized =
            materialize_inline_prompt_attachments(&session, "fixture-agent", vec![attachment])
                .unwrap();
        assert_eq!(materialized[0].mime(), "image/png");
        assert_eq!(materialized[0].filename(), Some("chariox-room-1-12.png"));
        let path = materialized[0].url().strip_prefix("file://").unwrap();
        assert_eq!(std::fs::read(path).unwrap(), captured.bytes);
        let root = inline_prompt_attachment_root(&session, "fixture-agent");
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::remove_dir(root.parent().unwrap()).unwrap();
    }
    #[test]
    fn rejects_overflow_offscreen_stale_and_malformed_images() {
        let mut r = region();
        r.x = u32::MAX;
        assert!(validate_region(&r).is_err());
        r = region();
        r.width = 0;
        assert!(validate_region(&r).is_err());
        r = region();
        r.frame_width = 9;
        assert!(crop_png(&fixture(), &r, &[]).is_err());
        assert!(crop_png(b"not PNG", &region(), &[]).is_err());
        assert!(crop_png(
            &fixture(),
            &region(),
            &[CaptureMask {
                x: f64::NAN,
                y: 0.0,
                width: 1.0,
                height: 1.0
            }]
        )
        .is_err());
    }
}
