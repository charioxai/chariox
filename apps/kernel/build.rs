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
    let library_paths = std::env::var_os("CHARIOX_NATIVE_DISPLAY_LIB")
        .map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default();
    for path in &library_paths {
        println!("cargo:rustc-link-search=native={}", path.display());
    }
    // MP-08/MP-10/MP-11: WebP 1.2 (Bookworm) has no separate SharpYUV
    // archive. Newer WebP needs it after libwebp; use the selected compiler's
    // search path as well as explicit native-library paths.
    let sharpyuv = library_paths
        .iter()
        .any(|path| path.join("libsharpyuv.a").is_file())
        || build
            .get_compiler()
            .to_command()
            .arg("-print-file-name=libsharpyuv.a")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .is_some_and(|path| std::path::Path::new(path.trim()).is_file());
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
