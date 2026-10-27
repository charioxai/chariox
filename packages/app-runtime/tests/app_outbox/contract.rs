use super::*;

#[test]
fn schedule_revision_wire_shape_distinguishes_omission_from_null() {
    let mut value = serde_json::to_value(occurrence("wire", "payload")).unwrap();
    assert!(value.get("scheduleRevision").is_none());
    assert!(serde_json::from_value::<Occurrence>(value.clone()).is_ok());
    value["scheduleRevision"] = json!(null);
    assert!(serde_json::from_value::<Occurrence>(value.clone()).is_err());
    value["scheduleRevision"] = json!(7);
    assert!(serde_json::from_value::<Occurrence>(value.clone()).is_err());
    value["scheduleRevision"] = json!("revision-7");
    let decoded: Occurrence = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(decoded.schedule_revision.as_deref(), Some("revision-7"));
    assert_eq!(serde_json::to_value(decoded).unwrap(), value);
}
