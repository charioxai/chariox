//! MP-09 / MP-11 A03: bounded, protected process observations before matching.
use std::io::Read;
use std::sync::Arc;
// Leave room for JSON escaping and source attribution in the 8 KiB event.
const TAIL_BYTES: usize = 2_048;
const LINE_BYTES: usize = 1_024;
// Lines held while they may begin a multi-line value; a longer hold is withheld.
const HOLD_BYTES: usize = 8_192;

/// Redacts registered values from process output, line by line and across lines.
#[derive(Clone)]
pub(super) struct Protector {
    scrub: Arc<dyn Fn(String) -> Option<String> + Send + Sync>,
    // Whether the text ends with the beginning of a value that continues on the next line.
    holds: Arc<dyn Fn(&str) -> bool + Send + Sync>,
}

impl Protector {
    pub(super) fn scrub(&self, text: String) -> Option<String> {
        (self.scrub)(text)
    }
    fn holds(&self, text: &str) -> bool {
        (self.holds)(text)
    }
    #[cfg(test)]
    fn passthrough() -> Self {
        Self {
            scrub: Arc::new(Some),
            holds: Arc::new(|_| false),
        }
    }
}

pub(super) fn room_protector(
    protection: super::room_secret_observation::RoomSecretObservations,
    room: String,
) -> Protector {
    let holder = (protection.clone(), room.clone());
    Protector {
        scrub: Arc::new(move |text| protection.scrub(&room, text).ok()),
        holds: Arc::new(move |text| holder.0.multiline_prefix(&holder.1, text)),
    }
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
    protect: Protector,
) {
    let mut buffer = [0u8; 4096];
    let mut line = Vec::new();
    let mut truncated = false;
    // Raw lines that may begin a value continuing on the next line.
    let mut held: Vec<String> = Vec::new();
    // `raw` is None for a withheld line. A cut line could expose only part of a
    // protected value, so it is withheld entirely rather than matched or retained.
    let publish = |raw: Option<String>, was_truncated: bool| {
        // Scrub the raw text too: sanitizing first would drop the byte after a stray escape.
        let text = raw.and_then(|raw| {
            protect
                .scrub(raw)
                .and_then(|text| protect.scrub(sanitize(text.as_bytes())))
        });
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
    let mut emit = |line: &mut Vec<u8>, truncated: &mut bool| {
        let was_truncated = std::mem::take(truncated);
        let raw = String::from_utf8_lossy(line).into_owned();
        line.clear();
        if was_truncated {
            for held in held.drain(..) {
                publish(Some(held), false);
            }
            return publish(None, true);
        }
        held.push(raw);
        let joined = held.join("\n") + "\n";
        // A value spanning lines is redacted over the joined text, never line by line.
        if let Some(redacted) = protect.scrub(joined.clone()).filter(|text| *text != joined) {
            held.clear();
            return redacted
                .lines()
                .for_each(|l| publish(Some(l.into()), false));
        }
        if protect.holds(&joined) {
            if joined.len() > HOLD_BYTES {
                held.clear();
                publish(None, false);
            }
            return;
        }
        for held in held.drain(..) {
            publish(Some(held), false);
        }
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
    // The stream ended inside a possible value: its first lines stay withheld.
    if !held.is_empty() {
        publish(None, false);
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
            Protector::passthrough(),
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
    fn public_watcher_match_fires_after_a_vault_fill_and_the_value_is_redacted() {
        let root = std::env::temp_dir().join(format!(
            "chariox-watcher-fill-{}-{}",
            std::process::id(),
            rand::random::<u128>()
        ));
        std::fs::create_dir(&root).unwrap();
        let protection = super::super::room_secret_observation::RoomSecretObservations::new(
            root.clone(),
            Default::default(),
        );
        protection.register("room", "fill-canary-value").unwrap();
        let output = Arc::new(Output::default());
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        drain(
            &b"login fill-canary-value accepted\n\x1b[32mserver ready\x1b[0m\n"[..],
            output.clone(),
            Some("ready".into()),
            sender,
            room_protector(protection, "room".into()),
        );
        assert!(matches!(
            receiver.try_recv(),
            Ok(Signal::Matched(line)) if line == "server ready"
        ));
        let tail = output.text();
        assert!(tail.contains("login [redacted] accepted") && !tail.contains("fill-canary"));
        std::fs::remove_dir_all(root).unwrap();
    }
    // A value on several lines must not be rebuilt from lines that each look harmless.
    #[test]
    fn a03_multiline_value_is_redacted_across_lines_and_public_lines_stay() {
        let root = std::env::temp_dir().join(format!(
            "chariox-watcher-multiline-{}-{}",
            std::process::id(),
            rand::random::<u128>()
        ));
        std::fs::create_dir(&root).unwrap();
        let protection = super::super::room_secret_observation::RoomSecretObservations::new(
            root.clone(),
            Default::default(),
        );
        protection
            .register("room", "watcher-alpha\nwatcher-beta")
            .unwrap();
        let run = |input: &str, pattern: &str| {
            let output = Arc::new(Output::default());
            let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
            drain(
                input.as_bytes(),
                output.clone(),
                Some(pattern.into()),
                sender,
                room_protector(protection.clone(), "room".into()),
            );
            let mut matches = vec![];
            while let Ok(Signal::Matched(line)) = receiver.try_recv() {
                matches.push(line);
            }
            (output.text(), matches)
        };
        let (tail, matches) = run(
            "start\nwatcher-alpha\nwatcher-beta\nserver ready\n",
            "ready",
        );
        assert!(
            tail.contains("start") && tail.contains("server ready"),
            "{tail}"
        );
        assert!(
            !tail.contains("watcher-alpha") && !tail.contains("watcher-beta"),
            "{tail}"
        );
        assert_eq!(matches, vec!["server ready".to_string()]);
        // A fragment is never matched or retained when the value may still complete.
        let (tail, matches) = run("watcher-alpha\n", "watcher-alpha");
        assert!(
            !tail.contains("watcher-alpha") && matches.is_empty(),
            "{tail}"
        );
        // Once the next line differs, the held line is ordinary public output.
        let (tail, _) = run("watcher-alpha\nserver ready\n", "ready");
        assert!(
            tail.contains("watcher-alpha") && tail.contains("server ready"),
            "{tail}"
        );
        std::fs::remove_dir_all(root).unwrap();
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
            Protector::passthrough(),
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
