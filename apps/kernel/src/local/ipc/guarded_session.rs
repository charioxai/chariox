use super::*;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionVersion {
    version: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Negotiation {
    #[serde(rename = "GuardedControlSession")]
    session: SessionVersion,
}

#[derive(Serialize)]
struct Admission {
    session: SessionVersion,
    #[serde(flatten)]
    envelope: IpcResponseEnvelope,
}

pub(super) async fn handle(
    router: &Arc<CommandRouter>,
    sequence: &AtomicU64,
    stream: &mut tokio::net::UnixStream,
    bytes: &[u8],
) -> Result<(), DaemonError> {
    let result = run(router, sequence, stream, bytes).await;
    if let Err(error) = result {
        crate::logging::warn_with_fields(
            "daemon.ipc.server",
            "guarded local request failed",
            serde_json::json!({"error": error.to_string()}),
        );
        let bytes = encode_envelope(IpcResponseEnvelope {
            response: None,
            error: Some(response_error_message(&error)),
        })?;
        return write_async_frame(stream, &bytes).await;
    }
    Ok(())
}

fn response_error_message(error: &DaemonError) -> String {
    if matches!(error, DaemonError::LocalTransport { operation, message }
        if *operation == "decode local frame" && message.contains("payload exceeded"))
    {
        "local payload exceeded ipc frame limit; request a smaller payload".to_string()
    } else {
        error.to_string()
    }
}

fn invalid(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "guarded local control",
        message: message.to_string(),
    }
}

async fn run(
    router: &Arc<CommandRouter>,
    sequence: &AtomicU64,
    stream: &mut tokio::net::UnixStream,
    bytes: &[u8],
) -> Result<(), DaemonError> {
    let negotiation: Negotiation = serde_json::from_slice(bytes).map_err(|error| {
        invalid(&crate::transport::request_decode_error::message(
            "invalid session negotiation",
            &error,
        ))
    })?;
    if negotiation.session.version != 1 {
        return Err(invalid("unsupported session version"));
    }
    let response = dispatch_local_ipc_request(
        router,
        sequence,
        LocalDaemonRequest::RelayStatus(crate::local::api::RelayStatusRequest),
    )
    .await?;
    let admission = Admission {
        session: SessionVersion { version: 1 },
        envelope: IpcResponseEnvelope {
            response: Some(response),
            error: None,
        },
    };
    let mut value =
        serde_json::to_value(admission).map_err(|_| invalid("encode session admission"))?;
    crate::local::redact_client_response_value(&mut value);
    let bytes = serde_json::to_vec(&value).map_err(|_| invalid("encode session admission"))?;
    write_open_async_frame(stream, &bytes).await?;
    let bytes = read_async_frame(stream).await?;
    let request: LocalDaemonRequest = serde_json::from_slice(&bytes).map_err(|error| {
        invalid(&crate::transport::request_decode_error::message(
            "invalid guarded command",
            &error,
        ))
    })?;
    if !matches!(
        &request,
        LocalDaemonRequest::CreateDisposableWorker(_)
            | LocalDaemonRequest::GetDisposableWorker(_)
            | LocalDaemonRequest::ReleaseDisposableWorker(_)
            | LocalDaemonRequest::KeepDisposableWorkerRunning(_)
            | LocalDaemonRequest::PrepareDisposableWorkerContextTransfer(_)
            | LocalDaemonRequest::KeepManagedEnvironmentRunning(_)
    ) {
        return Err(invalid(
            "session accepts exactly one guarded control command",
        ));
    }
    let response = dispatch_local_ipc_request(router, sequence, request).await?;
    write_async_frame(
        stream,
        &encode_envelope(IpcResponseEnvelope {
            response: Some(response),
            error: None,
        })?,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transport_snapshot_requires_protocol_371() {
        assert_eq!(crate::local::api::LOCAL_DAEMON_PROTOCOL_VERSION, 411);
        assert_eq!(
            serde_json::to_value(Negotiation {
                session: SessionVersion { version: 1 }
            })
            .unwrap(),
            serde_json::json!({"GuardedControlSession":{"version":1}})
        );
        let status = serde_json::json!({"configured":false,"connected":false,"relay_url":null,"relay_token_configured":false,"daemon_id":"home","daemon_alias":null,"machine_id":"machine","machine_alias":null,"capabilities":["disposable_worker_control_v1"]});
        let response = LocalDaemonResponse::RelayStatus {
            status: serde_json::from_value(status.clone()).unwrap(),
        };
        let admission = Admission {
            session: SessionVersion { version: 1 },
            envelope: IpcResponseEnvelope {
                response: Some(response),
                error: None,
            },
        };
        assert_eq!(
            serde_json::to_value(admission).unwrap(),
            serde_json::json!({"session":{"version":1},"error":null,"response":{"RelayStatus":{"status":status}}})
        );
        assert!(serde_json::from_value::<Negotiation>(
            serde_json::json!({"GuardedControlSession":{"version":1},"RelayStatus":null})
        )
        .is_err());
    }
}
