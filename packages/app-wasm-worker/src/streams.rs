//! Bounded stream bridges. Producers stop on guest drop and on invocation end.
use crate::{
    bindings::chariox::app::app_types::{AppError, LiveEvent},
    broker::context,
    host::{app_error, Host},
    ipc::error,
    scope::Scope,
};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    task::{Context, Poll},
};
use tokio::sync::mpsc;
use wasmtime::{
    component::{
        Accessor, Destination, HasSelf, Source, StreamConsumer, StreamProducer, StreamReader,
        StreamResult, VecBuffer,
    },
    StoreContextMut,
};

pub const CHUNK_BYTES: usize = 64 * 1024;
pub const READ_AHEAD: usize = 4;
pub type ByteReceiver = mpsc::Receiver<Result<Vec<u8>, String>>;
type Cancel = Pin<Box<dyn Future<Output = &'static str> + Send>>;
fn cancellation(scope: Scope) -> Cancel {
    Box::pin(async move { scope.cancelled().await })
}

pub struct Bytes {
    pub receiver: ByteReceiver,
    cancel: Cancel,
}
impl Bytes {
    pub fn new(receiver: ByteReceiver, scope: Scope) -> Self {
        Self {
            receiver,
            cancel: cancellation(scope),
        }
    }
}
impl<D> StreamProducer<D> for Bytes {
    type Item = u8;
    type Buffer = VecBuffer<u8>;
    fn poll_produce<'a>(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        _store: StoreContextMut<'a, D>,
        mut destination: Destination<'a, u8, Self::Buffer>,
        finish: bool,
    ) -> Poll<wasmtime::Result<StreamResult>> {
        let this = self.get_mut();
        if this.cancel.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Ok(StreamResult::Dropped));
        }
        if finish {
            return Poll::Ready(Ok(StreamResult::Cancelled));
        }
        match this.receiver.poll_recv(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                destination.set_buffer(VecBuffer::from(chunk));
                Poll::Ready(Ok(StreamResult::Completed))
            }
            Poll::Ready(Some(Err(message))) => {
                let error: wasmtime::Error = crate::limits::Stop("APP_HTTP_STREAM_FAILED").into();
                Poll::Ready(Err(error.context(message)))
            }
            Poll::Ready(None) => Poll::Ready(Ok(StreamResult::Dropped)),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// A single chunk may wait for its broker acknowledgement. No guest bytes
/// are consumed until a bounded channel slot has been reserved.
type UploadReservation = Pin<
    Box<dyn Future<Output = Result<mpsc::OwnedPermit<Vec<u8>>, mpsc::error::SendError<()>>> + Send>,
>;

pub struct Upload {
    sender: mpsc::Sender<Vec<u8>>,
    reserve: Option<UploadReservation>,
    cancel: Cancel,
}
impl Upload {
    pub fn new(sender: mpsc::Sender<Vec<u8>>, scope: Scope) -> Self {
        Self {
            sender,
            reserve: None,
            cancel: cancellation(scope),
        }
    }
}
impl<D> StreamConsumer<D> for Upload {
    type Item = u8;
    fn poll_consume(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mut store: StoreContextMut<D>,
        mut source: Source<'_, u8>,
        finish: bool,
    ) -> Poll<wasmtime::Result<StreamResult>> {
        let this = self.get_mut();
        if this.cancel.as_mut().poll(cx).is_ready() || this.sender.is_closed() {
            return Poll::Ready(Ok(StreamResult::Dropped));
        }
        if finish {
            this.reserve = None;
            return Poll::Ready(Ok(StreamResult::Cancelled));
        }
        if this.reserve.is_none() {
            this.reserve = Some(Box::pin(this.sender.clone().reserve_owned()));
        }
        let permit = match this.reserve.as_mut().unwrap().as_mut().poll(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(_)) => return Poll::Ready(Ok(StreamResult::Dropped)),
            Poll::Ready(Ok(permit)) => permit,
        };
        this.reserve = None;
        let mut buffer = Vec::with_capacity(CHUNK_BYTES);
        source.read(&mut store, &mut buffer)?;
        if !buffer.is_empty() {
            permit.send(buffer);
        }
        Poll::Ready(Ok(StreamResult::Completed))
    }
}

