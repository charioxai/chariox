//! MP-08 / MP-10 / MP-11 opt-in, bounded public dispatch timing diagnostics.
//! Never accepts or logs provider payloads, prompts, context, or credentials.
use std::collections::BTreeSet;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

struct Turn {
    context: String,
    prompt_id: String,
    started: Instant,
    phases: BTreeSet<&'static str>,
}

#[derive(Default)]
struct TimingState {
    turns: Vec<Turn>,
    emitted: usize,
}

pub(super) fn record(context: &str, prompt_id: &str, phase: &'static str) {
    if std::env::var_os("CHARIOX_CLAUDE_NATIVE_TIMING").as_deref()
        != Some(std::ffi::OsStr::new("1"))
    {
        return;
    }
    static STATE: OnceLock<Mutex<TimingState>> = OnceLock::new();
    let mut state = STATE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // Cap total logs as well as memory, even if more than 64 active turns
    // repeatedly evict one another from the diagnostic cache.
    if state.emitted >= 2048 {
        return;
    }
    let turns = &mut state.turns;
    let index = match turns
        .iter()
        .position(|turn| turn.context == context && turn.prompt_id == prompt_id)
    {
        Some(index) => index,
        None => {
            if turns.len() == 64 {
                turns.remove(0);
            }
            turns.push(Turn {
                context: context.into(),
                prompt_id: prompt_id.into(),
                started: Instant::now(),
                phases: BTreeSet::new(),
            });
            turns.len() - 1
        }
    };
    let turn = &mut turns[index];
    if turn.phases.len() >= 16 || !turn.phases.insert(phase) {
        return;
    }
    let elapsed_ms = turn.started.elapsed().as_millis() as u64;
    state.emitted += 1;
    drop(state);
    crate::logging::info_with_fields(
        "daemon.claude_native_timing",
        "native dispatch timing",
        serde_json::json!({ "prompt_id": prompt_id, "phase": phase, "elapsed_ms": elapsed_ms }),
    );
}
