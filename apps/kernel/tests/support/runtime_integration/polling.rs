use super::*;

pub fn wait_for_terminal_output(
    app: &mut DaemonApp,
    session_id: &str,
    attachment_id: &str,
) -> Vec<chariox_kernel::terminal::TerminalOutputRecord> {
    let timeout_ms = output_timeout_ms();
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        let records = chariox_kernel::transport::TransportService::pump_terminal_output(
            app,
            session_id,
            attachment_id,
        )
        .expect("terminal output should fan out");
        if !records.is_empty() {
            return records;
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for terminal output after {timeout_ms}ms"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

pub fn wait_for_local_terminal_output(
    client: &LocalDaemonClient,
    session_id: &str,
    attachment_id: &str,
) -> String {
    let timeout_ms = output_timeout_ms();
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        let response = client
            .send(LocalDaemonRequest::PumpTerminalOutput(
                PumpTerminalOutputRequest {
                    session_id: session_id.to_string(),
                    attachment_id: attachment_id.to_string(),
                },
            ))
            .expect("terminal output polling should succeed");

        let records = match response {
            LocalDaemonResponse::TerminalOutput { records } => records,
            _ => panic!("unexpected local response"),
        };

        if !records.is_empty() {
            let bytes = records
                .into_iter()
                .flat_map(|record| record.bytes)
                .collect::<Vec<u8>>();
            return String::from_utf8_lossy(&bytes).into_owned();
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for local terminal output after {timeout_ms}ms"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

pub fn wait_for_local_provider_run_ready(
    client: &LocalDaemonClient,
    session_id: &str,
    provider_run_id: &str,
) {
    let timeout_ms = output_timeout_ms();
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        let response = client
            .send(LocalDaemonRequest::GetSessionState(
                GetSessionStateRequest {
                    session_id: session_id.to_string(),
                },
            ))
            .expect("session state polling should succeed");

        if let LocalDaemonResponse::SessionState { session, .. } = response {
            if session.active_provider_run_id() == Some(provider_run_id) {
                return;
            }
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for provider run activation after {timeout_ms}ms"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

pub fn collect_terminal_output_until<F>(
    app: &mut DaemonApp,
    session_id: &str,
    attachment_id: &str,
    done: F,
) -> String
where
    F: Fn(&str, &chariox_kernel::session::RuntimeSession) -> bool,
{
    let timeout_ms = output_timeout_ms().max(8_000);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut output = Vec::new();

    loop {
        let records = chariox_kernel::transport::TransportService::pump_terminal_output(
            app,
            session_id,
            attachment_id,
        )
        .expect("terminal output should fan out");
        for record in records {
            output.extend(record.bytes);
        }

        let output_text = String::from_utf8_lossy(&output).into_owned();
        let session = app
            .sessions()
            .get_session(session_id)
            .expect("session should still exist");
        if done(&output_text, &session) {
            return output_text;
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for terminal output after {timeout_ms}ms: {output_text}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

pub fn collect_provider_output_until<F>(
    app: &mut DaemonApp,
    session_id: &str,
    provider_run_id: &str,
    recipient_attachment_ids: Vec<String>,
    done: F,
) -> String
where
    F: Fn(&str, &DaemonApp) -> bool,
{
    let timeout_ms = output_timeout_ms().max(8_000);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut output = Vec::new();

    loop {
        let records = chariox_kernel::transport::TransportService::pump_provider_output(
            app,
            session_id,
            provider_run_id,
            recipient_attachment_ids.clone(),
        )
        .expect("provider output should fan out");
        for record in records {
            output.extend(record.bytes);
        }

        let output_text = String::from_utf8_lossy(&output).into_owned();
        if done(&output_text, app) {
            return output_text;
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for provider output after {timeout_ms}ms: {output_text}; runs={:?}; session={:?}",
            app.providers()
                .list_runs()
                .into_iter()
                .map(|run| (run.id().to_string(), run.state()))
                .collect::<Vec<_>>(),
            app.sessions().get_session(session_id),
        );
        thread::sleep(Duration::from_millis(25));
    }
}

pub fn collect_provider_output_for_agent_until<F>(
    app: &mut DaemonApp,
    session_id: &str,
    agent_id: &str,
    initial_provider_run_id: &str,
    recipient_attachment_ids: Vec<String>,
    done: F,
) -> String
where
    F: Fn(&str, &DaemonApp) -> bool,
{
    let timeout_ms = output_timeout_ms().max(8_000);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut output = Vec::new();

    loop {
        let provider_run_id = app
            .providers()
            .get_run_for_agent(session_id, agent_id)
            .map(|run| run.id().to_string())
            .unwrap_or_else(|| initial_provider_run_id.to_string());
        let records = chariox_kernel::transport::TransportService::pump_provider_output(
            app,
            session_id,
            &provider_run_id,
            recipient_attachment_ids.clone(),
        )
        .expect("provider output should fan out");
        for record in records {
            output.extend(record.bytes);
        }

        let output_text = String::from_utf8_lossy(&output).into_owned();
        if done(&output_text, app) {
            return output_text;
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for provider output after {timeout_ms}ms: {output_text}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

pub fn collect_provider_records_until<F>(
    app: &mut DaemonApp,
    session_id: &str,
    provider_run_id: &str,
    recipient_attachment_ids: Vec<String>,
    done: F,
) -> Vec<chariox_kernel::terminal::TerminalOutputRecord>
where
    F: Fn(&[chariox_kernel::terminal::TerminalOutputRecord], &DaemonApp) -> bool,
{
    let timeout_ms = output_timeout_ms().max(4_000);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut records = Vec::new();

    loop {
        let next = chariox_kernel::transport::TransportService::pump_provider_output(
            app,
            session_id,
            provider_run_id,
            recipient_attachment_ids.clone(),
        )
        .expect("provider output should fan out");
        records.extend(next);

        if done(&records, app) {
            return records;
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for provider records after {timeout_ms}ms: {}",
            render_terminal_output(&records)
        );
        thread::sleep(Duration::from_millis(25));
    }
}

pub fn wait_for_provider_runtime_state(
    app: &DaemonApp,
    provider_run_id: &str,
    expected_bound: bool,
    context: &str,
) {
    let deadline = Instant::now() + Duration::from_millis(output_timeout_ms().max(4_000));
    while app
        .providers()
        .structured_runtime_state_bound_for_tests(provider_run_id)
        != expected_bound
    {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for provider runtime state to become {expected_bound} while {context}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

pub fn render_terminal_output(
    records: &[chariox_kernel::terminal::TerminalOutputRecord],
) -> String {
    let mut output = Vec::new();
    for record in records {
        output.extend_from_slice(&record.bytes);
    }
    String::from_utf8_lossy(&output).into_owned()
}
