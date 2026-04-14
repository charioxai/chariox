use arroba_daemon::local::run_local_harness;
use arroba_daemon::{DaemonApp, DaemonConfig};

fn main() -> Result<(), arroba_daemon::DaemonError> {
    let app = DaemonApp::bootstrap(DaemonConfig::for_tests())?;
    let report = run_local_harness(app)?;

    println!(
        "local harness ok: session={} prompt_source={} second_attachment={} run={} preview={}",
        report.session_id,
        report.prompt_attachment_id,
        report.second_attachment_id,
        report.provider_run_id,
        report.output_preview.trim_end()
    );

    Ok(())
}
