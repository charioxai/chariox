use super::*;
use serde::ser::SerializeStruct;
use std::sync::atomic::AtomicUsize;

#[derive(Debug, Clone, Deserialize)]
struct Counted {
    text: String,
    #[serde(skip)]
    calls: Arc<AtomicUsize>,
}
impl Serialize for Counted {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut state = serializer.serialize_struct("Counted", 1)?;
        state.serialize_field("text", &self.text)?;
        state.end()
    }
}
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn a_serialized_event_is_not_encoded_again() {
    let root = Scratch(std::env::temp_dir().join(format!(
        "chariox-serialized-event-{}-{}",
        std::process::id(),
        unix_epoch_ms()
    )));
    fs::create_dir_all(&root.0).unwrap();
    let path = root.0.join("events.jsonl");
    let log = EventLog::new_with_persistent_event_store(8, root.0.join("counter"), &path).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let event = SerializedEvent::new(Counted {
        text: "queue text".repeat(4096),
        calls: calls.clone(),
    })
    .unwrap();
    let logged = log.append_serialized("room", &event).await.unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "delivery and replay must share one encoding"
    );
    let persisted: LoggedEvent<Counted> =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(persisted.event_id, logged.event_id);
    assert_eq!(persisted.stream_seq, logged.stream_seq);
    assert_eq!(persisted.event.text, event.event().text);
}
