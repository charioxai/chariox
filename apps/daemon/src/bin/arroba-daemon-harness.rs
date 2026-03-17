use arroba_daemon::local::run_local_harness;
use arroba_daemon::{DaemonApp, DaemonConfig};

fn main() -> Result<(), arroba_daemon::DaemonError> {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())?;
    let report = run_local_harness(&mut app)?;

    println!(
        "local harness ok: session={} controller={} observer={} run={} preview={}",
        report.session_id,
        report.controller_attachment_id,
        report.observer_attachment_id,
        report.provider_run_id,
        report.output_preview.trim_end()
    );

    Ok(())
}
