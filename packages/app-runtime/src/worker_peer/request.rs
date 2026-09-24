use super::{Call, Message, PeerError, Result, WorkerPeer};
use crate::wire::{Sender, WIRE_VERSION};
use serde_json::Value;
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{
    sync::{oneshot, OwnedSemaphorePermit},
    time::Instant,
};

/// One already enqueued call. Dropping it closes the response receiver, which
/// asks the owning actor to cancel; transport and execution admission are not
/// evidence that an external effect stopped.
pub struct CallResponse {
    receive: oneshot::Receiver<Result<Message>>,
}
impl Future for CallResponse {
    type Output = Result<Message>;
    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.receive).poll(context).map(|result| {
            result
                .map_err(|_| PeerError::Closed)
                .and_then(|result| result)
        })
    }
}

/// An opaque, single-use identity and admission reservation minted by this peer.
/// Dropping an unsent slot releases it without sending anything to the worker.
pub struct RequestSlot {
    peer: WorkerPeer,
    id: String,
    deadline_ms: u64,
    deadline: Instant,
    permit: OwnedSemaphorePermit,
}
impl RequestSlot {
    pub(super) fn new(
        peer: WorkerPeer,
        id: String,
        deadline_ms: u64,
        deadline: Instant,
        permit: OwnedSemaphorePermit,
    ) -> Self {
        Self {
            peer,
            id,
            deadline_ms,
            deadline,
            permit,
        }
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn deadline_ms(&self) -> u64 {
        self.deadline_ms
    }
    pub fn generation(&self) -> &str {
        &self.peer.control.generation
    }

    /// Convenience for lifecycle/broker operations. Tool calls normally use
    /// send with the immutable request constructed by AppCatalog::prepare.
    pub async fn request(
        self,
        method: &str,
        params: Value,
        context: Option<Value>,
    ) -> Result<Message> {
        let message = Message::Request {
            version: WIRE_VERSION,
            generation: self.generation().into(),
            id: self.id.clone(),
            method: method.into(),
            params,
            deadline_ms: self.deadline_ms,
            context,
        };
        self.send(message).await
    }

    /// The actor owns response routing. A caller may cancel by dropping this
    /// future; the actor notices the closed reply and sends a Cancel message.
    pub async fn send(self, message: Message) -> Result<Message> {
        self.submit(message)?.await
    }

    /// Enqueue while the kernel still holds its lifecycle/binding guard. After
    /// this returns the guard may be released before awaiting App code. This
    /// is bounded transport admission, never an operation authorization grant.
    pub fn submit(self, message: Message) -> Result<CallResponse> {
        if self.peer.is_closed() {
            return Err(PeerError::Closed);
        }
        if Instant::now() >= self.deadline {
            return Err(PeerError::Deadline);
        }
        match &message {
            Message::Request {
                id,
                generation,
                deadline_ms,
                ..
            } if id == &self.id
                && generation == self.generation()
                && *deadline_ms == self.deadline_ms => {}
            _ => return Err(PeerError::Invalid),
        }
        message
            .validate(self.generation(), Sender::Supervisor)
            .map_err(|_| PeerError::Invalid)?;
        super::validation::message(&message)?;
        let (reply, receive) = oneshot::channel();
        self.peer
            .control
            .commands
            .try_send(Call {
                message,
                deadline: self.deadline,
                reply,
                permit: self.permit,
            })
            .map_err(|error| match error {
                tokio::sync::mpsc::error::TrySendError::Full(_) => PeerError::Busy,
                tokio::sync::mpsc::error::TrySendError::Closed(_) => PeerError::Closed,
            })?;
        Ok(CallResponse { receive })
    }
}
