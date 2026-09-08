use super::{
    protocol, BrowserError, BrowserReference, BrowserSnapshot, BrowserTarget, Command, Operation,
    Reply, ScreencastFrame, COMMAND_TIMEOUT,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{mpsc, oneshot, watch},
    time::Instant,
};
use tokio_tungstenite::{tungstenite::Message, WebSocketStream};

pub(super) struct BrowserActor<S> {
    target: BrowserTarget,
    socket: WebSocketStream<S>,
    // Fields drop in declaration order: close the socket before releasing an
    // active input reservation or the managed browser's execution lease.
    reservation: Option<Arc<dyn Send + Sync>>,
    _execution_lease: Arc<dyn Send + Sync>,
    frames: watch::Sender<Option<Arc<ScreencastFrame>>>,
    stop: watch::Receiver<bool>,
    next_id: u64,
    document: Option<(String, String)>,
    revision: u64,
    epoch: u64,
}
impl<S: AsyncRead + AsyncWrite + Unpin> BrowserActor<S> {
    pub(super) fn new(
        target: BrowserTarget,
        epoch: u64,
        socket: WebSocketStream<S>,
        frames: watch::Sender<Option<Arc<ScreencastFrame>>>,
        stop: watch::Receiver<bool>,
        execution_lease: Arc<dyn Send + Sync>,
    ) -> Self {
        Self {
            target,
            socket,
            reservation: None,
            _execution_lease: execution_lease,
            frames,
            stop,
            next_id: 1,
            document: None,
            revision: 1,
            epoch,
        }
    }
    pub(super) async fn run(
        mut self,
        mut commands: mpsc::Receiver<Command>,
        ready: oneshot::Sender<Result<(), BrowserError>>,
    ) -> Result<(), BrowserError> {
        let startup = self.initialize().await;
        let _ = ready.send(startup);
        startup?;
        let result = loop {
            tokio::select! {
                biased;
                _ = self.stop.changed() => break Ok(()),
                command = commands.recv() => match command {
                    None => break Ok(()),
                    Some(command) => {
                        if command.response.is_closed() { continue }
                        if Instant::now() >= command.deadline {
                            let _ = command.response.send(Err(BrowserError::Deadline));
                            continue;
                        }
                        self.reservation = command._reservation.clone();
                        let result = self.execute(&command).await;
                        let fatal = matches!(result, Err(BrowserError::Protocol | BrowserError::Unavailable | BrowserError::Deadline | BrowserError::OutcomeUncertain));
                        if !fatal { self.reservation = None; }
                        let _ = command.response.send(result);
                        if fatal { break Err(BrowserError::Unavailable) }
                    }
                },
                message = self.socket.next() => {
                    let message = match Self::decode(message) { Ok(message) => message, Err(error) => break Err(error) };
                    if let Err(error) = self.event(message).await { break Err(error) }
                },
            }
        };
        commands.close();
        while let Ok(command) = commands.try_recv() {
            let _ = command.response.send(Err(BrowserError::Unavailable));
        }
        self.frames.send_replace(None);
        let _ = tokio::time::timeout(Duration::from_millis(250), self.socket.close(None)).await;
        result
    }
    async fn initialize(&mut self) -> Result<(), BrowserError> {
        let deadline = Instant::now() + COMMAND_TIMEOUT;
        let target = self
            .rpc(
                "Target.getTargetInfo",
                json!({"targetId":self.target.target_id}),
                deadline,
            )
            .await?;
        if target["targetInfo"]["targetId"].as_str() != Some(self.target.target_id.as_str())
            || target["targetInfo"]["type"].as_str() != Some("page")
        {
            return Err(BrowserError::Protocol);
        }
        self.rpc("Page.enable", json!({}), deadline).await?;
        self.refresh_document(deadline).await?;
        self.rpc("Accessibility.enable", json!({}), deadline)
            .await?;
        self.rpc("Page.startScreencast", json!({"format":"jpeg","quality":65,"maxWidth":1920,"maxHeight":1080,"everyNthFrame":2}), deadline).await?;
        Ok(())
    }
    async fn execute(&mut self, command: &Command) -> Result<Reply, BrowserError> {
        if Instant::now() >= command.deadline {
            return Err(BrowserError::Deadline);
        }
        self.refresh_document(command.deadline).await?;
        match &command.operation {
            Operation::Snapshot => {
                let reference = self.reference();
                let frame_id = &self.document.as_ref().ok_or(BrowserError::Unavailable)?.0;
                let result = self
                    .rpc(
                        "Accessibility.getFullAXTree",
                        json!({"frameId":frame_id,"depth":8}),
                        command.deadline,
                    )
                    .await?;
                if reference != self.reference() {
                    return Err(BrowserError::StaleReference);
                }
                Ok(Reply::Snapshot(BrowserSnapshot {
                    reference,
                    accessibility: protocol::accessibility(result)?,
                }))
            }
            Operation::Input(reference, input) => {
                if reference != &self.reference() {
                    return Err(BrowserError::StaleReference);
                }
                // Cancellation before transmission has no browser effect. Once
                // transmitted, the reservation stays in this command until drain.
                if command.response.is_closed() {
                    return Err(BrowserError::Cancelled);
                }
                let (method, params) = protocol::input(input);
                command.transmitted.store(true, Ordering::Release);
                self.rpc(method, params, command.deadline)
                    .await
                    .map_err(|_| BrowserError::OutcomeUncertain)?;
                self.refresh_document(command.deadline)
                    .await
                    .map_err(|_| BrowserError::OutcomeUncertain)?;
                if reference != &self.reference() {
                    return Err(BrowserError::OutcomeUncertain);
                }
                Ok(Reply::Input(self.reference()))
            }
        }
    }
    fn reference(&self) -> BrowserReference {
        BrowserReference {
            environment_id: self.target.environment_id.clone(),
            tab_id: self.target.tab_id.clone(),
            runtime_generation: self.target.runtime_generation,
            controller_epoch: self.epoch,
            document_revision: self.revision,
        }
    }
    fn document_changed(&mut self, next: (String, String)) -> Result<(), BrowserError> {
        if self
            .document
            .as_ref()
            .is_some_and(|current| current != &next)
        {
            self.revision = self.revision.checked_add(1).ok_or(BrowserError::Protocol)?;
            self.frames.send_replace(None);
        }
        self.document = Some(next);
        Ok(())
    }
    async fn refresh_document(&mut self, deadline: Instant) -> Result<(), BrowserError> {
        let result = self.rpc("Page.getFrameTree", json!({}), deadline).await?;
        self.document_changed(protocol::main_document(&result)?)
    }
    async fn write(&mut self, message: Value, deadline: Instant) -> Result<(), BrowserError> {
        let bytes = serde_json::to_string(&message).map_err(|_| BrowserError::Protocol)?;
        if bytes.len() > 32768 {
            return Err(BrowserError::Invalid);
        }
        tokio::select! {
            biased;
            _ = self.stop.changed() => Err(BrowserError::Unavailable),
            result = tokio::time::timeout_at(deadline, self.socket.send(Message::Text(bytes.into()))) =>
                result.map_err(|_| BrowserError::Deadline)?.map_err(|_| BrowserError::Unavailable),
        }
    }
    async fn rpc(
        &mut self,
        method: &str,
        params: Value,
        deadline: Instant,
    ) -> Result<Value, BrowserError> {
        let id = self.next_id;
        self.next_id = id.checked_add(1).ok_or(BrowserError::Protocol)?;
        self.write(json!({"id":id,"method":method,"params":params}), deadline)
            .await?;
        loop {
            let message = tokio::select! {
                biased;
                _ = self.stop.changed() => return Err(BrowserError::Unavailable),
                result = tokio::time::timeout_at(deadline, self.socket.next()) => Self::decode(result.map_err(|_| BrowserError::Deadline)?)?,
            };
            if message["id"].as_u64() == Some(id) {
                if message.get("error").is_some() {
                    return Err(BrowserError::Protocol);
                }
                return message
                    .get("result")
                    .filter(|value| value.is_object())
                    .cloned()
                    .ok_or(BrowserError::Protocol);
            }
            self.event(message).await?;
        }
    }
    fn decode(
        message: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>,
    ) -> Result<Value, BrowserError> {
        match message {
            Some(Ok(Message::Text(text))) => {
                serde_json::from_str(&text).map_err(|_| BrowserError::Protocol)
            }
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => Ok(json!({})),
            _ => Err(BrowserError::Unavailable),
        }
    }
    async fn event(&mut self, message: Value) -> Result<(), BrowserError> {
        let params = &message["params"];
        match message["method"].as_str() {
            Some("Page.frameNavigated") if params["frame"].get("parentId").is_none() => {
                let frame = &params["frame"];
                self.document_changed((
                    protocol::identifier(&frame["id"])?,
                    protocol::identifier(&frame["loaderId"])?,
                ))?;
            }
            Some("Page.navigatedWithinDocument")
                if self
                    .document
                    .as_ref()
                    .is_some_and(|(id, _)| params["frameId"].as_str() == Some(id.as_str())) =>
            {
                self.revision = self.revision.checked_add(1).ok_or(BrowserError::Protocol)?;
                self.frames.send_replace(None);
            }
            Some("Page.screencastFrame") => {
                let (frame, session) = protocol::frame(params, self.reference())?;
                // A one-frame watch slot cannot accumulate a slow viewer backlog.
                self.frames.send_replace(Some(Arc::new(frame)));
                let id = self.next_id;
                self.next_id = id.checked_add(1).ok_or(BrowserError::Protocol)?;
                self.write(json!({"id":id,"method":"Page.screencastFrameAck","params":{"sessionId":session}}), Instant::now()+Duration::from_secs(1)).await?;
            }
            Some("Inspector.detached" | "Inspector.targetCrashed") => {
                return Err(BrowserError::Unavailable)
            }
            _ => {}
        }
        Ok(())
    }
}

impl<S> Drop for BrowserActor<S> {
    fn drop(&mut self) {
        self.frames.send_replace(None);
    }
}
