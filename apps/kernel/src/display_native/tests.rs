//! MP-08/MP-10/MP-11: native pixel contracts, supplementary to live masking.
use super::{
    ffi::{self, Codec, Rect, RowResult},
    raster,
};
#[test]
fn mp08_native_damage_covers_disjoint_changes_since_the_exact_base_and_rejects_dense_tiles() {
    let (w, h) = (1280, 800);
    let base = vec![255; w * h * 4];
    let mut current = base.clone();
    let mut bounds = [0; 4];
    let mut tiles = [0; 128];
    let diff = |raw: &[u8], bounds: &mut [i32; 4], tiles: &mut [i32; 128]| unsafe {
        ffi::cx_capture_difference(
            raw.as_ptr(),
            base.as_ptr(),
            w as i32,
            h as i32,
            bounds.as_mut_ptr(),
            tiles.as_mut_ptr(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(diff(&current, &mut bounds, &mut tiles), 0);
    current[4 * (20 * w + 10)] = 0;
    assert_eq!(diff(&current, &mut bounds, &mut tiles), 1);
    assert_eq!(&tiles[..4], &[0, 0, 32, 32]);
    // An intermediate capture was not presented. Both changes must ship.
    current[4 * (780 * w + 1270)] = 0;
    assert_eq!(diff(&current, &mut bounds, &mut tiles), 2);
    assert_eq!(&tiles[..8], &[0, 0, 32, 32, 1248, 768, 1280, 800]);
    assert_eq!(bounds, [0, 20, 1280, 781]);
    for x in (0..w).step_by(32) {
        current[x * 4] = 0;
    }
    assert_eq!(
        diff(&current, &mut bounds, &mut tiles),
        -1,
        "more than 32 exact tiles takes full repair"
    );
    current.fill(0);
    assert_eq!(diff(&current, &mut bounds, &mut tiles), -1);
    assert_eq!(bounds, [0, 0, 1280, 800]);
}
#[test]
fn mp11_native_masks_intersect_offscreen_bounds_before_pointer_arithmetic() {
    let regions = serde_json::from_value::<Vec<raster::Region>>(serde_json::json!([
        {"x":-100.,"y":0.,"width":20.,"height":20.},
        {"x":-2.,"y":-2.,"width":4.,"height":4.},
        {"x":999.,"y":999.,"width":20.,"height":20.}
    ]))
    .unwrap();
    assert_eq!(
        raster::regions(&regions, 128, 128).unwrap(),
        vec![Rect {
            left: 0,
            top: 0,
            right: 2,
            bottom: 2
        }]
    );
    let original = vec![255; 128 * 128 * 4];
    let masked = raster::masked(
        &original,
        128,
        128,
        &raster::regions(&regions, 128, 128).unwrap(),
    );
    assert_eq!(&masked[..4], &[0, 0, 0, 255]);
    assert_eq!(&masked[(127 * 128 + 127) * 4..], &[255; 4]);
    assert_eq!(original, vec![255; 128 * 128 * 4]);
}
#[test]
fn mp08_native_palette_and_rgb_fallback_preserve_every_rgb_value() {
    for width in [128, 300] {
        let mut raw = Vec::new();
        for y in 0..2 {
            for x in 0..width {
                raw.extend_from_slice(&[(x % 256) as u8, (x / 256) as u8, y as u8, 0]);
            }
        }
        let bytes = raster::png(&raw, width, 2, width as usize * 4).unwrap();
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut reader = decoder.read_info().unwrap();
        let mut out = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut out).unwrap();
        let pixels = &out[..info.buffer_size()];
        for (source, presented) in raw.chunks_exact(4).zip(pixels.chunks_exact(3)) {
            assert_eq!(presented, &[source[2], source[1], source[0]]);
        }
    }
}
#[test]
fn mp11_native_tile_only_plan_retains_exact_rgb_and_opaque_fallback_keeps_full_png() {
    use super::exact::ExactPlan;
    use base64::{engine::general_purpose::STANDARD, Engine};
    for repair_only in [true, false] {
        let plan = ExactPlan {
            pixels: vec![29; 128 * 128 * 4],
            rectangles: vec![[4, 5, 8, 9]],
            w: 128,
            h: 128,
            patch: false,
            repair_only,
            revision: Some(17),
            started: 0.,
        };
        let value = plan.finish().unwrap();
        assert_eq!(value.get("data_base64").is_none(), repair_only);
        assert_eq!(value.get("native_repair").is_some(), repair_only);
        let tile = &value["repair_tiles"][0];
        let data = STANDARD
            .decode(tile["data_base64"].as_str().unwrap())
            .unwrap();
        let mut decoder = png::Decoder::new(std::io::Cursor::new(data));
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut reader = decoder.read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!((info.width, info.height), (4, 4));
        assert_eq!(&pixels[..info.buffer_size()], &[29; 4 * 4 * 3]);
        assert_eq!(value["native_revision"], 17);
    }
}
#[test]
fn mp11_native_codec_masks_before_conversion_and_guards_motion_settle_and_idr() {
    for row_count in [1, 8] {
        let codec = Codec(unsafe { ffi::cx_codec_open(128, 128, 8000000, row_count) });
        assert!(!codec.0.is_null());
        let regions = [Rect {
            left: 36,
            top: 20,
            right: 70,
            bottom: 48,
        }];
        for cycle in 0..12 {
            let source = vec![if cycle % 2 == 0 { 220 } else { 128 }; 128 * 128 * 4];
            let before = source.clone();
            let mut rows = [RowResult::default(); 8];
            let count = unsafe {
                ffi::cx_codec_encode(
                    codec.0,
                    source.as_ptr(),
                    if cycle % 3 == 0 { 255 } else { 0 },
                    regions.as_ptr(),
                    regions.len(),
                    rows.as_mut_ptr(),
                )
            };
            assert_eq!(
                source, before,
                "native masking must not mutate immutable capture leases"
            );
            assert!(
                count >= 0,
                "decoded-output guard must accept opaque mask across native motion/IDR"
            );
            assert!(rows[..count as usize]
                .iter()
                .all(|r| r.length > 0 && r.length < 1024 * 1024));
            if cycle % 3 == 0 {
                assert_eq!(count, row_count);
                assert!(rows[..count as usize]
                    .iter()
                    .all(|r| r.key != 0 && r.sequence == 1));
            }
        }
    }
}
#[test]
fn mp08_native_exact_repairs_colored_pixels_and_retains_only_certified_neutrals_or_overlays() {
    for rows in [1, 8] {
        let codec = Codec(unsafe { ffi::cx_codec_open(128, 128, 8000000, rows) });
        assert!(!codec.0.is_null());
        let mut raw = vec![255; 128 * 128 * 4];
        let mut output = [RowResult::default(); 8];
        assert_eq!(
            unsafe {
                ffi::cx_codec_encode(
                    codec.0,
                    raw.as_ptr(),
                    255,
                    std::ptr::null(),
                    0,
                    output.as_mut_ptr(),
                )
            },
            rows
        );
        let mut bounds = [0; 4];
        unsafe {
            ffi::cx_codec_repair_bounds(
                codec.0,
                raw.as_ptr(),
                std::ptr::null(),
                255,
                0,
                0,
                128,
                128,
                bounds.as_mut_ptr(),
            );
        }
        assert!(
            bounds[0] >= bounds[2] && bounds[1] >= bounds[3],
            "literal decoded white is exact"
        );
        raw[(64 * 128 + 64) * 4..(64 * 128 + 64) * 4 + 3].copy_from_slice(&[11, 22, 33]);
        unsafe {
            ffi::cx_codec_repair_bounds(
                codec.0,
                raw.as_ptr(),
                std::ptr::null(),
                255,
                0,
                0,
                128,
                128,
                bounds.as_mut_ptr(),
            );
        }
        assert_eq!(
            bounds,
            [64, 64, 65, 65],
            "uncertain RGB must receive opaque repair"
        );
        unsafe {
            ffi::cx_codec_repair_bounds(
                codec.0,
                raw.as_ptr(),
                raw.as_ptr(),
                0,
                0,
                0,
                128,
                128,
                bounds.as_mut_ptr(),
            );
        }
        assert!(
            bounds[0] >= bounds[2] && bounds[1] >= bounds[3],
            "an identical lossless overlay is exact"
        );
        unsafe {
            ffi::cx_codec_repair_bounds(
                codec.0,
                raw.as_ptr(),
                std::ptr::null(),
                0,
                0,
                0,
                128,
                128,
                bounds.as_mut_ptr(),
            );
        }
        assert_eq!(
            bounds,
            [0, 0, 128, 128],
            "no delivered reference certifies nothing"
        );
    }
}

#[test]
fn mp08_native_retina_sparse_damage_scales_physical_tiles_and_rejects_dense_motion() {
    let (w, h) = (2560usize, 1600usize);
    let base = vec![255; w * h * 4];
    let mut current = base.clone();
    let mut bounds = [0; 4];
    let mut tiles = [0; 512];
    let mut motion_height = 0;
    for n in 0..96 {
        let x = n % 32 * 32;
        let y = n / 32 * 32;
        current[(y * w + x) * 4] = 0;
    }
    let diff = |raw: &[u8],
                bounds: &mut [i32; 4],
                tiles: &mut [i32; 512],
                motion_height: &mut i32| unsafe {
        ffi::cx_capture_difference(
            raw.as_ptr(),
            base.as_ptr(),
            w as i32,
            h as i32,
            bounds.as_mut_ptr(),
            tiles.as_mut_ptr(),
            motion_height,
        )
    };
    assert_eq!(
        diff(&current, &mut bounds, &mut tiles, &mut motion_height),
        96
    );
    assert_eq!(
        motion_height, 65,
        "codec scheduling retains the proved adjacent vertical span"
    );
    assert_eq!(&tiles[..4], &[0, 0, 32, 32]);
    assert_eq!(&tiles[95 * 4..96 * 4], &[992, 64, 1024, 96]);
    for n in 96..129 {
        let x = n % 40 * 32;
        let y = n / 40 * 32 + 256;
        current[(y * w + x) * 4] = 0;
    }
    assert_eq!(
        diff(&current, &mut bounds, &mut tiles, &mut motion_height),
        -1,
        "Retina cannot exceed128 sparse tiles"
    );
    current.fill(0);
    assert_eq!(
        diff(&current, &mut bounds, &mut tiles, &mut motion_height),
        -1
    );
    assert_eq!(bounds, [0, 0, 2560, 1600]);
    assert_eq!(motion_height, 1600);
}

#[test]
fn mp08_native_recovery_keys_fit_the_paced_link_before_any_rate_feedback() {
    let (w, h) = (1920, 1080);
    let mut source = vec![0u8; w * h * 4];
    let mut noise = 17u32;
    for pixel in source.chunks_exact_mut(4) {
        noise ^= noise << 13;
        noise ^= noise >> 17;
        noise ^= noise << 5;
        pixel.copy_from_slice(&[noise as u8, (noise >> 8) as u8, (noise >> 16) as u8, 255]);
    }
    let codec = Codec(unsafe { ffi::cx_codec_open(w as i32, h as i32, 8000000, 1) });
    assert!(!codec.0.is_null());
    for _ in 0..3 {
        let mut rows = [RowResult::default(); 8];
        assert_eq!(
            unsafe {
                ffi::cx_codec_encode(
                    codec.0,
                    source.as_ptr(),
                    255,
                    std::ptr::null(),
                    0,
                    rows.as_mut_ptr(),
                )
            },
            1
        );
        assert!(rows[0].key != 0 && rows[0].sequence == 1);
        // Twice the 50ms VBV budget leaves headers room without a 300ms key.
        assert!(
            rows[0].length <= 45000,
            "MP-08/MP-10: unpaced recovery key: {}",
            rows[0].length
        );
    }
}

/// Coded luma size from the first SPS of an Annex B baseline packet.
fn sps_size(packet: &[u8]) -> (u32, u32) {
    let start = packet
        .windows(4)
        .position(|w| w[..3] == [0, 0, 1] && w[3] & 31 == 7)
        .expect("SPS")
        + 4;
    let mut rbsp = Vec::new();
    for &byte in &packet[start..start + 32] {
        if byte == 3 && rbsp.ends_with(&[0, 0]) {
            continue;
        }
        rbsp.push(byte);
    }
    struct Bits<'a> {
        bytes: &'a [u8],
        bit: usize,
    }
    impl Bits<'_> {
        fn read(&mut self, n: usize) -> u32 {
            let mut value = 0;
            for _ in 0..n {
                value = value << 1 | (self.bytes[self.bit / 8] >> (7 - self.bit % 8) & 1) as u32;
                self.bit += 1;
            }
            value
        }
        fn golomb(&mut self) -> u32 {
            let mut zeros = 0;
            while self.read(1) == 0 {
                zeros += 1;
            }
            (1 << zeros) - 1 + self.read(zeros)
        }
    }
    let mut bits = Bits {
        bytes: &rbsp,
        bit: 24,
    };
    bits.golomb(); // sps id
    bits.golomb(); // log2_max_frame_num
    if bits.golomb() == 0 {
        bits.golomb();
    } // log2_max_pic_order_cnt_lsb
    bits.golomb(); // max_num_ref_frames
    bits.read(1);
    let (width, height) = (bits.golomb() + 1, bits.golomb() + 1);
    (width * 16, height * 16)
}

