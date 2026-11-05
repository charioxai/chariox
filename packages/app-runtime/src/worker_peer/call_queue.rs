use super::*;

impl Actor {
    pub(super) fn dispatch_calls(&mut self) -> Result<()> {
        // A sent cancellation still occupies the guest until its terminal reply.
        // Never send call 65 while the shared instance is retiring that work.
        let now = Instant::now();
        for _ in 0..self.waiting.len() {
            let call = self.waiting.pop_front().expect("queued call");
            if call.reply.is_closed() {
                continue;
            }
            if now >= call.deadline {
                let _ = call.reply.send(Err(PeerError::Deadline));
                continue;
            }
            self.waiting.push_back(call);
        }
        while self.pending.len() + self.cancelled.len() < self.limits.pending_calls {
            let Some(call) = self.waiting.pop_front() else {
                break;
            };
            self.start_call(call)?;
        }
        Ok(())
    }
    fn start_call(&mut self, call: Call) -> Result<()> {
        if call.reply.is_closed() {
            return Ok(());
        }
        if Instant::now() >= call.deadline {
            let _ = call.reply.send(Err(PeerError::Deadline));
            return Ok(());
        }
        let Message::Request {
            id,
            method,
            context,
            ..
        } = &call.message
        else {
            return Err(PeerError::Invalid);
        };
        let (id, method) = (id.clone(), method.clone());
        let actor = context
            .as_ref()
            .and_then(|context| context.get("actor"))
            .cloned();
        if self.pending.contains_key(&id) || self.pending.len() >= self.limits.pending_calls {
            return Err(PeerError::Protocol);
        }
        let live = Arc::new(FrameState::new());
        self.send(call.message, Some(live.clone()))?;
        self.pending.insert(
            id,
            Pending {
                method,
                reply: call.reply,
                deadline: call.deadline,
                live,
                actor,
                _permit: call.permit,
            },
        );
        Ok(())
    }
}
