use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use sha2::{Digest, Sha256};

use super::*;
use crate::local::DictationModelState;

const OWNER: &str = "user-a";
const FAILING: &str = "fail";
const MODEL_BYTES: &[u8] = b"pretend this is a speech model, long enough to resume in the middle";

struct ScriptedEngine {
    heard: Mutex<Vec<(usize, Option<String>)>>,
    text: String,
    fail: bool,
    released: AtomicUsize,
}

impl SpeechEngine for ScriptedEngine {
    fn transcribe(
        &self,
        model: &Path,
        samples: &[f32],
        language: Option<&str>,
    ) -> Result<Transcription, String> {
        assert!(model.exists(), "the engine only runs on an installed model");
        self.heard
            .lock()
            .unwrap()
            .push((samples.len(), language.map(str::to_string)));
        if self.fail {
            return Err("cannot start the speech model".to_string());
        }
        Ok(Transcription {
            text: self.text.clone(),
            language: language.unwrap_or("en").to_string(),
        })
    }
    fn release(&self) {
        self.released.fetch_add(1, Ordering::SeqCst);
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("chariox-dictation-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn spec_for(url: String, body: &[u8]) -> ModelSpec {
    ModelSpec {
        id: "test-model".to_string(),
        file_name: "model.bin".to_string(),
        url,
        sha256: sha256_hex(body),
        size_bytes: body.len() as u64,
    }
}

/// Serves `body`, honouring `Range` only when `ranges` is set. Returns the URL and the Range headers seen.
fn serve(
    body: &'static [u8],
    ranges: bool,
    requests: usize,
) -> (String, Arc<Mutex<Vec<Option<String>>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/model.bin", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for _ in 0..requests {
            let (mut stream, _) = listener.accept().unwrap();
            let mut head = Vec::new();
            let mut byte = [0_u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                head.push(byte[0]);
            }
            let head = String::from_utf8_lossy(&head).to_string();
            let range = head.lines().find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("range: bytes=")
                    .map(|value| value.trim_end_matches('-').to_string())
            });
            log.lock().unwrap().push(range.clone());
            let (status, start) = match (&range, ranges) {
                (Some(start), true) => ("206 Partial Content", start.parse::<usize>().unwrap()),
                _ => ("200 OK", 0),
            };
            let payload = &body[start..];
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                payload.len()
            );
            let _ = stream.write_all(payload);
        }
    });
    (url, seen)
}

fn service(
    dir: &Path,
    spec: ModelSpec,
    text: &str,
) -> (Arc<DictationService>, Arc<ScriptedEngine>) {
    let engine = Arc::new(ScriptedEngine {
        heard: Mutex::new(Vec::new()),
        text: text.to_string(),
        fail: text == FAILING,
        released: AtomicUsize::new(0),
    });
    (
        DictationService::with_parts(dir.to_path_buf(), spec, engine.clone()),
        engine,
    )
}