#[test]
fn mp08_dense_unprotected_motion_encodes_the_client_admitted_reduced_geometry() {
    let encode = |w: usize, h: usize, rows: i32, regions: &[Rect]| {
        let source = vec![255u8; w * h * 4];
        let codec = Codec(unsafe { ffi::cx_codec_open(w as i32, h as i32, 8000000, rows) });
        let mut results = [RowResult::default(); 8];
        let count = unsafe {
            ffi::cx_codec_encode(
                codec.0,
                source.as_ptr(),
                255,
                regions.as_ptr(),
                regions.len(),
                results.as_mut_ptr(),
            )
        };
        assert_eq!(count, rows);
        let packet = unsafe { std::slice::from_raw_parts(results[0].bytes, results[0].length) };
        let mut bounds = [0; 4];
        unsafe {
            ffi::cx_codec_repair_bounds(
                codec.0,
                source.as_ptr(),
                std::ptr::null(),
                255,
                0,
                0,
                128,
                128,
                bounds.as_mut_ptr(),
            );
        }
        if w > 1280 && regions.is_empty() && rows == 1 {
            let mut edge = [0; 4];
            unsafe {
                ffi::cx_codec_repair_bounds(
                    codec.0,
                    source.as_ptr(),
                    std::ptr::null(),
                    255,
                    w as i32 - 128,
                    h as i32 - 128,
                    128,
                    128,
                    edge.as_mut_ptr(),
                );
            }
            assert_eq!(
                edge,
                [w as i32 - 128, h as i32 - 128, w as i32, h as i32],
                "scaled video cannot certify even the last native tile"
            );
        }
        (sps_size(packet), bounds)
    };
    let (size, bounds) = encode(1920, 1080, 1, &[]);
    assert_eq!(size, (1280, 720), "1080p whole-frame motion is 720p");
    assert_eq!(
        bounds,
        [0, 0, 128, 128],
        "a client-scaled frame certifies nothing"
    );
    assert_eq!(
        encode(2560, 1600, 1, &[]).0,
        (1280, 800),
        "Retina motion is CSS size"
    );
    let (size, bounds) = encode(1280, 800, 1, &[]);
    assert_eq!(size, (1280, 800));
    assert!(bounds[0] >= bounds[2], "native white remains certified");
    let protected = [Rect {
        left: 100,
        top: 100,
        right: 200,
        bottom: 200,
    }];
    assert_eq!(
        encode(1920, 1080, 1, &protected).0,
        (1920, 1088),
        "protected motion stays native"
    );
    assert_eq!(encode(1920, 1080, 8, &[]).0 .0, 1920, "stripes stay native");
}