struct Subscription {
    name: String,
    scope: Scope,
    sender: mpsc::Sender<LiveEvent>,
}
#[derive(Default, Clone)]
pub struct Subscriptions(Arc<Mutex<HashMap<u64, Subscription>>>, Arc<AtomicU64>);
impl Subscriptions {
    pub async fn deliver(&self, name: &str, data: String) -> Result<(), AppError> {
        let subscriptions: Vec<_> = self
            .0
            .lock()
            .unwrap()
            .values()
            .filter(|item| item.name == name && item.scope.reason().is_none())
            .map(|item| (item.scope.clone(), item.sender.clone()))
            .collect();
        if subscriptions.is_empty() {
            return Err(app_error(error("NO_SUBSCRIBER", "no live subscription")));
        }
        futures::future::try_join_all(subscriptions.into_iter().map(|(scope,sender)| {
            let event=LiveEvent { name:name.into(),data:data.clone() };
            async move {
                tokio::select! {
                    biased;
                    _=scope.cancelled()=>Err(app_error(error("CANCELLED","subscription ended"))),
                    result=sender.send(event)=>result.map_err(|_|app_error(error("NO_SUBSCRIBER","subscription closed"))),
                }
            }
        })).await?;
        Ok(())
    }
}
struct Events {
    receiver: mpsc::Receiver<LiveEvent>,
    cancel: Cancel,
    subscriptions: Subscriptions,
    id: u64,
}
impl Drop for Events {
    fn drop(&mut self) {
        self.subscriptions.0.lock().unwrap().remove(&self.id);
    }
}
impl<D> StreamProducer<D> for Events {
    type Item = LiveEvent;
    type Buffer = VecBuffer<LiveEvent>;
    fn poll_produce<'a>(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        _store: StoreContextMut<'a, D>,
        mut destination: Destination<'a, LiveEvent, Self::Buffer>,
        finish: bool,
    ) -> Poll<wasmtime::Result<StreamResult>> {
        let this = self.get_mut();
        if this.cancel.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Ok(StreamResult::Dropped));
        }
        if finish {
            return Poll::Ready(Ok(StreamResult::Cancelled));
        }
        match this.receiver.poll_recv(cx) {
            Poll::Ready(Some(item)) => {
                destination.set_buffer(VecBuffer::from(vec![item]));
                Poll::Ready(Ok(StreamResult::Completed))
            }
            Poll::Ready(None) => Poll::Ready(Ok(StreamResult::Dropped)),
            Poll::Pending => Poll::Pending,
        }
    }
}
pub fn subscribe<T: Send>(
    accessor: &Accessor<T, HasSelf<Host>>,
    name: String,
) -> Result<StreamReader<LiveEvent>, AppError> {
    let (_, scope) = context(accessor)?;
    let slot = accessor
        .with(|mut access| access.get().stream_slots.clone().try_acquire_owned())
        .map_err(|_| app_error(error("APP_RESOURCE_LIMIT", "too many open streams")))?;
    let subscriptions = accessor.with(|mut access| access.get().subscriptions.clone());
    let id = subscriptions.1.fetch_add(1, Ordering::Relaxed);
    let (sender, receiver) = mpsc::channel(16);
    let closed = sender.clone();
    {
        let mut items = subscriptions.0.lock().unwrap();
        if items.len() >= 128 {
            return Err(app_error(error("APP_BUSY", "too many live subscriptions")));
        }
        items.insert(
            id,
            Subscription {
                name,
                scope: scope.clone(),
                sender,
            },
        );
    }
    let cleanup = subscriptions.clone();
    let watch = scope.clone();
    tokio::spawn(async move {
        tokio::select! { _=watch.cancelled()=>{}, _=closed.closed()=>{} }
        cleanup.0.lock().unwrap().remove(&id);
        drop(slot);
    });
    accessor
        .with(|mut access| {
            StreamReader::new(
                &mut access,
                Events {
                    receiver,
                    cancel: cancellation(scope),
                    subscriptions,
                    id,
                },
            )
        })
        .map_err(|_| app_error(error("APP_RESOURCE_LIMIT", "stream table full")))
}
