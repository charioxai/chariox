//! Hickory runtime adapter: retain every driver instead of detaching it.
use super::super::limits::LifetimeLease;
use hickory_resolver::net::runtime::{RuntimeProvider, Spawn, TokioRuntimeProvider};
use std::{
    future::Future,
    io,
    net::SocketAddr,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::task::JoinSet;

const MAX_DNS_TASKS: usize = 16;
#[derive(Default)]
struct State {
    closed: bool,
    overloaded: bool,
    tasks: JoinSet<()>,
    lease: Option<LifetimeLease>,
}
#[derive(Clone, Default)]
pub(super) struct Handle(Arc<Mutex<State>>);
pub(super) struct TaskOwner(Handle);
#[derive(Clone)]
pub(super) struct Provider {
    base: TokioRuntimeProvider,
    handle: Handle,
}

impl TaskOwner {
    pub(super) fn new(lease: LifetimeLease) -> Self {
        Self(Handle(Arc::new(Mutex::new(State {
            lease: Some(lease),
            ..State::default()
        }))))
    }
    pub(super) fn provider(&self) -> Provider {
        Provider {
            base: TokioRuntimeProvider::new(),
            handle: self.0.clone(),
        }
    }
    pub(super) fn overloaded(&self) -> bool {
        self.0 .0.lock().unwrap().overloaded
    }
    pub(super) async fn finish(self) {
        let mut tasks = {
            let mut state = self.0 .0.lock().unwrap();
            state.closed = true;
            state.lease.take();
            std::mem::take(&mut state.tasks)
        };
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }
}
impl Drop for TaskOwner {
    fn drop(&mut self) {
        // Panic fallback stops I/O; ordinary completion always joins in finish.
        let mut state = self.0 .0.lock().unwrap();
        state.closed = true;
        state.tasks.abort_all();
        state.lease.take();
    }
}
impl Spawn for Handle {
    fn spawn_bg(&mut self, future: impl Future<Output = ()> + Send + 'static) {
        let mut state = self.0.lock().unwrap();
        while state.tasks.try_join_next().is_some() {}
        if state.closed {
            return;
        }
        if state.tasks.len() >= MAX_DNS_TASKS {
            state.overloaded = true;
            state.closed = true;
            state.tasks.abort_all();
            return;
        }
        let lease = state.lease.clone();
        state.tasks.spawn(async move {
            future.await;
            drop(lease);
        });
    }
}
impl RuntimeProvider for Provider {
    type Handle = Handle;
    type Timer = <TokioRuntimeProvider as RuntimeProvider>::Timer;
    type Udp = <TokioRuntimeProvider as RuntimeProvider>::Udp;
    type Tcp = <TokioRuntimeProvider as RuntimeProvider>::Tcp;
    fn create_handle(&self) -> Self::Handle {
        self.handle.clone()
    }
    fn connect_tcp(
        &self,
        address: SocketAddr,
        bind: Option<SocketAddr>,
        timeout: Option<Duration>,
    ) -> Pin<Box<dyn Future<Output = io::Result<Self::Tcp>> + Send>> {
        self.base.connect_tcp(address, bind, timeout)
    }
    fn bind_udp(
        &self,
        local: SocketAddr,
        server: SocketAddr,
    ) -> Pin<Box<dyn Future<Output = io::Result<Self::Udp>> + Send>> {
        self.base.bind_udp(local, server)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[tokio::test]
    async fn finish_joins_drivers_and_closed_handles_cannot_spawn() {
        struct Dropped(Arc<AtomicUsize>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let limits = super::super::super::limits::HttpLimits::default();
        let owner = TaskOwner::new(limits.acquire("alice", "app").unwrap());
        let mut handle = owner.provider().create_handle();
        let dropped = Arc::new(AtomicUsize::new(0));
        for _ in 0..MAX_DNS_TASKS {
            let probe = Dropped(dropped.clone());
            handle.spawn_bg(async move {
                let _probe = probe;
                std::future::pending::<()>().await
            });
        }
        owner.finish().await;
        assert_eq!(dropped.load(Ordering::SeqCst), MAX_DNS_TASKS);
        let probe = Dropped(dropped.clone());
        handle.spawn_bg(async move {
            let _probe = probe;
        });
        assert_eq!(dropped.load(Ordering::SeqCst), MAX_DNS_TASKS + 1);
    }
}
