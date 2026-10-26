//! OpenCode SSE event-stream subscription lifecycle.

use std::io::{BufRead, BufReader, Cursor, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::error::DaemonError;

use super::events::parse_sse_event;
use super::http::read_http_headers;
use super::{OpenCodeClient, OpenCodeEvent};

#[derive(Debug)]
pub struct OpenCodeEventSubscription {
    pub receiver: Receiver<OpenCodeEvent>,
    stop: Arc<AtomicBool>,
    pub(crate) output_notifier: crate::provider::output_ready::ProviderEventNotifier,
}

impl OpenCodeEventSubscription {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn for_tests(receiver: Receiver<OpenCodeEvent>) -> Self {
        Self {
            receiver,
            output_notifier: Default::default(),
            stop: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl OpenCodeClient {
    pub fn subscribe_events(&self) -> Result<OpenCodeEventSubscription, DaemonError> {
        let address = self.base_url.strip_prefix("http://").ok_or_else(|| {
            self.protocol_error(
                "base_url_parse",
                format!("unsupported OpenCode base URL `{}`", self.base_url),
            )
        })?;
        let mut stream = TcpStream::connect(address)
            .map_err(|error| self.protocol_error("event_subscribe", error.to_string()))?;
        stream
            .set_read_timeout(Some(Duration::from_millis(500)))
            .map_err(|error| self.protocol_error("event_subscribe", error.to_string()))?;
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(|error| self.protocol_error("event_subscribe", error.to_string()))?;

        let request = format!(
            "GET /event HTTP/1.1\r\nHost: {address}\r\nAccept: text/event-stream\r\nX-Chariox-Provider-Client: kernel\r\nConnection: close\r\n\r\n"
        );
        stream
            .write_all(request.as_bytes())
            .and_then(|_| stream.flush())
            .map_err(|error| self.protocol_error("event_subscribe", error.to_string()))?;
        let (status_code, buffered_body) = read_http_headers(&mut stream)
            .map_err(|error| self.protocol_error("event_subscribe", error))?;
        if status_code >= 400 {
            return Err(self.protocol_error(
                "event_subscribe",
                format!("OpenCode returned HTTP {status_code}"),
            ));
        }

        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = stop.clone();
        let provider_run_id = self.provider_run_id.clone();

        let output_notifier = crate::provider::output_ready::ProviderEventNotifier::default();
        let notifier = output_notifier.clone();
        thread::spawn(move || {
            let mut reader = BufReader::new(Cursor::new(buffered_body).chain(stream));
            let mut data_lines = Vec::new();
            let mut last_text_message = String::new();
            let mut last_part_message = String::new();
            let mut text_part_ids = std::collections::BTreeSet::new();

            loop {
                if stop_for_thread.load(Ordering::SeqCst) {
                    break;
                }

                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        let line = line.trim_end_matches(['\r', '\n']);
                        if line.is_empty() {
                            if data_lines.is_empty() {
                                continue;
                            }

                            let payload = data_lines.join("\n");
                            data_lines.clear();
                            if let Some(event) = parse_sse_event(&payload, &provider_run_id) {
                                // MP-08 / MP-10: reasoning parts also use a `text`
                                // delta field. Classify from native part metadata,
                                // with bounded diagnostic state and no content.
                                if let OpenCodeEvent::MessagePartUpdated { part } = &event {
                                    if part.kind == "text" {
                                        text_part_ids.insert(part.id.clone());
                                        if text_part_ids.len() > 128 {
                                            text_part_ids.pop_first();
                                        }
                                    } else {
                                        text_part_ids.remove(&part.id);
                                    }
                                }
                                // MP-08 / MP-10: first-part/first-text timestamps at
                                // the reader, before the output pump. No content.
                                let first = match &event {
                                    OpenCodeEvent::MessagePartUpdated { part }
                                        if part.message_id != last_part_message =>
                                    {
                                        last_part_message.clone_from(&part.message_id);
                                        Some(("first part received", &part.message_id, &part.id))
                                    }
                                    OpenCodeEvent::MessagePartDelta {
                                        message_id,
                                        part_id,
                                        delta,
                                        field,
                                        ..
                                    } if field == "text"
                                        && text_part_ids.contains(part_id)
                                        && !delta.is_empty()
                                        && *message_id != last_text_message =>
                                    {
                                        last_text_message.clone_from(message_id);
                                        Some(("first text delta received", message_id, part_id))
                                    }
                                    _ => None,
                                };
                                if let Some((stage, message_id, part_id)) = first {
                                    crate::logging::debug_with_fields(
                                        "daemon.provider.opencode.timing",
                                        stage,
                                        serde_json::json!({"provider_run_id": provider_run_id,
                                            "message_id": message_id, "part_id": part_id}),
                                    );
                                }
                                if notifier.send(&tx, event).is_err() {
                                    break;
                                }
                            }
                            continue;
                        }

                        if let Some(data) = line.strip_prefix("data:") {
                            data_lines.push(data.trim_start().to_string());
                        }
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) => {}
                    Err(_) => break,
                }
            }
        });

        Ok(OpenCodeEventSubscription {
            receiver: rx,
            stop,
            output_notifier,
        })
    }

    pub fn subscribe_events_with_retry(
        &self,
        timeout: Duration,
        retry_interval: Duration,
    ) -> Result<OpenCodeEventSubscription, DaemonError> {
        let deadline = Instant::now() + timeout;
        let mut last_error = None;

        loop {
            match self.subscribe_events() {
                Ok(subscription) => return Ok(subscription),
                Err(error) if Instant::now() < deadline => {
                    last_error = Some(error);
                    std::thread::sleep(retry_interval);
                }
                Err(error) => return Err(last_error.unwrap_or(error)),
            }
        }
    }
}
