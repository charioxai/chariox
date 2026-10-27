#[cfg(any(target_os = "macos", target_os = "linux"))]
fn main() {
    use chariox_app_package::{developer, ErrorCode, PackageError};
    use std::io::Write;

    let args: Result<Vec<String>, _> = std::env::args_os()
        .skip(1)
        .map(|value| value.into_string())
        .collect();
    let result = args
        .map_err(|_| PackageError {
            code: ErrorCode::InvalidArguments,
            message: "command arguments must be UTF-8".to_owned(),
        })
        .and_then(|args| developer::run_cli(&args));
    let (value, code) = match result {
        Ok(result) => (serde_json::json!({"ok":true,"result":result}), 0),
        Err(error) => {
            let code = developer::exit_code(&error);
            (serde_json::json!({"ok":false,"error":error}), code)
        }
    };
    // One JSON record, including errors, so a wrapper never parses prose. No
    // private seed, public key bytes, or tool/build output is emitted.
    let mut stdout = std::io::stdout().lock();
    if serde_json::to_writer(&mut stdout, &value).is_err() || stdout.write_all(b"\n").is_err() {
        std::process::exit(4);
    }
    std::process::exit(code);
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn main() {
    println!("{{\"ok\":false,\"error\":{{\"code\":\"UNSUPPORTED_FEATURE\",\"message\":\"developer file commands currently require macOS or Linux\"}}}}");
    std::process::exit(3);
}
