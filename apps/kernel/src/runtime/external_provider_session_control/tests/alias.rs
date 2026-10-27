use super::*;

#[test]
fn external_provider_import_session_alias_slugifies_prompt_titles() {
    let record = ExternalProviderSessionRecord {
        title: Some("There is supposed to be a service in the kernel".to_string()),
        provider_session_id: "019eea18-f755-7680-ab3f-31be1f79d4d0".to_string(),
        ..record(
            "codex",
            "019eea18-f755-7680-ab3f-31be1f79d4d0",
            "/tmp/external-one",
        )
    };

    assert_eq!(
        external_provider_import_session_alias(&record, None),
        "there-is-supposed-to-be-a-service-in-the-kernel-31be1f79d4d0"
    );
}
