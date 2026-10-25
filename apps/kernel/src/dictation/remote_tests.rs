//! MP-08 / MP-10 / MP-11: the real service entry point, Vault lookup and HTTP adapter.
use super::*;
use crate::config::{CredentialVaultBackend, UserCredentialVaultConfig};
use crate::local::DictationProvider;

fn fake_service(status: u16, body: &'static str) -> (String, std::thread::JoinHandle<Vec<u8>>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/transcribe", listener.local_addr().unwrap());
    let task = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut head = Vec::new();
        let mut byte = [0];
        while !head.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            head.push(byte[0]);
        }
        let size = String::from_utf8_lossy(&head)
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(|value| value.trim().parse::<usize>().unwrap())
            })
            .unwrap();
        let mut bytes = vec![0; size];
        stream.read_exact(&mut bytes).unwrap();
        write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        head.extend(bytes);
        head
    });
    (url, task)
}

fn service(name: &str, url: String) -> (PathBuf, Arc<DictationService>, UserCredentialVaultConfig) {
    let dir = scratch(name);
    let mut service = DictationService::with_parts(
        dir.clone(),
        spec_for("unused".into(), MODEL_BYTES),
        Arc::new(ScriptedEngine {
            heard: Mutex::default(),
            text: "must not use whisper".into(),
            fail: true,
            released: AtomicUsize::new(0),
        }),
    );
    Arc::get_mut(&mut service).unwrap().services = remote::SpeechServices::for_test(url);
    let vault = UserCredentialVaultConfig {
        backend: CredentialVaultBackend::CharioxEncrypted,
        service: format!("dictation-r4-{name}"),
        path: dir.join("vault.json").display().to_string(),
        ..Default::default()
    };
    std::fs::create_dir_all(&dir).unwrap();
    crate::secret::create_chariox_encrypted_vault_for_test(
        Path::new(&vault.path),
        "synthetic-passphrase",
    )
    .unwrap();
    crate::secret::unlock_chariox_encrypted_vault(
        &vault.path,
        "synthetic-passphrase",
        crate::secret::VaultUnlockLease::KernelShutdown,
    )
    .unwrap();
    (dir, service, vault)
}

async fn choose(service: &Arc<DictationService>, provider: DictationProvider) {
    service
        .handle(OWNER, DictationCommand::SetService { service: provider })
        .await
        .unwrap();
}

async fn speech(service: &Arc<DictationService>, owner: &str) {
    use base64::Engine as _;
    service
        .handle(
            owner,
            DictationCommand::AppendAudio {
                upload_id: "speech".into(),
                offset: 0,
                pcm_base64: base64::engine::general_purpose::STANDARD
                    .encode(vec![1u8; MIN_SPEECH_BYTES]),
            },
        )
        .await
        .unwrap();
}

async fn transcribe(
    service: &Arc<DictationService>,
    owner: &str,
    vault: &UserCredentialVaultConfig,
) -> Result<DictationResult, DaemonError> {
    service
        .handle_with_vault(
            owner,
            DictationCommand::Transcribe {
                context: None,
                upload_id: "speech".into(),
            },
            Some(vault.clone()),
        )
        .await
}

fn store(vault: &UserCredentialVaultConfig, owner: &str, provider: DictationProvider, value: &str) {
    crate::secret::RuntimeSecretService::with_vault_config(Vec::new(), vault)
        .unwrap()
        .set_vault_secret(&vault_key(owner, provider).unwrap(), value)
        .unwrap();
}

