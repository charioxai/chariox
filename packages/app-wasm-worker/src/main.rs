#[cfg(all(not(test), not(debug_assertions)))]
const _: () = chariox_app_runtime::assert_production_build();

fn main() -> std::process::ExitCode {
    chariox_app_wasm_worker::cli::main()
}
