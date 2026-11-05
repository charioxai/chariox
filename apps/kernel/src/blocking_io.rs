//! Serialize blocking persistence without retaining a state mutex during I/O.
use std::sync::{Condvar, Mutex};

#[derive(Debug, Default)]
pub(crate) struct Serial {
    occupied: Mutex<bool>,
    ready: Condvar,
}

pub(crate) struct Permit<'a>(&'a Serial);

impl Serial {
    pub(crate) fn acquire(&self) -> Permit<'_> {
        let mut occupied = self
            .occupied
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while *occupied {
            occupied = self
                .ready
                .wait(occupied)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        *occupied = true;
        drop(occupied);
        Permit(self)
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        *self
            .0
            .occupied
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = false;
        self.0.ready.notify_one();
    }
}