#[tokio::test]
async fn remote_speech_uses_documented_requests_and_needs_no_local_model() {
    for provider in [DictationProvider::Openai, DictationProvider::Elevenlabs] {
        let (url, request) =
            fake_service(200, r#"{"text":"hello from speech","language_code":"en"}"#);
        let (dir, service, vault) = service(&format!("remote-{provider:?}"), url);
        choose(&service, provider).await;
        service
            .handle(
                OWNER,
                DictationCommand::SetLanguage {
                    language: Some("es".into()),
                },
            )
            .await
            .unwrap();
        store(&vault, OWNER, provider, "synthetic-speech-key");
        speech(&service, OWNER).await;
        assert!(
            matches!(transcribe(&service, OWNER, &vault).await.unwrap(), DictationResult::Transcript { text, .. } if text == "hello from speech")
        );
        let bytes = request.join().unwrap();
        let request = String::from_utf8_lossy(&bytes);
        assert!(request.contains("Content-Type: multipart/form-data;"));
        assert!(request.contains("filename=\"dictation.wav\""));
        assert!(request.contains("Content-Type: audio/wav"));
        let wav = bytes.windows(4).position(|part| part == b"RIFF").unwrap();
        assert_eq!(&bytes[wav + 8..wav + 12], b"WAVE");
        assert_eq!(&bytes[wav + 24..wav + 28], &16_000u32.to_le_bytes());
        assert_eq!(
            &bytes[wav + 44..wav + 44 + MIN_SPEECH_BYTES],
            vec![1u8; MIN_SPEECH_BYTES]
        );
        if provider == DictationProvider::Openai {
            assert!(request.contains("Authorization: Bearer synthetic-speech-key\r\n"));
            assert!(request.contains("name=\"model\"\r\n\r\ngpt-transcribe\r\n"));
            assert!(request.contains("name=\"language\"\r\n\r\nes\r\n"));
        } else {
            assert!(request.contains("xi-api-key: synthetic-speech-key\r\n"));
            assert!(request.contains("name=\"model_id\"\r\n\r\nscribe_v2\r\n"));
            assert!(request.contains("name=\"language_code\"\r\n\r\nes\r\n"));
            assert!(request.contains("name=\"tag_audio_events\"\r\n\r\nfalse\r\n"));
        }
        let saved = std::fs::read_to_string(dir.join("settings.json")).unwrap();
        assert!(!saved.contains("synthetic-speech-key"));
        let status = serde_json::to_string(&service.status(OWNER)).unwrap();
        assert!(!status.contains("synthetic-speech-key"));
        let restarted = DictationService::new(dir.clone());
        assert_eq!(restarted.status(OWNER).service, provider);
        assert_ne!(
            restarted.status(OWNER).vault_key,
            restarted.status("other-user").vault_key
        );
        crate::secret::lock_chariox_encrypted_vault(&vault.path).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[tokio::test]
async fn remote_speech_errors_are_typed_and_never_echo_provider_bodies() {
    for provider in [DictationProvider::Openai, DictationProvider::Elevenlabs] {
        for (status, body, expected) in [
            (
                401,
                r#"{"error":{"message":"synthetic-secret-echo"}}"#,
                "dictation_bad_key",
            ),
            (
                403,
                r#"{"detail":{"status":"invalid_api_key","message":"synthetic-secret-echo"}}"#,
                "dictation_bad_key",
            ),
            (
                429,
                r#"{"error":{"message":"synthetic-secret-echo"}}"#,
                "dictation_quota",
            ),
            (
                401,
                r#"{"detail":{"status":"quota_exceeded","message":"synthetic-secret-echo"}}"#,
                "dictation_quota",
            ),
            (503, "synthetic-secret-echo", "dictation_network"),
            (
                200,
                "synthetic-secret-echo",
                "dictation_transcription_failed",
            ),
            (
                400,
                "synthetic-secret-echo",
                "dictation_transcription_failed",
            ),
        ] {
            let (url, request) = fake_service(status, body);
            let (dir, service, vault) =
                service(&format!("errors-{provider:?}-{status}-{expected}"), url);
            choose(&service, provider).await;
            store(&vault, OWNER, provider, "synthetic-secret-echo");
            speech(&service, OWNER).await;
            let error = transcribe(&service, OWNER, &vault).await.unwrap_err();
            assert!(
                matches!(&error, DaemonError::RelayTransport { code, .. } if code == expected),
                "{error}"
            );
            assert!(!format!("{error:?}").contains("synthetic-secret-echo"));
            request.join().unwrap();
            crate::secret::lock_chariox_encrypted_vault(&vault.path).unwrap();
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}

#[tokio::test]
async fn remote_speech_reads_only_its_owners_current_vault_entry() {
    for provider in [DictationProvider::Openai, DictationProvider::Elevenlabs] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        drop(listener);
        let (dir, service, vault) = service(&format!("key-{provider:?}"), url);
        choose(&service, provider).await;
        store(&vault, "other-user", provider, "synthetic-other-user-key");
        speech(&service, OWNER).await;
        assert!(
            matches!(transcribe(&service, OWNER, &vault).await.unwrap_err(), DaemonError::RelayTransport { code, .. } if code == "dictation_no_key")
        );
        store(&vault, OWNER, provider, "synthetic-first-key");
        speech(&service, OWNER).await;
        assert!(
            matches!(transcribe(&service, OWNER, &vault).await.unwrap_err(), DaemonError::RelayTransport { code, .. } if code == "dictation_network")
        );
        crate::secret::RuntimeSecretService::with_vault_config(Vec::new(), &vault)
            .unwrap()
            .delete_vault_secret(&vault_key(OWNER, provider).unwrap())
            .unwrap();
        speech(&service, OWNER).await;
        assert!(
            matches!(transcribe(&service, OWNER, &vault).await.unwrap_err(), DaemonError::RelayTransport { code, .. } if code == "dictation_no_key")
        );
        crate::secret::lock_chariox_encrypted_vault(&vault.path).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}
