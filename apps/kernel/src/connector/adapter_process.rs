use super::*;

impl ConnectorAdapterProcessPool {
    pub async fn execute(
        &self,
        run_id: &str,
        prepared: PreparedConnectorCall,
    ) -> Result<ConnectorExecution, DaemonError> {
        let key = adapter_process_key(run_id, &prepared.connector, &prepared.adapter)?;
        let process = {
            let mut processes = self.processes.lock().await;
            if let Some(process) = processes.get(&key) {
                process.clone()
            } else {
                let process = Arc::new(Mutex::new(
                    WarmConnectorAdapterProcess::spawn(&prepared.adapter).await?,
                ));
                processes.insert(key, process.clone());
                process
            }
        };
        let mut process = process.lock().await;
        let response = process.call(prepared.request.clone()).await?;
        adapter_response_to_execution(prepared, response)
    }

    pub async fn shutdown_run(&self, run_id: &str) {
        let entries = {
            let mut processes = self.processes.lock().await;
            let prefix = format!("{run_id}:");
            let keys = processes
                .keys()
                .filter(|key| key.starts_with(&prefix))
                .cloned()
                .collect::<Vec<_>>();
            keys.into_iter()
                .filter_map(|key| processes.remove(&key))
                .collect::<Vec<_>>()
        };
        for process in entries {
            let mut process = process.lock().await;
            let _ = process.shutdown().await;
        }
    }
}

impl WarmConnectorAdapterProcess {
    async fn spawn(adapter: &CharioxConnectorAdapterDefinition) -> Result<Self, DaemonError> {
        let command = adapter.resolved_command()?;
        let mut child = tokio::process::Command::new(&command)
            .args(&adapter.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                connector_error(
                    "connector.adapter.spawn",
                    format!(
                        "failed to launch adapter `{}` with `{}`: {error}",
                        adapter.name,
                        command.display()
                    ),
                )
            })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            connector_error(
                "connector.adapter.spawn",
                format!("adapter `{}` did not expose stdin", adapter.name),
            )
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            connector_error(
                "connector.adapter.spawn",
                format!("adapter `{}` did not expose stdout", adapter.name),
            )
        })?;
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout).lines(),
            sequence: 0,
        })
    }

    async fn call(
        &mut self,
        mut request: ConnectorAdapterRequest,
    ) -> Result<ConnectorAdapterResponse, DaemonError> {
        self.sequence += 1;
        request.id = format!("call-{}", self.sequence);
        let timeout = Duration::from_millis(request.timeout_ms);
        let max_response_bytes = request.max_response_bytes as usize;
        let payload = serde_json::to_string(&request).map_err(|error| {
            connector_error(
                "connector.adapter.call",
                format!("failed to encode call: {error}"),
            )
        })?;
        self.stdin
            .write_all(payload.as_bytes())
            .await
            .map_err(io_error("connector.adapter.call"))?;
        self.stdin
            .write_all(b"\n")
            .await
            .map_err(io_error("connector.adapter.call"))?;
        self.stdin
            .flush()
            .await
            .map_err(io_error("connector.adapter.call"))?;
        let line = tokio::time::timeout(timeout, self.stdout.next_line())
            .await
            .map_err(|_| {
                connector_error(
                    "connector.adapter.call",
                    format!("adapter call timed out after {}ms", request.timeout_ms),
                )
            })?
            .map_err(io_error("connector.adapter.call"))?
            .ok_or_else(|| {
                connector_error(
                    "connector.adapter.call",
                    "adapter exited without a response".to_string(),
                )
            })?;
        if line.len() > max_response_bytes {
            return Err(connector_error(
                "connector.adapter.call",
                format!(
                    "adapter response exceeded {} bytes",
                    request.max_response_bytes
                ),
            ));
        }
        let response =
            serde_json::from_str::<ConnectorAdapterResponse>(&line).map_err(|error| {
                connector_error(
                    "connector.adapter.call",
                    format!("adapter returned invalid JSON: {error}"),
                )
            })?;
        if response.id != request.id {
            return Err(connector_error(
                "connector.adapter.call",
                format!(
                    "adapter response id `{}` did not match request id `{}`",
                    response.id, request.id
                ),
            ));
        }
        Ok(response)
    }

    async fn shutdown(&mut self) -> Result<(), DaemonError> {
        self.sequence += 1;
        let request = ConnectorAdapterRequest {
            id: format!("shutdown-{}", self.sequence),
            request_type: ConnectorAdapterRequestType::Shutdown,
            connector: String::new(),
            operation: None,
            arguments: None,
            config: None,
            operations: Vec::new(),
            credential: None,
            timeout_ms: 1000,
            max_response_bytes: 4096,
        };
        if let Ok(payload) = serde_json::to_string(&request) {
            let _ = self.stdin.write_all(payload.as_bytes()).await;
            let _ = self.stdin.write_all(b"\n").await;
            let _ = self.stdin.flush().await;
        }
        let _ = tokio::time::timeout(Duration::from_millis(1000), self.child.wait()).await;
        if self.child.id().is_some() {
            let _ = self.child.kill().await;
        }
        Ok(())
    }
}
