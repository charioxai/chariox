//! Real registration boundary, including the refusal log and responder store.
use super::*;

#[derive(Clone, Default)]
struct RefusalLog(Arc<std::sync::Mutex<Vec<String>>>);
impl tracing::Subscriber for RefusalLog {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        struct Fields(String);
        impl tracing::field::Visit for Fields {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                self.0.push_str(&format!("{field}={value:?} "));
            }
        }
        let mut fields = Fields(String::new());
        event.record(&mut fields);
        self.0.lock().unwrap().push(fields.0);
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

#[tokio::test]
async fn taxonomy_unknown_type_is_refused_before_registering_a_responder_or_card() {
    let fixture = Fixture::new();
    let log = RefusalLog::default();
    let error = tracing::subscriber::with_default(log.clone(), || {
        fixture
            .register_type("unknown", "synthetic-unregistered-test-type")
            .unwrap_err()
    });
    assert!(error.to_string().contains("not registered"));
    let events = log.0.lock().unwrap().join("\n");
    assert!(events.contains("card refused: unregistered type"));
    assert!(events.contains("synthetic-unregistered-test-type"));
    assert!(fixture.active_ids().is_empty());
    assert!(!fixture
        .state
        .owned
        .pending_interactions
        .write()
        .contains_key(&fixture.id("unknown")));
    // The same real entry accepts an individually approved type and keeps its responder.
    let mut receiver = fixture.register("approved").unwrap();
    assert_eq!(fixture.active_ids(), vec![fixture.id("approved")]);
    assert!(matches!(
        receiver.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
}
