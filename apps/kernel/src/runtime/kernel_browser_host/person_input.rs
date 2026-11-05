//! MP-08/MP-10/MP-11: wait outside mutation/actor reservations so new person keys can enter.
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const IDLE: Duration = Duration::from_secs(2);
type Key = (String, String);
struct Activity {
    last: Instant,
    pending: usize,
}
#[derive(Default)]
pub(super) struct PersonInputs(Mutex<BTreeMap<Key, Activity>>);
pub(super) struct Input<'a> {
    activity: &'a PersonInputs,
    key: Key,
}
impl PersonInputs {
    pub(super) fn begin(&self, user: &str, tab: &str) -> Input<'_> {
        let mut entries = self.0.lock().unwrap_or_else(|e| e.into_inner());
        entries.retain(|_, activity| activity.pending > 0 || activity.last.elapsed() < IDLE);
        let key = (user.to_owned(), tab.to_owned());
        let activity = entries.entry(key.clone()).or_insert(Activity {
            last: Instant::now(),
            pending: 0,
        });
        activity.last = Instant::now();
        activity.pending += 1;
        Input {
            activity: self,
            key,
        }
    }
    fn remaining(&self, key: &Key) -> Result<Duration, String> {
        let entries = self
            .0
            .lock()
            .map_err(|_| "browser person-input lock poisoned")?;
        Ok(entries
            .get(key)
            .map(|activity| {
                if activity.pending > 0 {
                    Duration::from_millis(50)
                } else {
                    IDLE.saturating_sub(activity.last.elapsed())
                }
            })
            .unwrap_or_default())
    }
    pub(super) fn quiet(&self, user: &str, tab: &str) -> Result<bool, String> {
        Ok(self
            .remaining(&(user.to_owned(), tab.to_owned()))?
            .is_zero())
    }

    pub(super) fn wait(
        &self,
        user: &str,
        tab: &str,
        authority: impl Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        let key = (user.to_owned(), tab.to_owned());
        loop {
            authority()?;
            let wait = self.remaining(&key)?;
            if wait.is_zero() {
                return Ok(());
            }
            std::thread::sleep(wait.min(Duration::from_millis(50)));
        }
    }
}
impl Drop for Input<'_> {
    fn drop(&mut self) {
        let mut entries = self.activity.0.lock().unwrap_or_else(|e| e.into_inner());
        let activity = entries
            .get_mut(&self.key)
            .expect("pending person input remains registered");
        activity.pending -= 1;
        activity.last = Instant::now();
    }
}
