// MP-08/MP-10: optional native Linux display worker. Build dependencies are
// explicit; ordinary kernel builds and non-Linux platforms keep their paths.
fn main() {
    for file in [
        "src/display_native/capture.c",
        "src/display_native/codec.c",
        "src/display_native/openh264.c",
        "src/display_native/lossless.c",
    ] {
        println!("cargo:rerun-if-changed={file}");
    }
    for key in [
        "CHARIOX_NATIVE_DISPLAY_INCLUDE",
        "CHARIOX_NATIVE_DISPLAY_LIB",
    ] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    if std::env::var_os("CARGO_FEATURE_NATIVE_DISPLAY").is_none()
        || std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux")
    {
        return;
    }
    let mut build = cc::Build::new();
    build.files([
        "src/display_native/capture.c",
        "src/display_native/codec.c",
        "src/display_native/openh264.c",
        "src/display_native/lossless.c",
    ]);
    if let Some(paths) = std::env::var_os("CHARIOX_NATIVE_DISPLAY_INCLUDE") {
        for path in std::env::split_paths(&paths) {
            build.include(path);
        }
    }
    let mut library_paths = std::env::var_os("CHARIOX_NATIVE_DISPLAY_LIB")
        .map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default();
    // Rust packs static native archives into the rlib before the final C
    // linker runs, so compiler-default multiarch directories must be explicit.
    for library in ["yuv", "webp", "sharpyuv"] {
        if library_paths
            .iter()
            .any(|path| path.join(format!("lib{library}.a")).is_file())
        {
            continue;
        }
        if let Some(path) = build
            .get_compiler()
            .to_command()
            .arg(format!("-print-file-name=lib{library}.a"))
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|path| std::path::PathBuf::from(path.trim()))
            .filter(|path| path.is_absolute() && path.is_file())
        {
            let parent = path.parent().unwrap().to_path_buf();
            if !library_paths.contains(&parent) {
                library_paths.push(parent);
            }
        }
    }
    for path in &library_paths {
        println!("cargo:rustc-link-search=native={}", path.display());
    }
    // Bookworm WebP 1.2 has no separate SharpYUV archive.
    let sharpyuv = library_paths
        .iter()
        .any(|path| path.join("libsharpyuv.a").is_file());
    build.compile("chariox_display_native");
    // OpenH264, x264 and libavcodec are loaded at runtime, never linked.
    for library in ["X11", "Xext", "Xdamage", "Xcomposite", "Xtst", "dl"] {
        println!("cargo:rustc-link-lib={library}");
    }
    println!("cargo:rustc-link-lib=static=yuv");
    println!("cargo:rustc-link-lib=static=webp");
    if sharpyuv {
        println!("cargo:rustc-link-lib=static=sharpyuv");
    }
}
