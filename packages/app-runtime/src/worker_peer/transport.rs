use super::{Message, PeerError, Result};
use crate::wire::{Reader, Writer};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{mpsc, watch},
};

pub(super) struct Outgoing {
    pub message: Message,
    // A queued request cancelled before its first write must not start later.
    pub live: Option<Arc<AtomicBool>>,
    pub publication: Option<super::publication::Guarded>,
}

pub(super) async fn read<T: AsyncRead + Unpin>(
    mut reader: Reader<T>,
    incoming: mpsc::Sender<Result<Message>>,
    mut stopped: watch::Receiver<bool>,
    budget: Duration,
) {
    loop {
        let message = tokio::select! { biased;
            _ = stop(&mut stopped) => return,
            result = reader.receive(budget) => result.map_err(|_| PeerError::Io),
        };
        let failed = message.is_err();
        tokio::select! { biased;
            _ = stop(&mut stopped) => return,
            result = incoming.send(message) => if result.is_err() { return; },
        }
        if failed {
            return;
        }
    }
}

pub(super) async fn write<T: AsyncWrite + Unpin>(
    mut writer: Writer<T>,
    mut outgoing: mpsc::Receiver<Outgoing>,
    published: mpsc::Sender<super::publication::Ack>,
    incoming: mpsc::Sender<Result<Message>>,
    mut stopped: watch::Receiver<bool>,
    budget: Duration,
) {
    loop {
        let mut item = tokio::select! { biased;
            _ = stop(&mut stopped) => return,
            item = outgoing.recv() => match item {Some(item)=>item,None=>return},
        };
        if item
            .live
            .as_ref()
            .is_some_and(|live| !live.load(Ordering::Acquire))
        {
            continue;
        }
        if item
            .publication
            .as_ref()
            .is_some_and(|guard| guard.stopped())
        {
            let ack = item.publication.take().unwrap().complete(false);
            tokio::select! { biased;
                _ = stop(&mut stopped) => return,
                result = published.send(ack) => if result.is_err() { return; },
            }
            continue;
        }
        let result = if let Some(guard) = &mut item.publication {
            // Once any frame write may have started, cancellation closes both
            // halves. The channel can never continue from a partial reply.
            tokio::select! { biased;
                _ = stop(&mut stopped) => return,
                _ = stop(&mut guard.cancellation) => Err(crate::wire::WireError::Invalid("reply publication stopped")),
                _ = tokio::time::sleep_until(guard.deadline) => Err(crate::wire::WireError::Invalid("reply publication stopped")),
                result = writer.send(&item.message,budget) => result,
            }
        } else {
            tokio::select! { biased;
                _ = stop(&mut stopped) => return,
                result = writer.send(&item.message,budget) => result,
            }
        };
        if result.is_err() {
            tokio::select! { biased;
                _ = stop(&mut stopped) => {},
                _ = incoming.send(Err(PeerError::Io)) => {},
            }
            return;
        }
        if let Some(guard) = item.publication.take() {
            let ack = guard.complete(true);
            tokio::select! { biased;
                _ = stop(&mut stopped) => return,
                result = published.send(ack) => if result.is_err() { return; },
            }
        }
    }
}

pub(super) async fn stop(receiver: &mut watch::Receiver<bool>) {
    while !*receiver.borrow_and_update() {
        if receiver.changed().await.is_err() {
            return;
        }
    }
}