async fn settle(service: &Arc<DictationService>) -> DictationStatus {
    for _ in 0..200 {
        let status = service.status(OWNER);
        if !matches!(status.model, DictationModelState::Downloading { .. })
            && !matches!(service.lock().download, Download::Running(_))
        {
            return status;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("download did not settle");
}

fn pcm_base64(samples: usize) -> String {
    base64::engine::general_purpose::STANDARD.encode(vec![1_u8; samples * 2])
}

fn code(error: DaemonError) -> String {
    match error {
        DaemonError::RelayTransport { code, .. } => code,
        other => panic!("unexpected error {other}"),
    }
}

#[tokio::test]
async fn status_is_not_downloaded_until_the_model_is_installed() {
    let dir = scratch("status");
    let (service, _) = service(
        &dir,
        spec_for("http://127.0.0.1:9/model.bin".into(), MODEL_BYTES),
        "hi",
    );
    assert_eq!(
        service.status(OWNER).model,
        DictationModelState::NotDownloaded
    );
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("model.bin"), MODEL_BYTES).unwrap();
    assert_eq!(service.status(OWNER).model, DictationModelState::Ready);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn download_verifies_the_checksum_and_installs_the_model() {
    let dir = scratch("download");
    let (url, seen) = serve(MODEL_BYTES, true, 1);
    let (service, _) = service(&dir, spec_for(url, MODEL_BYTES), "hi");
    let started = service.start_download(OWNER);
    assert!(
        matches!(started.model, DictationModelState::Downloading { total_bytes, .. } if total_bytes == MODEL_BYTES.len() as u64)
    );
    assert_eq!(settle(&service).await.model, DictationModelState::Ready);
    assert_eq!(std::fs::read(dir.join("model.bin")).unwrap(), MODEL_BYTES);
    assert!(!dir.join("model.bin.part").exists());
    assert_eq!(*seen.lock().unwrap(), vec![None]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn download_resumes_a_partial_file_with_a_range_request() {
    let dir = scratch("resume");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("model.bin.part"), &MODEL_BYTES[..20]).unwrap();
    let (url, seen) = serve(MODEL_BYTES, true, 1);
    let (service, _) = service(&dir, spec_for(url, MODEL_BYTES), "hi");
    service.start_download(OWNER);
    assert_eq!(settle(&service).await.model, DictationModelState::Ready);
    assert_eq!(*seen.lock().unwrap(), vec![Some("20".to_string())]);
    assert_eq!(std::fs::read(dir.join("model.bin")).unwrap(), MODEL_BYTES);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn download_restarts_when_the_server_ignores_the_range() {
    let dir = scratch("norange");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("model.bin.part"), &MODEL_BYTES[..20]).unwrap();
    let (url, _) = serve(MODEL_BYTES, false, 1);
    let (service, _) = service(&dir, spec_for(url, MODEL_BYTES), "hi");
    service.start_download(OWNER);
    assert_eq!(settle(&service).await.model, DictationModelState::Ready);
    assert_eq!(std::fs::read(dir.join("model.bin")).unwrap(), MODEL_BYTES);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_corrupt_download_is_rejected_and_removed() {
    let dir = scratch("corrupt");
    let (url, _) = serve(MODEL_BYTES, true, 1);
    let mut spec = spec_for(url, MODEL_BYTES);
    spec.sha256 = sha256_hex(b"something else");
    let (service, _) = service(&dir, spec, "hi");
    service.start_download(OWNER);
    let status = settle(&service).await;
    assert!(
        matches!(&status.model, DictationModelState::Failed { reason } if reason.contains("checksum")),
        "{status:?}"
    );
    assert!(!dir.join("model.bin").exists() && !dir.join("model.bin.part").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn an_unreachable_server_fails_with_a_next_step_and_can_be_retried() {
    let dir = scratch("offline");
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/model.bin", closed.local_addr().unwrap());
    drop(closed);
    let (service, _) = service(&dir, spec_for(url, MODEL_BYTES), "hi");
    service.start_download(OWNER);
    let status = settle(&service).await;
    assert!(
        matches!(&status.model, DictationModelState::Failed { reason } if reason.contains("try again")),
        "{status:?}"
    );
    assert!(matches!(
        service.start_download(OWNER).model,
        DictationModelState::Downloading { .. }
    ));
    settle(&service).await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn dictating_without_the_model_says_to_download_it() {
    let dir = scratch("missing");
    let (service, engine) = service(
        &dir,
        spec_for("http://127.0.0.1:9/m".into(), MODEL_BYTES),
        "hi",
    );
    let error = service.transcribe(OWNER, "rec-1").await.err().unwrap();
    assert_eq!(code(error), "dictation_model_missing");
    assert!(engine.heard.lock().unwrap().is_empty());
}

fn ready_service(name: &str, text: &str) -> (PathBuf, Arc<DictationService>, Arc<ScriptedEngine>) {
    let dir = scratch(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("model.bin"), MODEL_BYTES).unwrap();
    let (service, engine) = service(
        &dir,
        spec_for("http://127.0.0.1:9/m".into(), MODEL_BYTES),
        text,
    );
    (dir, service, engine)
}

#[tokio::test]
async fn chunks_are_assembled_in_order_and_transcribed_once() {
    let (dir, service, engine) = ready_service("assemble", " Hello [BLANK_AUDIO] world (music) ");
    let first = service
        .lock()
        .uploads
        .append(OWNER, "rec-1", 0, &pcm_base64(8000))
        .unwrap();
    assert_eq!(first, 16000);
    service
        .lock()
        .uploads
        .append(OWNER, "rec-1", 16000, &pcm_base64(8000))
        .unwrap();
    let DictationResult::Transcript { text, .. } =
        service.transcribe(OWNER, "rec-1").await.unwrap()
    else {
        panic!("transcript")
    };
    assert_eq!(text, "Hello world");
    assert_eq!(*engine.heard.lock().unwrap(), vec![(16000, None)]);
    assert_eq!(
        code(service.transcribe(OWNER, "rec-1").await.err().unwrap()),
        "dictation_audio_unknown"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn the_chosen_language_is_saved_used_and_validated() {
    let (dir, service, engine) = ready_service("language", "hola");
    let DictationResult::Status { status } =
        service.set_language(OWNER, Some("es".into())).unwrap()
    else {
        panic!("status")
    };
    assert_eq!(status.language.as_deref(), Some("es"));
    assert_eq!(
        code(
            service
                .set_language(OWNER, Some("zz-not".into()))
                .err()
                .unwrap()
        ),
        "dictation_language_invalid"
    );
    service
        .lock()
        .uploads
        .append(OWNER, "rec-1", 0, &pcm_base64(16000))
        .unwrap();
    service.transcribe(OWNER, "rec-1").await.unwrap();
    assert_eq!(engine.heard.lock().unwrap()[0].1.as_deref(), Some("es"));
    let (reloaded, _) = self::service(
        &dir,
        spec_for("http://127.0.0.1:9/m".into(), MODEL_BYTES),
        "x",
    );
    assert_eq!(reloaded.status(OWNER).language.as_deref(), Some("es"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_recording_too_short_to_hold_speech_is_empty_without_running_the_model() {
    let (dir, service, engine) = ready_service("short", "noise");
    service
        .lock()
        .uploads
        .append(OWNER, "rec-1", 0, &pcm_base64(100))
        .unwrap();
    let DictationResult::Transcript { text, .. } =
        service.transcribe(OWNER, "rec-1").await.unwrap()
    else {
        panic!("transcript")
    };
    assert_eq!(text, "");
    assert!(engine.heard.lock().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recordings_are_bounded_and_ordered() {
    let mut uploads = AudioUploads::default();
    assert_eq!(
        uploads.append(OWNER, "bad id", 0, &pcm_base64(10)),
        Err(AudioError::InvalidId)
    );
    assert_eq!(
        uploads.append(OWNER, "a", 0, "not base64!"),
        Err(AudioError::InvalidChunk)
    );
    assert_eq!(
        uploads.append(
            OWNER,
            "a",
            0,
            &base64::engine::general_purpose::STANDARD.encode([1_u8; 3])
        ),
        Err(AudioError::InvalidChunk)
    );
    uploads.append(OWNER, "a", 0, &pcm_base64(10)).unwrap();
    assert_eq!(
        uploads.append(OWNER, "a", 0, &pcm_base64(10)),
        Err(AudioError::OutOfOrder { held: 20 })
    );
    for id in ["b", "c", "d"] {
        uploads.append(OWNER, id, 0, &pcm_base64(10)).unwrap();
    }
    assert_eq!(
        uploads.append(OWNER, "e", 0, &pcm_base64(10)),
        Err(AudioError::Busy)
    );
    let chunk = pcm_base64(100_000);
    let mut offset = 0;
    let counter = AtomicUsize::new(0);
    let outcome = loop {
        counter.fetch_add(1, Ordering::Relaxed);
        match uploads.append(OWNER, "a", 20 + offset, &chunk) {
            Ok(held) => offset = held - 20,
            Err(error) => break error,
        }
    };
    assert_eq!(outcome, AudioError::TooLong);
    assert!(
        uploads.take(OWNER, "a").is_err(),
        "an oversized recording is discarded"
    );
}

#[test]
fn whisper_annotations_are_not_dictated() {
    assert_eq!(spoken_text("[BLANK_AUDIO]"), "");
    assert_eq!(
        spoken_text(" (soft music) Hola  mundo [Music] "),
        "Hola mundo"
    );
    assert_eq!(
        spoken_text("an unclosed [bracket stays"),
        "an unclosed [bracket stays"
    );
}

/// Real whisper.cpp on real speech. Run with `--ignored` and set
/// CHARIOX_DICTATION_TEST_MODEL (the pinned model file), CHARIOX_DICTATION_TEST_WAV
/// (16 kHz mono 16-bit WAV with a 44-byte header) and CHARIOX_DICTATION_TEST_WORDS (a phrase it contains).
#[tokio::test]
#[ignore = "needs the downloaded model and a recorded sample; see the doc comment"]
async fn the_real_engine_transcribes_recorded_speech() {
    let var = |name| std::env::var(name).unwrap_or_else(|_| panic!("set {name}"));
    let model = PathBuf::from(var("CHARIOX_DICTATION_TEST_MODEL"));
    let wav = std::fs::read(var("CHARIOX_DICTATION_TEST_WAV")).unwrap();
    let dir = scratch("real");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(&model, dir.join(ModelSpec::pinned().file_name)).unwrap();
    let service = DictationService::with_parts(
        dir.clone(),
        ModelSpec::pinned(),
        Arc::new(WhisperEngine::default()),
    );
    assert_eq!(service.status(OWNER).model, DictationModelState::Ready);
    for (index, chunk) in wav[44..].chunks(DICTATION_CHUNK).enumerate() {
        let encoded = base64::engine::general_purpose::STANDARD.encode(chunk);
        service
            .lock()
            .uploads
            .append(OWNER, "real-1", (index * DICTATION_CHUNK) as u64, &encoded)
            .unwrap();
    }
    let DictationResult::Transcript { text, language } =
        service.transcribe(OWNER, "real-1").await.unwrap()
    else {
        panic!("transcript")
    };
    eprintln!("transcript ({language}): {text}");
    assert!(
        text.to_lowercase()
            .contains(&var("CHARIOX_DICTATION_TEST_WORDS").to_lowercase()),
        "{text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

const DICTATION_CHUNK: usize = 64 * 1024;

#[tokio::test]
async fn a_recording_and_a_language_belong_to_the_user_who_made_them() {
    let (dir, service, engine) = ready_service("owners", "private words");
    service
        .lock()
        .uploads
        .append("alice", "rec-1", 0, &pcm_base64(16000))
        .unwrap();
    assert_eq!(
        code(service.transcribe("bob", "rec-1").await.err().unwrap()),
        "dictation_audio_unknown",
        "bob cannot consume alice's recording"
    );
    assert!(engine.heard.lock().unwrap().is_empty());
    service.set_language("alice", Some("es".into())).unwrap();
    assert_eq!(service.status("bob").language, None);
    service.transcribe("alice", "rec-1").await.unwrap();
    assert_eq!(engine.heard.lock().unwrap()[0].1.as_deref(), Some("es"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(start_paused = true)]
async fn the_model_is_released_after_five_idle_minutes_even_when_transcription_failed() {
    let (dir, service, engine) = ready_service("failed-idle", FAILING);
    service
        .lock()
        .uploads
        .append(OWNER, "rec-1", 0, &pcm_base64(16000))
        .unwrap();
    let error = service.transcribe(OWNER, "rec-1").await.err().unwrap();
    assert_eq!(code(error), "dictation_transcription_failed");
    tokio::time::sleep(RELEASE_MODEL_AFTER - Duration::from_secs(1)).await;
    assert_eq!(engine.released.load(Ordering::SeqCst), 0, "not yet idle");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(engine.released.load(Ordering::SeqCst), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(start_paused = true)]
async fn a_newer_use_keeps_the_model_until_it_has_been_idle_too() {
    let (dir, service, engine) = ready_service("newer-use", "words");
    for (id, wait) in [("rec-1", 200), ("rec-2", 0)] {
        service
            .lock()
            .uploads
            .append(OWNER, id, 0, &pcm_base64(16000))
            .unwrap();
        service.transcribe(OWNER, id).await.unwrap();
        tokio::time::sleep(Duration::from_secs(wait)).await;
    }
    tokio::time::sleep(Duration::from_secs(150)).await;
    assert_eq!(
        engine.released.load(Ordering::SeqCst),
        0,
        "rec-2 is only 150 s old"
    );
    tokio::time::sleep(Duration::from_secs(151)).await;
    assert_eq!(
        engine.released.load(Ordering::SeqCst),
        1,
        "one release for the last use"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
