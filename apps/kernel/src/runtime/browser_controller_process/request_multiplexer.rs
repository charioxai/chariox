use super::{
    cancellation::CancellationSignal, kill_child, BrowserControllerRpcRequest,
    BrowserControllerRpcResponse,
};
use serde::Serialize;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

const MAX_IN_FLIGHT_CONTROLLER_REQUESTS: usize = 128;

type PendingResponse = Result<BrowserControllerRpcResponse, String>;

fn request_exit_error(method: &str, error: String) -> String {
    if error == "browser controller exited during request" {
        format!("browser controller exited during `{method}`")
    } else {
        error
    }
}

struct CancellationAckObserver {
    signal: Arc<CancellationSignal>,
    result: Mutex<Option<Result<(), String>>>,
}

impl CancellationAckObserver {
    fn new(signal: Arc<CancellationSignal>) -> Self {
        Self {
            signal,
            result: Mutex::new(None),
        }
    }

    fn observe(&self, response: &BrowserControllerRpcResponse) {
        let accepted = response.ok
            && response
                .result
                .as_ref()
                .and_then(|value| value.get("accepted"))
                .and_then(serde_json::Value::as_bool)
                == Some(true);
        if accepted {
            self.signal.confirm_stop();
        } else {
            self.signal.reject_after_stop();
        }
        *self
            .result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Ok(()));
    }

    fn fail(&self, error: String) {
        *self
            .result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Err(error));
    }

    fn result(&self) -> Option<Result<(), String>> {
        self.result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

struct PendingResponseRoute {
    sender: mpsc::Sender<PendingResponse>,
    cancellation_ack: Option<Arc<CancellationAckObserver>>,
}

struct PendingState {
    requests: HashMap<u64, PendingResponseRoute>,
    closing: bool,
    closed: Option<String>,
}

#[derive(Default)]
struct ResponseRouter {
    state: Mutex<PendingState>,
}

impl Default for PendingState {
    fn default() -> Self {
        Self {
            requests: HashMap::new(),
            closing: false,
            closed: None,
        }
    }
}

impl ResponseRouter {
    fn register(
        &self,
        request_id: u64,
        response: mpsc::Sender<PendingResponse>,
        cancellation_ack: Option<Arc<CancellationAckObserver>>,
        allow_during_shutdown: bool,
    ) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "browser controller response router poisoned".to_string())?;
        if let Some(error) = &state.closed {
            return Err(error.clone());
        }
        if state.closing && !allow_during_shutdown {
            return Err("browser controller is shutting down".to_string());
        }
        if state.requests.len() >= MAX_IN_FLIGHT_CONTROLLER_REQUESTS {
            return Err("browser controller in-flight request capacity is exhausted".to_string());
        }
        state.requests.insert(
            request_id,
            PendingResponseRoute {
                sender: response,
                cancellation_ack,
            },
        );
        Ok(())
    }

    fn remove(&self, request_id: u64) {
        if let Ok(mut state) = self.state.lock() {
            state.requests.remove(&request_id);
        }
    }

    fn dispatch(&self, response: BrowserControllerRpcResponse) -> Result<(), String> {
        let request_id = response
            .id
            .ok_or_else(|| "browser controller response omitted its request ID".to_string())?;
        let sender = self
            .state
            .lock()
            .map_err(|_| "browser controller response router poisoned".to_string())?
            .requests
            .remove(&request_id);
        if let Some(route) = sender {
            if let Some(observer) = &route.cancellation_ack {
                observer.observe(&response);
            }
            let _ = route.sender.send(Ok(response));
        }
        // A response for an expired request is stale. Ignore it without
        // delivering it to another request that happens to be waiting.
        Ok(())
    }

    fn begin_shutdown(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closing = true;
        }
    }

    fn close(&self, error: String) {
        let requests = match self.state.lock() {
            Ok(mut state) => {
                if state.closed.is_some() {
                    return;
                }
                state.closed = Some(error.clone());
                std::mem::take(&mut state.requests)
            }
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                state.closed = Some(error.clone());
                std::mem::take(&mut state.requests)
            }
        };
        for (_, route) in requests {
            if let Some(observer) = &route.cancellation_ack {
                observer.fail(error.clone());
            }
            let _ = route.sender.send(Err(error.clone()));
        }
    }
}

struct RpcConnection {
    stdin: Mutex<ChildStdin>,
    child: Arc<Mutex<Child>>,
    router: Arc<ResponseRouter>,
    next_request_id: AtomicU64,
}

#[derive(Clone)]
pub(super) struct BrowserControllerRpcClient {
    connection: Arc<RpcConnection>,
}

struct PendingRequest {
    request_id: u64,
    response: mpsc::Receiver<PendingResponse>,
    router: Arc<ResponseRouter>,
}

impl Drop for PendingRequest {
    fn drop(&mut self) {
        self.router.remove(self.request_id);
    }
}

impl BrowserControllerRpcClient {
    pub(super) fn new(stdin: ChildStdin, child: Arc<Mutex<Child>>) -> Self {
        Self {
            connection: Arc::new(RpcConnection {
                stdin: Mutex::new(stdin),
                child,
                router: Arc::new(ResponseRouter::default()),
                next_request_id: AtomicU64::new(1),
            }),
        }
    }

