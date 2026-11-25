//! Reading the handover document the previous program left. Every terminal,
//! agent turn and job in it is read on its own: one that cannot be read ends
//! as after a restart, with its reason in the log, and the rest is handed on.

use serde::de::DeserializeOwned;
use serde_json::Value;

use super::{Handover, HANDOVER_FORMAT, PREVIOUS_HANDOVER_FORMAT};

/// What the document held that this program could not read, and what it
/// left running: the processes and descriptors the part names.
pub(super) struct Unreadable {
    part: &'static str,
    name: String,
    reason: String,
    pids: Vec<u32>,
    fds: Vec<i32>,
}

pub(super) struct ReadHandover {
    pub(super) handover: Handover,
    pub(super) unreadable: Vec<Unreadable>,
}

/// The document as this program reads it. Only a document that is not JSON or
/// is of a format neither this release nor the previous one writes fails as a
/// whole.
pub(super) fn read_handover(body: &[u8]) -> Result<ReadHandover, String> {
    let mut document: Value = serde_json::from_slice(body).map_err(|e| e.to_string())?;
    let format = document
        .get("format")
        .and_then(Value::as_u64)
        .ok_or("a handover without a format")?;
    if format != u64::from(HANDOVER_FORMAT) && format != u64::from(PREVIOUS_HANDOVER_FORMAT) {
        return Err(format!("a handover of format {format}"));
    }
    let mut unreadable = Vec::new();
    let user_domain = match document
        .as_object_mut()
        .and_then(|fields| fields.remove("user_domain"))
    {
        Some(value) => match serde_json::from_value(value) {
            Ok(access) => access,
            Err(error) => {
                // Without its grants the owner's agents list fewer tools: the safe side.
                unreadable.push(Unreadable {
                    part: "user_domain",
                    name: String::new(),
                    reason: error.to_string(),
                    pids: Vec::new(),
                    fds: Vec::new(),
                });
                Default::default()
            }
        },
        None => Default::default(),
    };
    let terminals = parts(&mut document, "terminals", &mut unreadable, terminal_part);
    let providers = parts(&mut document, "providers", &mut unreadable, provider_part);
    let attachments = parts(
        &mut document,
        "attachments",
        &mut unreadable,
        attachment_part,
    );
    let released_jobs = parts(&mut document, "released_jobs", &mut unreadable, no_part);
    let watched = parts(&mut document, "watched", &mut unreadable, watched_part);
    let local_jobs = parts(&mut document, "local_jobs", &mut unreadable, local_job_part);
    let admission = parts(&mut document, "admission", &mut unreadable, no_part);
    // What is left is the document's own: descriptors and listeners.
    let mut handover: Handover = serde_json::from_value(document).map_err(|e| e.to_string())?;
    handover.user_domain = user_domain;
    handover.terminals = terminals;
    handover.providers = providers;
    handover.attachments = attachments;
    handover.released_jobs = released_jobs;
    handover.watched = watched;
    handover.local_jobs = local_jobs;
    handover.admission = admission;
    Ok(ReadHandover {
        handover,
        unreadable,
    })
}

/// Names what an unreadable part left running: its name, pids and fds.
type Salvage = fn(&Value) -> (String, Vec<u32>, Vec<i32>);

/// The parts of one list, each read on its own.
fn parts<T: DeserializeOwned>(
    document: &mut Value,
    list: &'static str,
    unreadable: &mut Vec<Unreadable>,
    salvage: Salvage,
) -> Vec<T> {
    let raw = document
        .as_object_mut()
        .and_then(|fields| fields.insert(list.to_string(), Value::Array(Vec::new())));
    let Some(Value::Array(raw)) = raw else {
        return Vec::new();
    };
    raw.into_iter()
        .filter_map(|part| match serde_json::from_value::<T>(part.clone()) {
            Ok(read) => Some(read),
            Err(error) => {
                let (name, pids, fds) = salvage(&part);
                unreadable.push(Unreadable {
                    part: list,
                    name,
                    reason: error.to_string(),
                    pids,
                    fds,
                });
                None
            }
        })
        .collect()
}

fn text<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter()
        .try_fold(value, |value, key| value.get(key))?
        .as_str()
}

fn number(value: &Value, path: &[&str]) -> Option<i64> {
    path.iter()
        .try_fold(value, |value, key| value.get(key))?
        .as_i64()
}

fn no_part(_: &Value) -> (String, Vec<u32>, Vec<i32>) {
    (String::new(), Vec::new(), Vec::new())
}

