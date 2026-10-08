//! MP-09 / MP-11 A03: bounded, protected process observations before matching.
use std::io::Read;
use std::sync::Arc;
// Leave room for JSON escaping and source attribution in the 8 KiB event.
const TAIL_BYTES: usize = 2_048;
const LINE_BYTES: usize = 1_024;

pub(super) fn room_protector(
    protection: super::room_secret_observation::RoomSecretObservations,
    room: String,
) -> Arc<dyn Fn(String) -> Option<String> + Send + Sync> {
    Arc::new(move |text| {
        if protection.protects_bytes(&room) {
            None
        } else {
            protection.scrub(&room, text).ok()
        }
    })
}

/// Strips terminal escapes/control bytes and redacts secrets before any
/// output is matched, retained or shown to the model.
pub(super) fn sanitize(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let control = if c == '\u{1b}' {
            chars.next()
        } else {
            match c {
                '\u{9b}' => Some('['),
                '\u{9d}' => Some(']'),
                '\u{90}' | '\u{98}' | '\u{9e}' | '\u{9f}' => Some('P'),
                _ => None,
            }
        };
        match control {
            Some('[') => {
                for n in chars.by_ref() {
                    if ('@'..='~').contains(&n) {
                        break;
                    }
                }
            }
            Some(']' | 'P' | 'X' | '^' | '_') => {
                while let Some(n) = chars.next() {
                    if n == '\u{7}' || n == '\u{9c}' {
                        break;
                    }
                    if n == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            Some(_) => {}
            None if c == '\t' || !c.is_control() => out.push(c),
            None => {}
        }
    }
    crate::secret_redaction::redact_secrets(&out).into_owned()
}

pub(super) enum Signal {
    Matched(String),
    Exited(i32, String),
    Lost(String),
    SupervisionFailed,
}

#[derive(Default)]
pub(super) struct Output {
    tail: std::sync::Mutex<std::collections::VecDeque<String>>,
    matched: std::sync::atomic::AtomicBool,
    stopped: std::sync::atomic::AtomicBool,
}

impl Output {
    pub(super) fn stop(&self) {
        self.stopped
            .store(true, std::sync::atomic::Ordering::Release);
    }
    fn push(&self, line: String) {
        let Ok(mut tail) = self.tail.lock() else {
            return;
        };
        tail.push_back(line);
        while tail.iter().map(|l| l.len() + 1).sum::<usize>() > TAIL_BYTES {
            tail.pop_front();
        }
    }
    pub(super) fn text(&self) -> String {
        self.tail
            .lock()
            .map(|t| t.iter().cloned().collect::<Vec<_>>().join("\n"))
            .unwrap_or_default()
    }
}

pub(super) fn drain(
    mut stream: impl Read,
    output: Arc<Output>,
    pattern: Option<String>,
    sender: tokio::sync::mpsc::UnboundedSender<Signal>,
    protect: Arc<dyn Fn(String) -> Option<String> + Send + Sync>,
) {
    let mut buffer = [0u8; 4096];
    let mut line = Vec::new();
    let mut truncated = false;
    let emit = |line: &mut Vec<u8>, truncated: &mut bool| {
        // A cut line could expose only part of a protected value. Withhold
        // it entirely rather than matching or retaining an unsafe prefix.
        let text = if *truncated {
            None
        } else {
            protect(sanitize(line))
        };
        line.clear();
        let was_truncated = std::mem::take(truncated);
        let Some(text) = text else {
            output.push(
                if was_truncated {
                    "[process output line withheld: truncated]"
                } else {
                    "[sensitive Room process output withheld]"
                }
                .into(),
            );
            return;
        };
        if let Some(pattern) = &pattern {
            if text.contains(pattern.as_str())
                && !output
                    .matched
                    .swap(true, std::sync::atomic::Ordering::AcqRel)
            {
                let _ = sender.send(Signal::Matched(text.clone()));
            }
        }
        output.push(text);
    };
    while !output.stopped.load(std::sync::atomic::Ordering::Acquire) {
        let n = match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => n,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(20));
                continue;
            }
            Err(_) => break,
        };
        for byte in &buffer[..n] {
            if *byte == b'\n' {
                emit(&mut line, &mut truncated);
            } else if line.len() < LINE_BYTES {
                line.push(*byte);
            } else {
                truncated = true;
            }
        }
    }
    if !line.is_empty() {
        emit(&mut line, &mut truncated);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a03_escaped_exit_excerpt_fits_the_durable_source_limit() {
        let output = Arc::new(Output::default());
        let (sender, _receiver) = tokio::sync::mpsc::unbounded_channel();
        let input = ("\"".repeat(1_023) + "\n").repeat(6);
        drain(
            input.as_bytes(),
            output.clone(),
            None,
            sender,
            Arc::new(Some),
        );
        let answer = serde_json::json!({"wake_id":"wake-source","kind":"process","label":"界".repeat(200),"exit_code":0,"process_lost":false,"output_tail":output.text()});
        assert!(
            serde_json::to_vec(&answer).unwrap().len() <= 8_192,
            "escaped output must not make a valid exit impossible to commit"
        );
    }

    #[test]
    fn a03_protected_and_unknown_room_output_cannot_match_or_be_retained() {
        for unknown in [false, true] {
            let root = std::env::temp_dir().join(format!(
                "chariox-am3-protected-output-{:032x}",
                rand::random::<u128>()
            ));
            std::fs::create_dir(&root).unwrap();
            let protection = super::super::room_secret_observation::RoomSecretObservations::new(
                root.clone(),
                Default::default(),
            );
            let canary = "private-observation-canary";
            if unknown {
                assert!(protection.register("room", "").is_err());
            } else {
                protection.register("room", canary).unwrap();
            }
            let output = Arc::new(Output::default());
            let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
            drain(
                canary.as_bytes(),
                output.clone(),
                Some(canary.into()),
                sender,
                room_protector(protection, "room".into()),
            );
            assert!(
                !output.text().contains(canary),
                "protected bytes must never enter the retained tail"
            );
            assert!(
                receiver.try_recv().is_err(),
                "protected bytes must never trigger a match event"
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn a03_output_is_sanitized_bounded_and_matches_once() {
        assert_eq!(sanitize(b"\x1b[31mred\x1b[0m ok\x07"), "red ok");
        let output = Arc::new(Output::default());
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let mut input = b"boot\nready one\nready two\n".to_vec();
        input.extend(std::iter::repeat_n(b'x', 10_000));
        drain(
            &input[..],
            output.clone(),
            Some("ready".into()),
            sender,
            Arc::new(Some),
        );
        let mut matches = vec![];
        while let Ok(Signal::Matched(line)) = receiver.try_recv() {
            matches.push(line);
        }
        assert_eq!(matches, vec!["ready one".to_string()]);
        let tail = output.text();
        assert!(
            tail.len() <= TAIL_BYTES && tail.ends_with("[process output line withheld: truncated]")
        );
    }
}
