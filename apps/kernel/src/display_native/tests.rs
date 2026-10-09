//! MP-08/MP-10/MP-11: native pixel contracts, supplementary to live masking.
use super::{
    ffi::{self, Codec, Rect, RowResult},
    raster,
};
#[test]
fn mp11_codec_lease_prevents_reuse_and_keeps_mapping_alive_after_owner_drop() {
    use std::os::unix::fs::DirBuilderExt;
    let root =
        std::env::temp_dir().join(format!("chariox-mp11-slot-{:032x}", rand::random::<u128>()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&root)
        .unwrap();
    let mut slot = raster::Slot::create(&root, 0, 4096).unwrap();
    unsafe {
        *slot.pixels = 0x5a;
    }
    slot.serial = Some(7);
    let lease = slot.codec_lease();
    slot.serial = None; // Client timed out while the codec is still reading.
    assert!(
        !slot.available(),
        "MP-11: released slot remains codec-owned"
    );
    drop(lease);
    assert!(slot.available());
    let lease = slot.codec_lease();
    drop(slot); // Supervisor failure cannot unmap the encoder's input.
    assert_eq!(unsafe { *lease.pixels() }, 0x5a);
    drop(lease);
    std::fs::remove_dir(root).unwrap();
}
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
            tiles: Vec::new(),
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
        assert_eq!(tile["format"], "webp");
        let data = STANDARD
            .decode(tile["data_base64"].as_str().unwrap())
            .unwrap();
        let (width, height, rgb) = webp_rgb(&data);
        assert_eq!((width, height), (4, 4));
        assert_eq!(rgb, vec![29; 4 * 4 * 3]);
        assert_eq!(value["native_revision"], 17);
    }
}
#[test]
fn mp11_native_codec_masks_before_conversion_and_guards_motion_settle_and_idr() {
    for row_count in [1, 8] {
        let codec = Codec(unsafe { ffi::cx_codec_open(128, 128, 8000000, row_count, 0) });
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
        let codec = Codec(unsafe { ffi::cx_codec_open(128, 128, 8000000, rows, 0) });
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
    let codec = Codec(unsafe { ffi::cx_codec_open(w as i32, h as i32, 8000000, 1, 0) });
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
        let packet = unsafe { std::slice::from_raw_parts(rows[0].bytes, rows[0].length) };
        assert_eq!(sps_size(packet), (1920, 1088), "the native 1080p key");
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
fn mp08_contention_fallback_motion_encodes_the_client_admitted_reduced_geometry() {
    // MP-08/MP-10: native resolution unless the contention fallback is open.
    for (w, h, native) in [(1920, 1080, (1920, 1088)), (2560, 1600, (2560, 1600))] {
        let source = vec![255u8; w * h * 4];
        let codec = Codec(unsafe { ffi::cx_codec_open(w as i32, h as i32, 8000000, 1, 0) });
        let mut results = [RowResult::default(); 8];
        let count = unsafe {
            ffi::cx_codec_encode(
                codec.0,
                source.as_ptr(),
                255,
                std::ptr::null(),
                0,
                results.as_mut_ptr(),
            )
        };
        assert_eq!(count, 1);
        let packet = unsafe { std::slice::from_raw_parts(results[0].bytes, results[0].length) };
        assert_eq!(
            sps_size(packet),
            native,
            "default motion keeps native detail"
        );
    }
    let encode = |w: usize, h: usize, rows: i32, regions: &[Rect]| {
        let source = vec![255u8; w * h * 4];
        let codec = Codec(unsafe { ffi::cx_codec_open(w as i32, h as i32, 8000000, rows, 1) });
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
        // A render node whose h264_vaapi init failed encodes software and
        // takes the fallback geometry; only working hardware stays native.
        if unsafe { ffi::cx_codec_backend(codec.0) } == 1 {
            return None;
        }
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
        Some((sps_size(packet), bounds))
    };
    let Some((size, bounds)) = encode(1920, 1080, 1, &[]) else {
        return;
    };
    assert_eq!(size, (1280, 720), "1080p whole-frame motion is 720p");
    assert_eq!(
        bounds,
        [0, 0, 128, 128],
        "a client-scaled frame certifies nothing"
    );
    assert_eq!(
        encode(2560, 1600, 1, &[]).unwrap().0,
        (1280, 800),
        "Retina motion is CSS size"
    );
    let (size, bounds) = encode(1280, 800, 1, &[]).unwrap();
    assert_eq!(size, (1280, 800));
    assert!(bounds[0] >= bounds[2], "native white remains certified");
    let protected = [Rect {
        left: 100,
        top: 100,
        right: 200,
        bottom: 200,
    }];
    assert_eq!(
        encode(1920, 1080, 1, &protected).unwrap().0,
        (1920, 1088),
        "protected motion stays native"
    );
    assert_eq!(
        encode(1920, 1080, 8, &[]).unwrap().0 .0,
        1920,
        "stripes stay native"
    );
}

fn webp_rgb(data: &[u8]) -> (i32, i32, Vec<u8>) {
    let (mut width, mut height) = (0, 0);
    let pixels = unsafe { ffi::WebPDecodeRGB(data.as_ptr(), data.len(), &mut width, &mut height) };
    assert!(!pixels.is_null());
    let rgb = unsafe { std::slice::from_raw_parts(pixels, (width * height * 3) as usize) }.to_vec();
    unsafe { ffi::cx_webp_free(pixels) };
    (width, height, rgb)
}
#[test]
fn mp08_lossless_webp_regions_decode_rgb_exact_at_both_repair_efforts() {
    let (w, h) = (300usize, 70usize);
    let mut pixels = vec![0u8; w * h * 4];
    for (i, p) in pixels.chunks_exact_mut(4).enumerate() {
        let (x, y) = (i % w, i / w);
        p.copy_from_slice(&[(x * 7 + y) as u8, (y * 13) as u8, (x ^ y) as u8, 0x5a]);
    }
    // MP-08/MP-10/MP-11: both repair effort levels preserve every RGB value
    // and reject clips outside the admitted raster.
    for effort in [25, 50] {
        let bytes = raster::webp(&pixels, w * 4, [17, 9, 251, 53], 1, effort).unwrap();
        let (width, height, rgb) = webp_rgb(&bytes);
        assert_eq!((width, height), (251, 53));
        for y in 0..53 {
            for x in 0..251 {
                let p = &pixels[((y + 9) * w + x + 17) * 4..][..4];
                assert_eq!(&rgb[(y * 251 + x) * 3..][..3], &[p[2], p[1], p[0]]);
            }
        }
        assert!(raster::webp(&pixels, w * 4, [290, 0, 11, 1], 1, effort).is_err());
        assert!(raster::webp(&pixels, w * 4, [0, 69, 1, 2], 1, effort).is_err());
    }
}
#[test]
fn mp08_scroll_plan_proves_moves_fixed_cells_and_exposed_rows() {
    let (w, h, dy) = (1280i32, 800i32, -37i32);
    let row = |y: i32| -> Vec<u8> {
        (0..w)
            .flat_map(|x| {
                let v = ((x * 31 + y * 17) ^ y.wrapping_mul(y)) as u8;
                [v, v.wrapping_add(y as u8), (x / 5 + y / 256 * 77) as u8, 0]
            })
            .collect()
    };
    let document = |offset: i32| -> Vec<u8> { (0..h).flat_map(|y| row(y + offset)).collect() };
    let base = document(0);
    let mut current = document(-dy);
    // A fixed (unscrolled) element and one changed cell elsewhere.
    for y in 10..42 {
        let at = ((y * w + 1200) * 4) as usize;
        current[at..at + 64 * 4].copy_from_slice(&base[at..at + 64 * 4]);
    }
    let changed = ((400 * w + 70) * 4) as usize;
    current[changed] ^= 0xff;
    let mut out = [0i32; 4 + 128 * 4];
    assert_eq!(
        unsafe { ffi::cx_shift_plan(current.as_ptr(), base.as_ptr(), w, h, out.as_mut_ptr()) },
        1
    );
    assert_eq!(out[0], dy);
    let (moves, dirty) = (out[1] as usize, out[2] as usize);
    let rects = |start: usize, count: usize| {
        out[start..start + count * 4]
            .chunks_exact(4)
            .map(|r| [r[0], r[1], r[2], r[3]])
            .collect::<Vec<_>>()
    };
    let (moves, dirty) = (rects(4, moves), rects(4 + moves * 4, dirty));
    // Reconstruct the viewer: unchanged base, then moves from one snapshot,
    // then exact residuals from the current raster. Must equal `current`.
    let mut viewer = base.clone();
    for [x, y, rw, rh] in &moves {
        for r in 0..*rh {
            let (dst, src) = (((y + r) * w + x) * 4, ((y + r - dy) * w + x) * 4);
            viewer[dst as usize..(dst + rw * 4) as usize]
                .copy_from_slice(&base[src as usize..(src + rw * 4) as usize]);
        }
    }
    let mut residual = 0;
    for [x, y, rw, rh] in &dirty {
        assert!(rw * rh <= 2560 * 256);
        residual += rw * rh;
        for r in 0..*rh {
            let at = (((y + r) * w + x) * 4) as usize;
            viewer[at..at + (*rw * 4) as usize]
                .copy_from_slice(&current[at..at + (*rw * 4) as usize]);
        }
    }
    assert!(
        viewer == current,
        "MP-08: scroll plan did not reconstruct the exact raster"
    );
    assert_eq!(residual, out[3]);
    // Exposed rows plus the changed cell, not the whole frame.
    assert!(residual < w * (-dy) + 64 * 64 * 2, "residual {residual}");
    // Unrelated rasters produce no plan.
    let noise: Vec<u8> = (0..w * h * 4)
        .map(|i| (i.wrapping_mul(2654435761u32 as i32) >> 7) as u8)
        .collect();
    assert_eq!(
        unsafe { ffi::cx_shift_plan(noise.as_ptr(), base.as_ptr(), w, h, out.as_mut_ptr()) },
        0
    );
}
#[test]
fn mp08_committed_canvas_plan_reencodes_only_changed_cells_without_a_scroll() {
    let (w, h) = (1280i32, 800i32);
    // Pseudo-random (non-periodic) rows: no vertical offset can match.
    let mut state = 0x2545F491u32;
    let base: Vec<u8> = (0..w * h * 4)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect();
    let mut current = base.clone();
    for y in 300..320 {
        for x in 500..540 {
            current[((y * w + x) * 4) as usize] ^= 0x5a;
        }
    }
    let mut out = [0i32; 4 + 128 * 4];
    assert_eq!(
        unsafe { ffi::cx_shift_plan(current.as_ptr(), base.as_ptr(), w, h, out.as_mut_ptr()) },
        1
    );
    assert_eq!((out[0], out[1]), (0, 0), "static plan: no offset, no moves");
    let dirty: Vec<[i32; 4]> = out[4..4 + out[2] as usize * 4]
        .chunks_exact(4)
        .map(|r| [r[0], r[1], r[2], r[3]])
        .collect();
    let mut viewer = base.clone();
    for [x, y, rw, rh] in &dirty {
        for r in 0..*rh {
            let at = (((y + r) * w + x) * 4) as usize;
            viewer[at..at + (*rw * 4) as usize]
                .copy_from_slice(&current[at..at + (*rw * 4) as usize]);
        }
    }
    assert!(viewer == current);
    assert!(
        out[3] <= 2 * 64 * 20 + 600,
        "only the two touched cell columns: {}",
        out[3]
    );
    assert_eq!(
        unsafe { ffi::cx_shift_plan(base.as_ptr(), base.as_ptr(), w, h, out.as_mut_ptr()) },
        0,
        "nothing to send"
    );
}
#[test]
fn mp11_protected_retina_rows_keep_the_mask_black_at_the_paced_rate() {
    // MP-08/MP-11: dense detail starves a 1.5-frame VBV (x264 emergency QPs);
    // masked macroblocks used to decode grey, fail output_safe and drop
    // every frame.
    let (w, h) = (2560usize, 1600usize);
    let regions = [
        Rect {
            left: 1001,
            top: 403,
            right: 1137,
            bottom: 447,
        },
        Rect {
            left: 90,
            top: 603,
            right: 333,
            bottom: 640,
        },
    ];
    for row_count in [1, 8] {
        let codec = Codec(unsafe { ffi::cx_codec_open(w as i32, h as i32, 8000000, row_count, 0) });
        assert!(!codec.0.is_null());
        for frame in 0..6 {
            // Dense text-like strokes on white, scrolled each frame.
            let source: Vec<u8> = (0..h)
                .flat_map(|y| (0..w).map(move |x| (x, y)))
                .flat_map(|(x, y)| {
                    let yy = y + frame * 37;
                    let mut k = ((x / 3) as u32).wrapping_mul(2654435761)
                        ^ ((yy / 3) as u32).wrapping_mul(40503);
                    k ^= k >> 15;
                    let ink = yy % 26 < 16 && k % 3 == 0;
                    if ink {
                        [30, 40, if k & 64 != 0 { 200 } else { 35 }, 255]
                    } else {
                        [250, 248, 245, 255]
                    }
                })
                .collect();
            let mut rows = [RowResult::default(); 8];
            let count = unsafe {
                ffi::cx_codec_encode(
                    codec.0,
                    source.as_ptr(),
                    if frame == 0 { 255 } else { 0 },
                    regions.as_ptr(),
                    regions.len(),
                    rows.as_mut_ptr(),
                )
            };
            assert!(
                count >= 0,
                "protected rows={row_count} frame={frame} dropped ({count})"
            );
            let bytes: usize = rows[..count as usize].iter().map(|row| row.length).sum();
            assert!(bytes <= 1024 * 1024 - 4096,
                "MP-08/MP-10/MP-11 protected aggregate leaves room for packet headers: {bytes}");
        }
    }
}

#[test]
fn mp08_native_codec_prefers_runtime_openh264_and_keeps_x264_optional() {
    // Owner 2026-10-09: OpenH264 is the default encoder, x264 an opt-in; both
    // load at runtime. A configured OpenH264 binary must be the one in use.
    let codec = Codec(unsafe { ffi::cx_codec_open(64, 64, 1_000_000, 1, 0) });
    assert!(!codec.0.is_null(), "MP-10: a runtime codec must load");
    let x264 = std::env::var("CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER").as_deref() == Ok("libx264");
    if std::env::var_os("CHARIOX_BROWSER_DISPLAY_OPENH264").is_some() {
        assert_eq!(unsafe { ffi::cx_codec_openh264(codec.0) } == 1, !x264);
    }
}