fn terminal_part(part: &Value) -> (String, Vec<u32>, Vec<i32>) {
    (
        text(part, &["terminal_id"]).unwrap_or_default().to_string(),
        number(part, &["pty", "pid"])
            .into_iter()
            .map(|n| n as u32)
            .collect(),
        number(part, &["pty", "master_fd"])
            .into_iter()
            .map(|n| n as i32)
            .collect(),
    )
}

fn provider_part(part: &Value) -> (String, Vec<u32>, Vec<i32>) {
    let name = format!(
        "run {} of agent {} in session {}",
        text(part, &["run_id"]).unwrap_or("?"),
        text(part, &["agent_id"]).unwrap_or("?"),
        text(part, &["session_id"]).unwrap_or("?"),
    );
    let (pid, fds) = if let Some(claude) = part.pointer("/process/claude") {
        (
            number(claude, &["pid"]),
            ["stdin", "stdout", "stderr"]
                .iter()
                .filter_map(|key| number(claude, &[key]))
                .collect::<Vec<_>>(),
        )
    } else if let Some(codex) = part.pointer("/process/codex") {
        (
            number(codex, &["pty", "pid"]),
            [
                number(codex, &["pty", "master_fd"]),
                number(codex, &["turn", "connection"]),
            ]
            .into_iter()
            .flatten()
            .collect(),
        )
    } else {
        (None, Vec::new())
    };
    (
        name,
        pid.into_iter().map(|n| n as u32).collect(),
        fds.into_iter().map(|n| n as i32).collect(),
    )
}

fn attachment_part(part: &Value) -> (String, Vec<u32>, Vec<i32>) {
    (
        text(part, &["id"]).unwrap_or_default().to_string(),
        Vec::new(),
        Vec::new(),
    )
}

