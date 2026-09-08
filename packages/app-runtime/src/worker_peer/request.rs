use super::{Call, Message, PeerError, Result, WorkerPeer};
use crate::wire::{Sender, WIRE_VERSION};
use serde_json::Value;
use tokio::{
    sync::{oneshot, OwnedSemaphorePermit},
    time::Instant,
};

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
        receive.await.map_err(|_| PeerError::Closed)?
    }
}
