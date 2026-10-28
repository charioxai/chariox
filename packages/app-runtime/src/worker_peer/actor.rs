use super::{
    transport::{self, FrameState, Outgoing},
    validation, wall_ms, Broker, BrokerCancellation, BrokerRequest, Call, CancellationCause,
    Channel, ControlEvent, Message, PeerError, PeerLimits, RemoteError, Result,
};
use crate::wire::{Failure, Outcome, Success, WIRE_VERSION};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{mpsc, oneshot, watch, OwnedSemaphorePermit},
    task::JoinSet,
    time::Instant,
};

#[path = "broker_queue.rs"]
mod broker_queue;
#[path = "call_queue.rs"]
mod call_queue;

struct Pending {
    method: String,
    reply: oneshot::Sender<Result<Message>>,
    deadline: Instant,
    live: Arc<FrameState>,
    /// The caller the kernel named in this call's context.
    actor: Option<serde_json::Value>,
    _permit: OwnedSemaphorePermit,
}
struct Cancelled {
    deadline: Instant,
    _permit: OwnedSemaphorePermit,
}

struct Active {
    cancel: watch::Sender<bool>,
    cancellation_cause: Arc<AtomicU8>,
    deadline: Instant,
    replied: bool,
}
struct Actor {
    generation: String,
    limits: PeerLimits,
    broker: Arc<dyn Broker>,
    queued: VecDeque<Outgoing>,
    events: mpsc::Sender<ControlEvent>,
    pending: BTreeMap<String, Pending>,
    waiting: VecDeque<Call>,
    broker_waiting: VecDeque<broker_queue::Waiting>,
    publishing: BTreeMap<String, super::publication::Pending>,
    active: BTreeMap<String, Active>,
    handlers: JoinSet<(String, std::result::Result<serde_json::Value, RemoteError>)>,
    /// Calls that expired or were cancelled, until the worker's terminal
    /// response for each arrives. The SDK answers every call it holds with one,
    /// even when its handler ignores cancellation; none within the grace means
    /// the worker's event loop is blocked.
    cancelled: BTreeMap<String, Cancelled>,
}

const UNRESPONSIVE_GRACE: Duration = Duration::from_secs(5);

#[allow(clippy::too_many_arguments)]
pub(super) async fn run<T>(
    channel: Channel<T>,
    generation: String,
    broker: Arc<dyn Broker>,
    limits: PeerLimits,
    mut commands: mpsc::Receiver<Call>,
    events: mpsc::Sender<ControlEvent>,
    stopped: watch::Sender<bool>,
    mut stop_receiver: watch::Receiver<bool>,
) -> Result<()>
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (reader, writer) = channel.split();
    let (incoming_tx, mut incoming) = mpsc::channel(limits.queued_frames);
    let (outgoing, outgoing_rx) = mpsc::channel(limits.queued_frames);
    let (published_tx, mut published) = mpsc::channel(limits.broker_handlers);
    let read_task = tokio::spawn(transport::read(
        reader,
        incoming_tx.clone(),
        stop_receiver.clone(),
        limits.frame_timeout,
    ));
    let write_task = tokio::spawn(transport::write(
        writer,
        outgoing_rx,
        published_tx,
        incoming_tx,
        stop_receiver.clone(),
        limits.frame_timeout,
    ));
    let mut actor = Actor {
        generation,
        limits,
        broker,
        queued: VecDeque::new(),
        events,
        pending: BTreeMap::new(),
        waiting: VecDeque::new(),
        broker_waiting: VecDeque::new(),
        publishing: BTreeMap::new(),
        active: BTreeMap::new(),
        handlers: JoinSet::new(),
        cancelled: BTreeMap::new(),
    };
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let result = loop {
        // Give an explicit terminal close priority without starving any of the
        // normal actor inputs. Closed transport mailboxes are a consequence of
        // that close, not a new I/O failure racing its completion result.
        let result = tokio::select! { biased;
            _=transport::stop(&mut stop_receiver)=>break Ok(()),
            result=async {tokio::select! {
            ack=published.recv()=>match ack {Some(ack)=>actor.published(ack),None=>Err(PeerError::Io)},
            command=commands.recv()=> match command {Some(command)=>{actor.waiting.push_back(command);Ok(())},None=>Err(PeerError::Closed)},
            message=incoming.recv()=>match message {Some(Ok(message))=>actor.receive(message),Some(Err(error))=>Err(error),None=>Err(PeerError::Io)},
            completed=actor.handlers.join_next(), if !actor.handlers.is_empty()=>match completed {
                Some(Ok((id,outcome)))=>actor.complete(id,outcome),
                _=>Err(PeerError::Broker),
            },
            permit=outgoing.reserve(), if !actor.queued.is_empty()=>match permit {
                Ok(permit)=>{permit.send(actor.queued.pop_front().expect("queued frame"));Ok(())},
                Err(_)=>Err(PeerError::Io),
            },
            _=tick.tick()=>actor.expire(),
            }}=>result,
        };
        let result = result
            .and_then(|()| actor.dispatch_calls())
            .and_then(|()| actor.dispatch_brokers());
        if let Err(error) = result {
            break if *stop_receiver.borrow() {
                Ok(())
            } else {
                Err(error)
            };
        }
    };
    stopped.send_replace(true);
    commands.close();
    // Destroy queued requests and settle pending callers before awaiting broker
    // completion. No read/write task is left orphaned after the peer closes.
    while let Ok(call) = commands.try_recv() {
        let _ = call.reply.send(Err(PeerError::Closed));
    }
    for (_, pending) in std::mem::take(&mut actor.pending) {
        pending.live.retire();
        let _ = pending.reply.send(Err(PeerError::Closed));
    }
    actor.waiting.clear();
    actor.discard_waiting_brokers();
    actor.queued.clear();
    for active in actor.active.values() {
        active.cancel.send_replace(true);
    }
    for pending in actor.publishing.values() {
        pending.cancel.send_replace(true);
    }
    actor.broker.begin_draining();
    let _ = read_task.await;
    let _ = write_task.await;
    // Deliberately do not abort these futures: a broker may already be committing
    // a transaction. The kernel owner retains admission until actual completion.
    while actor.handlers.join_next().await.is_some() {}
    actor.broker.drain().await;
    result
}