fn watched_part(part: &Value) -> (String, Vec<u32>, Vec<i32>) {
    let fds = part
        .get("fds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_i64)
        .map(|n| n as i32)
        .collect();
    (
        text(part, &["wake"]).unwrap_or_default().to_string(),
        number(part, &["pid"])
            .into_iter()
            .map(|n| n as u32)
            .collect(),
        fds,
    )
}

fn local_job_part(part: &Value) -> (String, Vec<u32>, Vec<i32>) {
    // A running command is the child of the previous program, kept apart from its output
    // pipes, which the job owns and a reader on the other side may block on.
    let streams = part.pointer("/exec/streams");
    let command = streams
        .and_then(|streams| streams.pointer("/child/Running/witness/root"))
        .and_then(Value::as_u64)
        .map(|pid| pid as u32);
    (
        text(part, &["job"]).unwrap_or_default().to_string(),
        command.into_iter().collect(),
        [
            number(part, &["state_fd"]),
            number(part, &["log_fd"]),
            streams.and_then(|streams| number(streams, &["stdout"])),
            streams.and_then(|streams| number(streams, &["stderr"])),
        ]
        .into_iter()
        .flatten()
        .map(|n| n as i32)
        .collect(),
    )
}

/// What a document that was refused as a whole still names, as far as it can be parsed at
/// all: every terminal, agent turn, watched process and job it holds ends as after a restart.
pub(super) fn salvage_document(body: &[u8]) -> Vec<Unreadable> {
    let Ok(document) = serde_json::from_slice::<Value>(body) else {
        return Vec::new();
    };
    let lists: [(&'static str, Salvage); 4] = [
        ("terminals", terminal_part),
        ("providers", provider_part),
        ("watched", watched_part),
        ("local_jobs", local_job_part),
    ];
    let mut unreadable = Vec::new();
    for (list, salvage) in lists {
        for part in document
            .get(list)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let (name, pids, fds) = salvage(part);
            unreadable.push(Unreadable {
                part: list,
                name,
                reason: "the handover document was refused as a whole".to_string(),
                pids,
                fds,
            });
        }
    }
    unreadable
}

/// Logs each part that could not be read and ends what it left running, so
/// that terminal or agent starts again as after a restart. Only children of
/// this program are ended, and only descriptors no readable part names are
/// closed.
pub(super) fn end_unreadable(unreadable: Vec<Unreadable>, named: &[i32]) {
    for part in unreadable {
        super::note_for_log(
            true,
            "a handed-over part could not be read; it ends as after a restart and the rest continues",
            serde_json::json!({
                "part": part.part,
                "name": part.name,
                "reason": part.reason,
                "processes": part.pids,
            }),
        );
        for pid in part.pids {
            super::end_own_child(pid);
        }
        for fd in part.fds {
            if fd > 2 && !named.contains(&fd) {
                // SAFETY: the previous program left this descriptor open for this part alone.
                unsafe { libc::close(fd) };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// The documents the previous release's kernel wrote in a real hot reload with a Claude
    /// turn and a Codex turn running (`apps/kernel/tests/fixtures/hot_reload/README.md`).
    fn previous_release_documents() -> Vec<(String, Vec<u8>)> {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hot_reload");
        let mut documents: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|file| file.path().extension().is_some_and(|ext| ext == "json"))
            .map(|file| {
                (
                    file.file_name().to_string_lossy().into_owned(),
                    std::fs::read(file.path()).unwrap(),
                )
            })
            .collect();
        documents.sort();
        assert!(
            !documents.is_empty(),
            "no previous release's handover in {dir:?}"
        );
        documents
    }

    fn dead(pid: u32) -> bool {
        // SAFETY: signal 0 only reports whether the process exists.
        unsafe { libc::kill(pid as libc::pid_t, 0) != 0 }
    }

    fn command_that_waits() -> std::process::Child {
        std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .unwrap()
    }

    #[test]
    fn a_local_job_that_cannot_be_read_names_its_running_command_and_pipes() {
        let part = serde_json::json!({
            "job": "job-1",
            "exec": { "streams": {
                "child": { "Running": { "witness": { "root": 4242, "owned": {} } } },
                "stdout": 31, "stderr": 32 } },
            "state_fd": 29, "log_fd": 30,
        });
        let (name, pids, fds) = local_job_part(&part);
        assert_eq!(name, "job-1");
        assert_eq!(pids, vec![4242]);
        assert_eq!(fds, vec![29, 30, 31, 32]);
    }

    #[test]
    fn a_corrupt_local_job_ends_its_running_command() {
        let mut child = command_that_waits();
        let pid = child.id();
        let (body, mut document) = {
            let (_, body) = previous_release_documents().remove(0);
            let document = json(&body);
            (body, document)
        };
        drop(body);
        // The job's wake is damaged; its command and pipes are intact.
        document["local_jobs"] = serde_json::json!([{
            "job": "job-corrupt",
            "exec": { "streams": { "child": { "Running": { "witness": { "root": pid, "owned": {} } } } } },
            "state_fd": -1, "log_fd": -1,
        }]);
        let read = read_handover(&serde_json::to_vec(&document).unwrap()).unwrap();
        assert_eq!(read.unreadable.len(), 1);
        end_unreadable(read.unreadable, &[]);
        assert!(
            dead(pid),
            "the command of a rejected local job kept running"
        );
        let _ = child.try_wait();
    }

    #[test]
    fn a_document_refused_as_a_whole_still_ends_what_it_left_running() {
        let mut terminal = command_that_waits();
        let mut job = command_that_waits();
        let document = serde_json::json!({
            "format": 999_999,
            "terminals": [{ "terminal_id": "t", "pty": { "pid": terminal.id(), "master_fd": -1 } }],
            "local_jobs": [{
                "job": "j",
                "exec": { "streams": { "child": { "Running": { "witness": { "root": job.id(), "owned": {} } } } } },
                "state_fd": -1, "log_fd": -1,
            }],
        });
        let body = serde_json::to_vec(&document).unwrap();
        assert!(
            read_handover(&body).is_err(),
            "the document is refused as a whole"
        );
        end_unreadable(salvage_document(&body), &[]);
        assert!(
            dead(terminal.id()),
            "a terminal of a refused document kept running"
        );
        assert!(dead(job.id()), "a job of a refused document kept running");
        let _ = (terminal.try_wait(), job.try_wait());
    }

    fn json(body: &[u8]) -> Value {
        serde_json::from_slice(body).unwrap()
    }

    fn adapters(handover: &Handover) -> Vec<&'static str> {
        handover
            .providers
            .iter()
            .map(|p| p.process.adapter_key())
            .collect()
    }

    #[test]
    fn the_format_numbers_follow_the_rule() {
        const {
            assert!(PREVIOUS_HANDOVER_FORMAT <= HANDOVER_FORMAT);
            assert!(
                HANDOVER_FORMAT <= PREVIOUS_HANDOVER_FORMAT + 1,
                "a release reads its own handover format and the one before it"
            );
        }
    }

    /// The guard: a field added to a handover shape without a serde default, or a shape
    /// change without a `HANDOVER_FORMAT` bump, fails here, because the document the previous
    /// release wrote no longer reads.
    #[test]
    fn the_previous_releases_handover_is_read_whole() {
        for (name, body) in previous_release_documents() {
            let document = json(&body);
            let format = document["format"].as_u64().unwrap() as u32;
            assert!(
                format == HANDOVER_FORMAT || format == PREVIOUS_HANDOVER_FORMAT,
                "{name} has format {format}: keep only the previous release's documents"
            );
            let read = read_handover(&body).unwrap_or_else(|error| panic!("{name}: {error}"));
            if let Some(part) = read.unreadable.first() {
                panic!(
                    "{name}: {} {} cannot be read: {}",
                    part.part, part.name, part.reason
                );
            }
            assert_eq!(
                read.handover.providers.len(),
                document["providers"].as_array().unwrap().len()
            );
            assert_eq!(
                read.handover.terminals.len(),
                document["terminals"].as_array().unwrap().len()
            );
            let adapters = adapters(&read.handover);
            assert!(
                adapters.contains(&"claude") && adapters.contains(&"codex"),
                "{name} must hold a running Claude turn and a running Codex turn: {adapters:?}"
            );
        }
    }

    /// L12 wrote turns without what L12.1 added: a hot reload from L12 reads them.
    #[test]
    fn a_handover_written_before_the_added_fields_is_read_whole() {
        for (name, body) in previous_release_documents() {
            let mut document = json(&body);
            for part in document["providers"].as_array_mut().unwrap() {
                if let Some(turn) = part.pointer_mut("/process/codex/turn") {
                    let turn = turn.as_object_mut().unwrap();
                    turn.remove("usage_tracker");
                    turn.remove("selected_model");
                }
                if let Some(turn) = part.pointer_mut("/process/claude/turn") {
                    let turn = turn.as_object_mut().unwrap();
                    turn.remove("own_turn");
                    turn.remove("background_notification_pending");
                }
            }
            let read = read_handover(&serde_json::to_vec(&document).unwrap())
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert!(read.unreadable.is_empty(), "{name}");
            assert_eq!(
                read.handover.providers.len(),
                document["providers"].as_array().unwrap().len()
            );
        }
    }

    /// One agent's part that cannot be read costs that agent its turn and nothing else.
    #[test]
    fn a_part_that_cannot_be_read_costs_only_its_own_agent() {
        let (_, body) = previous_release_documents().remove(0);
        for damaged in ["codex", "claude"] {
            let mut document = json(&body);
            let providers = document["providers"].as_array_mut().unwrap();
            let total = providers.len();
            let part = providers
                .iter_mut()
                .find(|part| part["process"].get(damaged).is_some())
                .unwrap();
            let run_id = part["run_id"].as_str().unwrap().to_string();
            // A field of the wrong kind, as a release that wrote the part differently leaves.
            match damaged {
                "codex" => part["process"]["codex"]["turn"]["next_request_id"] = "x".into(),
                _ => part["process"]["claude"]["turn"]["next_turn_number"] = "x".into(),
            }
            let read = read_handover(&serde_json::to_vec(&document).unwrap()).unwrap();
            assert_eq!(read.handover.providers.len(), total - 1, "{damaged}");
            assert!(read
                .handover
                .providers
                .iter()
                .all(|provider| provider.run_id != run_id));
            assert_eq!(read.unreadable.len(), 1, "{damaged}");
            let lost = &read.unreadable[0];
            assert_eq!(lost.part, "providers");
            assert!(lost.name.contains(&run_id), "{}", lost.name);
            assert!(!lost.reason.is_empty());
            // What the lost part left running is named, so it can be ended.
            assert_eq!(lost.pids.len(), 1, "{damaged}");
            assert!(!lost.fds.is_empty(), "{damaged}");
        }
    }

    #[test]
    fn a_damaged_attachment_or_shared_part_does_not_drop_the_agents() {
        let (_, body) = previous_release_documents().remove(0);
        let mut document = json(&body);
        let total = document["providers"].as_array().unwrap().len();
        document["attachments"] = serde_json::json!([
            {"id": "broken"},
            {"id": "a1", "session_id": "s", "client_id": "c",
             "capability_level": "FullTerminal", "owner_user_id": "u"},
        ]);
        document["user_domain"] = serde_json::json!("not an access record");
        let read = read_handover(&serde_json::to_vec(&document).unwrap()).unwrap();
        assert_eq!(read.handover.providers.len(), total);
        assert_eq!(read.handover.attachments.len(), 1);
        let lost: Vec<_> = read.unreadable.iter().map(|part| part.part).collect();
        assert_eq!(lost, ["user_domain", "attachments"]);
    }

    #[test]
    fn a_handover_of_another_format_or_no_document_is_refused_whole() {
        let (_, body) = previous_release_documents().remove(0);
        let mut document = json(&body);
        document["format"] = (HANDOVER_FORMAT + 1).into();
        assert!(read_handover(&serde_json::to_vec(&document).unwrap()).is_err());
        assert!(read_handover(b"not json").is_err());
    }
}
