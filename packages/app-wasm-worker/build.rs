fn main() {
    println!(
        "cargo:rustc-env=CHARIOX_WORKER_TARGET={}",
        std::env::var("TARGET").unwrap()
    );
}
