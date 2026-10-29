//! A local stand-in for the OpenCode app server's HTTP surface, so tests drive
//! the real client against a provider that answers a prompt as they choose.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// What the server answers to `POST /session/{id}/prompt_async`.
#[derive(Clone)]
pub(crate) enum PromptReply {
    Status {
        code: u16,
        body: &'static str,
    },
    /// Closes the connection without a response.
    Dropped,
}

pub(crate) struct FakeOpenCodeServer {
    pub(crate) endpoint: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FakeOpenCodeServer {
    pub(crate) fn start(prompt_reply: PromptReply) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake OpenCode server");
        listener
            .set_nonblocking(true)
            .expect("fake OpenCode server must not block");
        let endpoint = format!("http://{}", listener.local_addr().expect("server address"));
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let thread = std::thread::spawn({
            let stop = Arc::clone(&stop);
            let requests = Arc::clone(&requests);
            move || {
                while !stop.load(Ordering::SeqCst) {
                    let Ok((mut stream, _)) = listener.accept() else {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    };
                    stream.set_nonblocking(false).expect("blocking stream");
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .expect("read timeout");
                    let Some(request_line) = read_request_line(&mut stream) else {
                        continue;
                    };
                    requests
                        .lock()
                        .expect("request log")
                        .push(request_line.clone());
                    let (code, body) = if request_line.contains("/prompt_async ") {
                        match &prompt_reply {
                            PromptReply::Status { code, body } => (*code, *body),
                            PromptReply::Dropped => continue,
                        }
                    } else if request_line.starts_with("GET ") {
                        (200, "[]")
                    } else {
                        (200, "{}")
                    };
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {code} Reply\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                }
            }
        });
        Self {
            endpoint,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    /// The request line of every request received so far.
    pub(crate) fn requests(&self) -> Vec<String> {
        self.requests.lock().expect("request log").clone()
    }
}

impl Drop for FakeOpenCodeServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("fake OpenCode server thread");
        }
    }
}

/// Reads one whole request and returns its request line.
fn read_request_line(stream: &mut std::net::TcpStream) -> Option<String> {
    let mut request = Vec::new();
    loop {
        let mut buffer = [0; 4096];
        let size = stream.read(&mut buffer).ok().filter(|size| *size > 0)?;
        request.extend_from_slice(&buffer[..size]);
        let text = String::from_utf8_lossy(&request);
        let Some((headers, body)) = text.split_once("\r\n\r\n") else {
            continue;
        };
        let length = headers
            .lines()
            .find_map(|line| {
                line.to_lowercase()
                    .strip_prefix("content-length: ")
                    .and_then(|value| value.parse::<usize>().ok())
            })
            .unwrap_or(0);
        if body.len() >= length {
            return text.lines().next().map(str::to_string);
        }
    }
}
