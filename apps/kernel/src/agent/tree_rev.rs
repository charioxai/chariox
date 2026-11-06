//! The revision clients compare to know a sub-agent tree changed. One counter
//! for the kernel, seeded from the clock so it still grows after a restart; a
//! tree gets a new number only when what a client reads of it differs.
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

#[derive(Default)]
struct Revs {
    counter: u64,
    trees: HashMap<(String, String), (u64, u64)>,
}

fn revs() -> &'static Mutex<Revs> {
    static REVS: OnceLock<Mutex<Revs>> = OnceLock::new();
    REVS.get_or_init(Mutex::default)
}

/// What `observe` found for one tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Observed {
    pub(crate) rev: u64,
    /// The tree differs from the last one seen, or was never seen.
    pub(crate) changed: bool,
}

/// Records the fingerprint of the tree below `root` and returns its revision.
pub(crate) fn observe(session: &str, root: &str, fingerprint: u64) -> Observed {
    let mut revs = revs()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let key = (session.to_owned(), root.to_owned());
    let seen = revs.trees.get(&key).copied();
    if let Some((rev, _)) = seen.filter(|(_, last)| *last == fingerprint) {
        return Observed {
            rev,
            changed: false,
        };
    }
    revs.counter = revs
        .counter
        .max(crate::session::unix_epoch_ms())
        .saturating_add(1);
    let rev = revs.counter;
    revs.trees.insert(key, (rev, fingerprint));
    Observed { rev, changed: true }
}

/// The roots of one session whose trees were seen.
pub(crate) fn seen_roots(session: &str) -> Vec<String> {
    let revs = revs()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    revs.trees
        .keys()
        .filter(|(seen, _)| seen == session)
        .map(|(_, root)| root.clone())
        .collect()
}

/// Forgets the trees of agents that no longer exist.
pub(crate) fn retain(live: impl Fn(&str, &str) -> bool) {
    let mut revs = revs()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    revs.trees.retain(|(session, root), _| live(session, root));
}
