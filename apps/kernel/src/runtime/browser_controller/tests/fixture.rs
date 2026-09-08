use super::*;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::{io::DuplexStream, sync::Notify};
use tokio_tungstenite::{
    tungstenite::{protocol::Role, Message},
    WebSocketStream,
};

#[derive(Clone, Copy)]
pub(super) enum Mode {
    Normal,
    WrongTarget,
    NavigateOnInput,
    HoldInput,
}

pub(super) struct Peer {
    pub(super) methods: Arc<Mutex<Vec<String>>>,
    pub(super) inputs: Arc<AtomicUsize>,
    pub(super) release: Arc<Notify>,
    input_seen: Arc<Notify>,
    frames: mpsc::Sender<u8>,
    task: Mutex<Option<JoinHandle<()>>>,
}
impl Peer {
    pub(super) async fn spawn(mode: Mode) -> (WebSocketStream<DuplexStream>, Self) {
        let (a, b) = tokio::io::duplex(65536);
        let config = || {
            WebSocketConfig::default()
                .max_message_size(Some(MAX_WIRE_BYTES))
                .max_frame_size(Some(MAX_WIRE_BYTES))
        };
        let client = WebSocketStream::from_raw_socket(a, Role::Client, Some(config())).await;
        let mut server = WebSocketStream::from_raw_socket(b, Role::Server, Some(config())).await;
        let methods = Arc::new(Mutex::new(Vec::new()));
        let inputs = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Notify::new());
        let input_seen = Arc::new(Notify::new());
        let (frames, mut frame_rx) = mpsc::channel::<u8>(2);
        let (seen, count, gate, notice) = (
            methods.clone(),
            inputs.clone(),
            release.clone(),
            input_seen.clone(),
        );
        let task = tokio::spawn(async move {
            let mut loader = "loader-1";
            loop {
                let request = tokio::select! {
                    frame = frame_rx.recv() => {
                        let Some(frame) = frame else { break };
                        let data = base64::engine::general_purpose::STANDARD.encode([0xff,0xd8,frame,0xd9]);
                        if send(&mut server, json!({"method":"Page.screencastFrame","params":{"sessionId":frame,"data":data,"metadata":{"deviceWidth":800,"deviceHeight":600}}})).await.is_err() { break }
                        continue;
                    },
                    next = server.next() => match next {
                        Some(Ok(Message::Text(text))) => serde_json::from_str::<Value>(&text).unwrap(),
                        _ => break,
                    },
                };
                let method = request["method"].as_str().unwrap();
                {
                    let mut methods = seen.lock().unwrap();
                    assert!(methods.len() < 128, "unexpected unbounded retry");
                    methods.push(method.to_owned());
                }
                let result = match method {
                    "Target.getTargetInfo" => {
                        assert_eq!(request["params"]["targetId"], "chosen-target");
                        json!({"targetInfo":{"targetId":if matches!(mode, Mode::WrongTarget) {"other-target"} else {"chosen-target"},"type":"page"}})
                    }
                    "Page.getFrameTree" => {
                        json!({"frameTree":{"frame":{"id":"main-frame","loaderId":loader}}})
                    }
                    "Accessibility.getFullAXTree" => {
                        json!({"nodes":[{"nodeId":"1","role":{"value":"RootWebArea"},"name":{"value":"Chosen App"}}]})
                    }
                    method if method.starts_with("Input.") => {
                        count.fetch_add(1, Ordering::Release);
                        notice.notify_one();
                        if matches!(mode, Mode::HoldInput) {
                            gate.notified().await;
                        }
                        if matches!(mode, Mode::NavigateOnInput) {
                            loader = "loader-2";
                            if send(&mut server, json!({"method":"Page.frameNavigated","params":{"frame":{"id":"main-frame","loaderId":loader}}})).await.is_err() { break }
                        }
                        json!({})
                    }
                    "Page.enable"
                    | "Accessibility.enable"
                    | "Page.startScreencast"
                    | "Page.screencastFrameAck" => json!({}),
                    _ => panic!("unexpected CDP method {method}"),
                };
                if send(&mut server, json!({"id":request["id"],"result":result}))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        (
            client,
            Self {
                methods,
                inputs,
                release,
                input_seen,
                frames,
                task: Mutex::new(Some(task)),
            },
        )
    }
    pub(super) async fn wait_for_input(&self) {
        tokio::time::timeout(Duration::from_secs(2), self.input_seen.notified())
            .await
            .unwrap();
    }
    pub(super) async fn frame(&self, value: u8) {
        self.frames.send(value).await.unwrap();
    }
    pub(super) async fn finish(&self) {
        let task = self.task.lock().unwrap().take().unwrap();
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        if let Some(task) = self.task.lock().unwrap().take() {
            task.abort();
        }
    }
}
async fn send(socket: &mut WebSocketStream<DuplexStream>, value: Value) -> Result<(), ()> {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .map_err(|_| ())
}