impl Actor {
    fn send(&mut self, message: Message, live: Option<Arc<FrameState>>) -> Result<()> {
        validation::message(&message)?;
        let bound =
            self.limits.pending_calls + 2 * self.limits.broker_handlers + self.limits.queued_frames;
        if self.queued.len() >= bound {
            return Err(PeerError::Busy);
        }
        self.queued.push_back(Outgoing {
            message,
            live,
            publication: None,
        });
        Ok(())
    }
    fn response(&mut self, id: String, outcome: Outcome) -> Result<()> {
        self.send(
            Message::Response {
                version: WIRE_VERSION,
                generation: self.generation.clone(),
                id,
                outcome,
            },
            None,
        )
    }
    fn failure(&mut self, id: String, code: &str, message: &str) -> Result<()> {
        self.response(
            id,
            Outcome::Failure(Failure {
                error: RemoteError {
                    code: code.into(),
                    message: message.into(),
                    retryable: Some(code == "BUSY"),
                },
            }),
        )
    }
    fn receive(&mut self, message: Message) -> Result<()> {
        message
            .validate(&self.generation, crate::wire::Sender::Worker)
            .map_err(|_| PeerError::Protocol)?;
        match message {
            Message::Response { ref id, .. } => {
                // Match SDK behavior: never revive a retired or unknown call,
                // and never grow an unbounded collection of response tombstones.
                if let Some(pending) = self.pending.remove(id) {
                    pending.live.retire();
                    let result = if Instant::now() >= pending.deadline {
                        Err(PeerError::Deadline)
                    } else {
                        Ok(message)
                    };
                    let _ = pending.reply.send(result);
                } else {
                    self.cancelled.remove(id);
                }
                Ok(())
            }
            Message::Request {
                id,
                method,
                params,
                deadline_ms,
                ..
            } => self.broker_request(id, method, params, deadline_ms),
            Message::Cancel { id, .. } => {
                self.cancel_handler(&id, "CANCELLED", "App broker call cancelled")
            }
            Message::Event { name, data, .. } => {
                if !name.starts_with("worker.") {
                    return Err(PeerError::Protocol);
                }
                self.events
                    .try_send(ControlEvent { name, data })
                    .map_err(|_| PeerError::Busy)
            }
        }
    }
    fn cancel_handler(&mut self, id: &str, code: &str, message: &str) -> Result<()> {
        if self.cancel_waiting_broker(id, code)? {
            return Ok(());
        }
        if let Some(pending) = self.publishing.get_mut(id) {
            let _completion = pending
                .completion
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if pending.failure.is_some() || pending.finished.load(Ordering::Acquire) {
                return Ok(());
            }
            pending.failure = Some(RemoteError {
                code: code.into(),
                message: message.into(),
                retryable: Some(false),
            });
            pending.cancel.send_replace(true);
            // The writer chooses one outcome. Sending failure here could race
            // a complete success frame and duplicate this response identity.
            return Ok(());
        }
        let Some(active) = self.active.get_mut(id) else {
            return Ok(());
        };
        // Keep the first cause: an explicit cancellation that precedes the
        // timer must not be relabelled when maintenance later visits this id.
        if !active.replied {
            let cause = match code {
                "DEADLINE_EXCEEDED" => CancellationCause::Deadline,
                "CANCELLED" => CancellationCause::Request,
                _ => CancellationCause::Teardown,
            };
            active
                .cancellation_cause
                .store(cause as u8, Ordering::Release);
        }
        active.cancel.send_replace(true);
        if active.replied {
            return Ok(());
        }
        active.replied = true;
        self.failure(id.into(), code, message)
    }
    fn complete(
        &mut self,
        id: String,
        outcome: std::result::Result<serde_json::Value, RemoteError>,
    ) -> Result<()> {
        let guard = self.broker.take_response_guard(&id);
        let Some(active) = self.active.remove(&id) else {
            return Err(PeerError::Protocol);
        };
        if active.replied {
            return Ok(());
        }
        if Instant::now() >= active.deadline {
            return self.failure(id, "DEADLINE_EXCEEDED", "App broker deadline exceeded");
        }
        let outcome = match outcome {
            Ok(result) => Outcome::Success(Success { result }),
            Err(error) => Outcome::Failure(Failure { error }),
        };
        if let Some(guard) = guard {
            let finished = Arc::new(AtomicBool::new(false));
            let completion = Arc::new(std::sync::Mutex::new(()));
            let physically_published = Arc::new(AtomicBool::new(false));
            let publication = super::publication::Guarded {
                completion: completion.clone(),
                physically_published: physically_published.clone(),
                id: id.clone(),
                deadline: active.deadline,
                cancellation: active.cancel.subscribe(),
                finished: finished.clone(),
                guard,
            };
            self.response(id.clone(), outcome)?;
            self.queued.back_mut().expect("new response").publication = Some(publication);
            self.publishing.insert(
                id,
                super::publication::Pending {
                    cancel: active.cancel,
                    deadline: active.deadline,
                    failure: None,
                    completion,
                    physically_published,
                    finished,
                },
            );
            Ok(())
        } else {
            self.response(id, outcome)
        }
    }
    fn published(&mut self, ack: super::publication::Ack) -> Result<()> {
        if !self
            .publishing
            .get(&ack.id)
            .is_some_and(|pending| Arc::ptr_eq(&pending.finished, &ack.finished))
        {
            return Ok(());
        }
        let pending = self
            .publishing
            .remove(&ack.id)
            .expect("matched publication");
        if !ack.published {
            let error = pending.failure.unwrap_or(RemoteError {
                code: "DEADLINE_EXCEEDED".into(),
                message: "App broker reply was not published within its budget".into(),
                retryable: Some(false),
            });
            return self.response(ack.id, Outcome::Failure(Failure { error }));
        }
        Ok(())
    }
    fn expire(&mut self) -> Result<()> {
        let now = Instant::now();
        let expired = self
            .broker_waiting
            .iter()
            .filter(|entry| now >= entry.request.deadline)
            .map(|entry| entry.request.id.clone())
            .collect::<Vec<_>>();
        for id in expired {
            self.cancel_waiting_broker(&id, "DEADLINE_EXCEEDED")?;
        }
        let expired: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, entry)| entry.reply.is_closed() || now >= entry.deadline)
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            let pending = self
                .pending
                .remove(&id)
                .expect("collected live pending call");
            // Only a request the worker received owes a terminal response.
            let sent = pending.live.retire();
            if sent {
                self.cancelled.insert(
                    id.clone(),
                    Cancelled {
                        deadline: now + UNRESPONSIVE_GRACE,
                        _permit: pending._permit,
                    },
                );
            }
            let _ = pending.reply.send(Err(PeerError::Deadline));
            if sent {
                self.send(
                    Message::Cancel {
                        version: WIRE_VERSION,
                        generation: self.generation.clone(),
                        id,
                    },
                    None,
                )?;
            }
        }
        let expired: Vec<_> = self
            .active
            .iter()
            .filter(|(_, entry)| !entry.replied && now >= entry.deadline)
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            self.cancel_handler(&id, "DEADLINE_EXCEEDED", "App broker deadline exceeded")?;
        }
        let expired = self
            .publishing
            .iter()
            .filter(|(_, pending)| {
                pending.failure.is_none()
                    && !pending.finished.load(Ordering::Acquire)
                    && now >= pending.deadline
            })
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in expired {
            self.cancel_handler(
                &id,
                "DEADLINE_EXCEEDED",
                "App broker reply deadline exceeded",
            )?;
        }
        if self.cancelled.values().any(|entry| now >= entry.deadline) {
            return Err(PeerError::Unresponsive);
        }
        Ok(())
    }
}
