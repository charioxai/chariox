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
    incoming: mpsc::Sender<Result<Message>>,
    mut stopped: watch::Receiver<bool>,
    budget: Duration,
) {
    loop {
        let item = tokio::select! { biased;
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
        let result = tokio::select! { biased;
            _ = stop(&mut stopped) => return,
            result = writer.send(&item.message,budget) => result,
        };
        if result.is_err() {
            tokio::select! { biased;
                _ = stop(&mut stopped) => {},
                _ = incoming.send(Err(PeerError::Io)) => {},
            }
            return;
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
