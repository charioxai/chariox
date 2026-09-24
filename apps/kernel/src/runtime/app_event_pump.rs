//! One bounded background pass shared by every clone of AppControl. The existing
//! transport pump supplies ticks; this component owns no scheduler or thread.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub(crate) type AppCursor = (String, String);
#[derive(Clone)]
pub(crate) struct AppEventPump(Arc<Mutex<State>>);
struct State {
    in_flight: bool,
    wake: bool,
    next: Instant,
    delivery: Option<AppCursor>,
    maintenance: Option<AppCursor>,
    dispatch: Option<String>,
}
pub(crate) struct AppEventPass {
    state: Arc<Mutex<State>>,
    delivery: Option<AppCursor>,
    maintenance: Option<AppCursor>,
    dispatch: Option<String>,
    finished: bool,
}
impl AppEventPump {
    pub(crate) fn new() -> Self {
        Self(Arc::new(Mutex::new(State {
            in_flight: false,
            wake: true,
            next: Instant::now(),
            delivery: None,
            maintenance: None,
            dispatch: None,
        })))
    }
    pub(crate) fn wake(&self) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .wake = true;
    }
    pub(crate) fn try_begin(&self) -> Option<AppEventPass> {
        self.try_begin_at(Instant::now())
    }
    fn try_begin_at(&self, now: Instant) -> Option<AppEventPass> {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.in_flight || now < state.next {
            return None;
        }
        state.in_flight = true;
        state.wake = false;
        Some(AppEventPass {
            state: self.0.clone(),
            delivery: state.delivery.clone(),
            maintenance: state.maintenance.clone(),
            dispatch: state.dispatch.clone(),
            finished: false,
        })
    }
}
impl AppEventPass {
    pub(crate) fn delivery_after(&self) -> Option<(&str, &str)> {
        self.delivery
            .as_ref()
            .map(|(a, b)| (a.as_str(), b.as_str()))
    }
    pub(crate) fn maintenance_after(&self) -> Option<(&str, &str)> {
        self.maintenance
            .as_ref()
            .map(|(a, b)| (a.as_str(), b.as_str()))
    }
    pub(crate) fn dispatch_after(&self) -> Option<&str> {
        self.dispatch.as_deref()
    }
    pub(crate) fn finish(
        mut self,
        delivery: Option<AppCursor>,
        maintenance: Option<AppCursor>,
        dispatch: Option<String>,
        _more: bool,
    ) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.delivery = delivery;
        state.maintenance = maintenance;
        state.dispatch = dispatch;
        // Backlog and wakes cannot bypass the monotonic interval. In particular,
        // an unsubmitted Ready run blocked on a workspace must not busy-loop.
        state.next = Instant::now() + Duration::from_secs(1);
        state.in_flight = false;
        self.finished = true;
    }
}
impl Drop for AppEventPass {
    fn drop(&mut self) {
        if !self.finished {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.in_flight = false;
            state.next = Instant::now() + Duration::from_secs(1);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_pass_keeps_wakes_and_cursors_across_cancellation() {
        let pump = AppEventPump::new();
        let clone = pump.clone();
        let first = pump.try_begin().unwrap();
        assert!(clone.try_begin().is_none());
        clone.wake();
        first.finish(
            Some(("owner".into(), "app".into())),
            None,
            Some("session".into()),
            false,
        );
        assert!(clone.try_begin().is_none());
        let next = clone
            .try_begin_at(Instant::now() + Duration::from_secs(2))
            .unwrap();
        assert_eq!(next.delivery_after(), Some(("owner", "app")));
        assert_eq!(next.dispatch_after(), Some("session"));
        drop(next);
        assert!(pump.try_begin().is_none());
        let next = pump
            .try_begin_at(Instant::now() + Duration::from_secs(2))
            .unwrap();
        assert_eq!(next.delivery_after(), Some(("owner", "app")));
        next.finish(None, None, None, true);
        assert!(pump.try_begin().is_none());
        pump.wake();
        assert!(pump.try_begin().is_none());
        assert!(pump
            .try_begin_at(Instant::now() + Duration::from_secs(2))
            .is_some());
    }
}
