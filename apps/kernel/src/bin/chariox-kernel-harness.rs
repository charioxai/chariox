use chariox_kernel::local::run_local_harness;
use chariox_kernel::{DaemonApp, DaemonConfig};

fn main() -> Result<(), chariox_kernel::DaemonError> {
    let app = DaemonApp::bootstrap(DaemonConfig::for_tests())?;
    let report = run_local_harness(app)?;

    println!(
        "local harness ok: session={} prompt_source={} second_attachment={} run={} room_list_schema={} public_agents={} preview={}",
        report.session_id,
        report.prompt_attachment_id,
        report.second_attachment_id,
        report.provider_run_id,
        report.room_list_snapshot_schema_version,
        report.room_list_agent_count,
        report.output_preview.trim_end()
    );

    Ok(())
}
