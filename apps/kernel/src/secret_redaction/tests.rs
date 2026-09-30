use super::*;
use std::time::{Duration, Instant};

fn redacted(text: &str) -> String {
    redact_secrets(text).into_owned()
}

// Secret-shaped samples are assembled at run time, so the source holds no
// literal token for a secret scanner to flag.
fn alnum(len: usize) -> String {
    "a1B2c3D4e5".chars().cycle().take(len).collect()
}

fn jwt() -> String {
    format!("eyJ{}.eyJ{}.{}", alnum(33), alnum(40), alnum(43))
}

fn pem(label: &str) -> String {
    format!(
        "-----BEGIN {label}-----\n{}\n{}\n-----END {label}-----",
        "A".repeat(64),
        "B".repeat(20)
    )
}

#[test]
fn each_token_family_is_redacted_with_its_kind() {
    let cases = [
        (format!("sk-ant-api03-{}", alnum(40)), "anthropic-key"),
        (format!("sk-proj-{}", alnum(40)), "openai-key"),
        (format!("sk-{}", alnum(48)), "openai-key"),
        (format!("sk_live_{}", alnum(24)), "stripe-key"),
        (
            format!("xoxb-{}-{}", "1".repeat(12), alnum(24)),
            "slack-token",
        ),
        (format!("xoxp-{}", alnum(30)), "slack-token"),
        (format!("xapp-1-{}", alnum(30)), "slack-token"),
        (format!("ghp_{}", alnum(36)), "github-token"),
        (format!("gho_{}", alnum(36)), "github-token"),
        (format!("ghs_{}", alnum(36)), "github-token"),
        (
            format!("github_pat_{}_{}", alnum(22), alnum(59)),
            "github-token",
        ),
        (format!("AKIA{}", "IOSFODNN7EXAMPLE"), "aws-access-key"),
        (format!("ASIA{}", "Q3EGZ2XOOEXAMPLE"), "aws-access-key"),
        (format!("AIza{}", alnum(35)), "google-api-key"),
        (jwt(), "jwt"),
        // Unsigned (`alg: none`): the signature segment is empty.
        (format!("eyJ{}.eyJ{}.", alnum(20), alnum(30)), "jwt"),
        (pem("RSA PRIVATE KEY"), "private-key"),
        (pem("OPENSSH PRIVATE KEY"), "private-key"),
        (pem("PRIVATE KEY"), "private-key"),
    ];
    for (secret, kind) in cases {
        let marker = format!("[redacted:{kind}]");
        assert_eq!(redacted(&secret), marker, "{secret}");
        assert_eq!(
            redacted(&format!("calling with {secret} now")),
            format!("calling with {marker} now"),
        );
        assert_eq!(
            redacted(&format!("[\"{secret}\"],(x-{secret})")),
            format!("[\"{marker}\"],(x-{marker})"),
        );
        assert!(contains_secret(&secret));
    }
}

#[test]
fn secrets_named_by_their_context_are_redacted() {
    let cases = [
        (
            "Authorization: Bearer abc123def456".to_string(),
            "Authorization: Bearer [redacted:bearer-token]",
        ),
        (
            format!("sending bearer {} upstream", jwt()),
            "sending bearer [redacted:bearer-token] upstream",
        ),
        (
            "proxy-authorization=Basic dXNlcjpwYXNz".into(),
            "proxy-authorization=Basic [redacted:authorization]",
        ),
        (
            r#"{"authorization":"Bearer abc123 def"}"#.into(),
            r#"{"authorization":"Bearer [redacted:bearer-token]"}"#,
        ),
        (
            "password=hunter2 user=bob".into(),
            "password=[redacted:password] user=bob",
        ),
        (
            r#"{"password": "correct horse battery"}"#.into(),
            r#"{"password": "[redacted:password]"}"#,
        ),
        (
            r#"payload={\"token\":\"abc\"}"#.into(),
            r#"payload={\"token\":\"[redacted:token]\"}"#,
        ),
        (
            "GET /cb?access_token=abc123&state=ok".into(),
            "GET /cb?access_token=[redacted:token]&state=ok",
        ),
        (
            "clientSecret='s3cr3t'".into(),
            "clientSecret='[redacted:secret]'",
        ),
        (
            "secret = abc; next".into(),
            "secret = [redacted:secret]; next",
        ),
        (
            "x-api-key: 0123456789".into(),
            "x-api-key: [redacted:api-key]",
        ),
        (
            format!(
                "AWS_SECRET_ACCESS_KEY={}/K7MDENG/bPxRfiCYEXAMPLEKEY",
                "wJalrXUtnFEMI"
            ),
            "AWS_SECRET_ACCESS_KEY=[redacted:aws-secret-key]",
        ),
        (
            "--db-password=pa55 --verbose".into(),
            "--db-password=[redacted:password] --verbose",
        ),
        (
            "postgres://app:pa55word@db.internal:5432/app".into(),
            "postgres://app:[redacted:url-password]@db.internal:5432/app",
        ),
        (
            "redis://:pa55@cache".into(),
            "redis://:[redacted:url-password]@cache",
        ),
    ];
    for (text, expected) in cases {
        assert_eq!(redacted(&text), expected, "{text}");
    }
}

