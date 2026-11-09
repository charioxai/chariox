//! The revision clients compare to know a sub-agent tree changed. One counter
//! for the kernel, seeded from the clock so it still grows after a restart; a
//! tree gets a new number only when what a client reads of it differs.
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// What one tree last read as, and what followers were last told of it.
struct Tree {
    rev: u64,
    fingerprint: u64,
    published: Option<u64>,
}

#[derive(Default)]
struct Revs {
    counter: u64,
    trees: HashMap<(String, String), Tree>,
}

fn revs() -> &'static Mutex<Revs> {
    static REVS: OnceLock<Mutex<Revs>> = OnceLock::new();
    REVS.get_or_init(Mutex::default)
}

/// Records the fingerprint of the tree below `root` and returns its revision.
/// A read says nothing about what followers were told: see `claim_publication`.
pub(crate) fn observe(session: &str, root: &str, fingerprint: u64) -> u64 {
    let mut revs = revs()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let key = (session.to_owned(), root.to_owned());
    if let Some(tree) = revs
        .trees
        .get(&key)
        .filter(|tree| tree.fingerprint == fingerprint)
    {
        return tree.rev;
    }
    revs.counter = revs
        .counter
        .max(crate::session::unix_epoch_ms())
        .saturating_add(1);
    let rev = revs.counter;
    let published = revs.trees.get(&key).and_then(|tree| tree.published);
    revs.trees.insert(
        key,
        Tree {
            rev,
            fingerprint,
            published,
        },
    );
    rev
}

/// Whether followers are still owed an event for this fingerprint of the tree;
/// true once, until the tree changes again.
pub(crate) fn claim_publication(session: &str, root: &str, fingerprint: u64) -> bool {
    let mut revs = revs()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(tree) = revs.trees.get_mut(&(session.to_owned(), root.to_owned())) else {
        return false;
    };
    if tree.published == Some(fingerprint) {
        return false;
    }
    tree.published = Some(fingerprint);
    true
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