    pub(super) fn read_responses(&self, stdout: ChildStdout) {
        for line in BufReader::new(stdout).lines() {
            let response = line
                .map_err(|error| format!("failed to read browser controller response: {error}"))
                .and_then(|line| {
                    serde_json::from_str::<BrowserControllerRpcResponse>(&line)
                        .map_err(|error| format!("browser controller returned invalid JSON: {error}"))
                });
            match response {
                Ok(response) => {
                    if let Err(error) = self.connection.router.dispatch(response) {
                        self.connection.router.close(error);
                        return;
                    }
                }
                Err(error) => {
                    self.connection.router.close(error);
                    return;
                }
            }
        }
        self.connection
            .router
            .close("browser controller exited during request".to_string());
    }

    pub(super) fn begin_shutdown(&self) {
        self.connection.router.begin_shutdown();
    }

    pub(super) fn close(&self, error: String) {
        self.connection.router.close(error);
    }

    pub(super) fn request<P: Serialize>(
        &self,
        method: &str,
        params: &P,
        timeout: Duration,
        cancellation: Option<Arc<CancellationSignal>>,
    ) -> Result<BrowserControllerRpcResponse, String> {
        if cancellation
            .as_ref()
            .is_some_and(|signal| signal.requested())
        {
            cancellation.as_ref().unwrap().confirm_stop();
            return Err("browser action cancelled before dispatch".into());
        }

        let mut terminal_response = None;
        let mut cancellation_request = None;
        let mut cancellation_ack: Option<Arc<CancellationAckObserver>> = None;
        let mut cancellation_acknowledged = false;
        let original_request = self.send_request(method, params, method == "shutdown", None)?;
        let request_id = original_request.request_id;
        let started = Instant::now();

        loop {
            if !cancellation_acknowledged {
                if let Some(acknowledgement) =
                    cancellation_ack.as_ref().and_then(|ack| ack.result())
                {
                    acknowledgement.map_err(|error| request_exit_error(method, error))?;
                    cancellation_acknowledged = true;
                    if let Some(response) = terminal_response.take() {
                        return Ok(response);
                    }
                }
            }

            if cancellation_request.is_none()
                && cancellation
                    .as_ref()
                    .is_some_and(|signal| signal.requested())
            {
                let acknowledgement = Arc::new(CancellationAckObserver::new(
                    Arc::clone(cancellation.as_ref().expect("cancellation was requested")),
                ));
                let cancel = self.send_request(
                    "browser.cancel",
                    &serde_json::json!({ "request_id": request_id }),
                    true,
                    Some(Arc::clone(&acknowledgement)),
                )?;
                cancellation_request = Some(cancel);
                cancellation_ack = Some(acknowledgement);
            }

            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                if let Some(signal) = cancellation.as_ref().filter(|signal| signal.requested()) {
                    let mut child = self
                        .connection
                        .child
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    kill_child(&mut child);
                    self.connection.router.close(
                        "browser controller was fenced after cancellation timed out".to_string(),
                    );
                    signal.confirm_fence();
                    return Ok(BrowserControllerRpcResponse {
                        id: Some(request_id),
                        ok: false,
                        result: None,
                        error: Some(super::BrowserControllerRpcError {
                            code: "browser_action_cancelled".to_string(),
                            message: "browser controller was fenced after cancellation timed out"
                                .to_string(),
                        }),
                    });
                }
                return Err(format!(
                    "browser controller `{method}` timed out after {}ms",
                    timeout.as_millis()
                ));
            }

            let poll = if cancellation.is_some() || cancellation_request.is_some() {
                remaining.min(Duration::from_millis(20))
            } else {
                remaining
            };
            match original_request.response.recv_timeout(poll) {
                Ok(response) => {
                    let response = response.map_err(|error| request_exit_error(method, error))?;
                    if !response.ok
                        && response.error.as_ref().is_some_and(|error| {
                            error.code == "browser_action_cancelled"
                        })
                    {
                        if let Some(signal) = &cancellation {
                            signal.confirm_stop();
                        }
                    }
                    if cancellation_request.is_some() && !cancellation_acknowledged {
                        terminal_response = Some(response);
                        continue;
                    }
                    return Ok(response);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(request_exit_error(
                        method,
                        "browser controller exited during request".to_string(),
                    ));
                }
            }
        }
    }

    fn send_request<P: Serialize>(
        &self,
        method: &str,
        params: &P,
        allow_during_shutdown: bool,
        cancellation_ack: Option<Arc<CancellationAckObserver>>,
    ) -> Result<PendingRequest, String> {
        let request_id = self
            .connection
            .next_request_id
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |next| next.checked_add(1))
            .map_err(|_| "browser controller request identity space is exhausted".to_string())?;
        let encoded = serde_json::to_vec(&BrowserControllerRpcRequest {
            id: request_id,
            method,
            params,
        })
        .map_err(|error| format!("failed to encode browser controller request: {error}"))?;
        let (sender, response) = mpsc::channel();
        self.connection
            .router
            .register(request_id, sender, cancellation_ack, allow_during_shutdown)?;
        let write_result = self
            .connection
            .stdin
            .lock()
            .map_err(|_| "browser controller request writer poisoned".to_string())
            .and_then(|mut stdin| {
                stdin
                    .write_all(&encoded)
                    .and_then(|()| stdin.write_all(b"\n"))
                    .and_then(|()| stdin.flush())
                    .map_err(|error| format!("failed to send browser controller `{method}`: {error}"))
            });
        if let Err(error) = write_result {
            self.connection.router.close(error.clone());
            return Err(error);
        }
        Ok(PendingRequest {
            request_id,
            response,
            router: Arc::clone(&self.connection.router),
        })
    }
}
