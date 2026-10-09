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
    if let Some(paths) = std::env::var_os("CHARIOX_NATIVE_DISPLAY_LIB") {
        for path in std::env::split_paths(&paths) {
            println!("cargo:rustc-link-search=native={}", path.display());
        }
    }
    build.compile("chariox_display_native");
    // OpenH264, x264 and libavcodec are loaded at runtime, never linked.
    for library in ["X11", "Xext", "Xdamage", "Xcomposite", "Xtst", "dl"] {
        println!("cargo:rustc-link-lib={library}");
    }
    println!("cargo:rustc-link-lib=static=yuv");
    println!("cargo:rustc-link-lib=static=webp");
    println!("cargo:rustc-link-lib=static=sharpyuv");
}