#[test]
fn a_private_key_block_without_its_end_line_is_redacted_to_the_end() {
    let text = format!("loaded -----BEGIN EC PRIVATE KEY-----\n{}", "C".repeat(70));
    assert_eq!(redacted(&text), "loaded [redacted:private-key]");
}

#[test]
fn ordinary_text_ids_and_hashes_are_kept() {
    let certificate = pem("CERTIFICATE");
    for text in [
        "Synced 42 todos in 180 ms; next sync at 12:00:05",
        "user 3f2b9c1e-8d4a-4b6f-9e2d-7a1c5b3d9e0f opened task-list-2024",
        "sha256 9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
        "commit da39a3ee5e6b4b0d3255bfef95601890afd80709",
        "blob aGVsbG8gd29ybGQgdGhpcyBpcyBiYXNlNjQgZGF0YQ==",
        "installed com.chariox.drill-misbehaving 1.0.1 as app_01HZX3K9Q2",
        "uses sk-learn and ask-me-anything; disk-sk-01 is fine",
        "the token expired; password: missing; secret: none; token=",
        "max_tokens=4096 token_count=12 password_hash=ab12 secretary=bob",
        "has_password=true api_key=null",
        "Bearer authentication is required",
        "see https://docs.example.com/p?q=1#frag or mailto:someone@example.com",
        "secret::Vault::open(token://local)",
        "ASIAPACIFIC region, AKIA short",
        "eyJhbGciOiJIUzI1NiJ9 alone is a header, not a token",
        "caf\u{e9} \u{1F600} password",
        certificate.as_str(),
        "",
    ] {
        assert!(
            matches!(redact_secrets(text), Cow::Borrowed(_)),
            "{text} became {}",
            redacted(text)
        );
    }
}

#[test]
fn redaction_is_idempotent_and_keeps_the_surrounding_text() {
    let text = format!(
        "h\u{e9}llo password=x1 ghp_{} \u{1F600} https://u:p4ss@h/x Authorization: Bearer tok3n-value {}",
        alnum(36),
        pem("PRIVATE KEY"),
    );
    let once = redacted(&text);
    assert_eq!(
        once,
        "h\u{e9}llo password=[redacted:password] [redacted:github-token] \u{1F600} \
         https://u:[redacted:url-password]@h/x Authorization: Bearer [redacted:bearer-token] \
         [redacted:private-key]"
    );
    assert!(matches!(redact_secrets(&once), Cow::Borrowed(_)));
}

#[test]
fn json_fields_redact_strings_keys_and_secret_named_values() {
    let slack = format!("xoxb-{}", alnum(24));
    let mut fields = serde_json::json!({
        "password": "hunter2",
        "token": 12345,
        "has_password": true,
        "api_key": null,
        "session_token": "expired",
        "retries": 3,
        "note": format!("sent with {slack}"),
        "nested": [{"client_secret": "abc"}, "Bearer abc123def456"],
    });
    fields
        .as_object_mut()
        .unwrap()
        .insert(slack, "value".into());
    redact_json_secrets(&mut fields);
    assert_eq!(
        fields,
        serde_json::json!({
            "password": "[redacted:password]",
            "token": "[redacted:token]",
            "has_password": true,
            "api_key": null,
            "session_token": "expired",
            "retries": 3,
            "note": "sent with [redacted:slack-token]",
            "nested": [{"client_secret": "[redacted:secret]"}, "Bearer [redacted:bearer-token]"],
            "[redacted:slack-token]": "value",
        })
    );
}

#[test]
fn hostile_input_is_scanned_in_linear_time() {
    // Each input has many candidate starts that fail late, which makes a
    // backtracking or retrying scanner quadratic: minutes at this size.
    let size = 256 * 1024;
    let hostile = [
        "a".repeat(size),
        "sk-".repeat(size / 3),
        "x-".repeat(size / 2),
        "eyJaaaaaaaaaa-".repeat(size / 14),
        "eyJaaaaaaaaaa.".repeat(size / 14),
        "-----BEGIN CERTIFICATE ".repeat(size / 23),
        "password:\"".repeat(size / 10),
        "password ".repeat(size / 9),
        "Authorization: Bearer ".repeat(size / 22),
        "Bearer ".repeat(size / 7),
        "://a:".repeat(size / 5),
        "\\\"token\\\":\\\"".repeat(size / 12),
        format!("password={}", "=".repeat(size)),
    ];
    for text in &hostile {
        let started = Instant::now();
        let _ = redact_secrets(text);
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(1),
            "{} bytes of {:?} took {elapsed:?}",
            text.len(),
            &text[..12]
        );
    }
}
