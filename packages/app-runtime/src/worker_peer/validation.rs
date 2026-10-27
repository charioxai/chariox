use super::{Message, PeerError, Result};
use crate::wire::{Outcome, MAX_FRAME_BYTES};
use serde_json::Value;
use std::io::{self, Write};

/// Bound the pre-queue graph walk as well as serialized size. Wire decoding
/// still supplies duplicate-key, numeric, sender and generation validation.
pub(super) fn message(message: &Message) -> Result<()> {
    let mut budget = Budget {
        nodes: 262_144,
        bytes: MAX_FRAME_BYTES,
    };
    match message {
        Message::Request {
            params, context, ..
        } => {
            tree(params, 1, &mut budget)?;
            if let Some(context) = context {
                tree(context, 1, &mut budget)?;
            }
        }
        Message::Response {
            outcome: Outcome::Success(success),
            ..
        } => tree(&success.result, 1, &mut budget)?,
        Message::Event { data, .. } => tree(data, 1, &mut budget)?,
        _ => {}
    }
    serde_json::to_writer(Bounded(0), message).map_err(|_| PeerError::Invalid)
}
struct Budget {
    nodes: usize,
    bytes: usize,
}
impl Budget {
    fn take(&mut self, bytes: usize) -> Result<()> {
        self.bytes = self.bytes.checked_sub(bytes).ok_or(PeerError::Invalid)?;
        Ok(())
    }
}
fn tree(value: &Value, depth: usize, budget: &mut Budget) -> Result<()> {
    budget.nodes = budget.nodes.checked_sub(1).ok_or(PeerError::Invalid)?;
    budget.take(1)?;
    match value {
        Value::String(value) => budget.take(value.len())?,
        Value::Array(items) => {
            if depth >= 64 {
                return Err(PeerError::Invalid);
            }
            for value in items {
                tree(value, depth + 1, budget)?;
            }
        }
        Value::Object(items) => {
            if depth >= 64 {
                return Err(PeerError::Invalid);
            }
            for (key, value) in items {
                budget.take(key.len())?;
                tree(value, depth + 1, budget)?;
            }
        }
        Value::Number(number) => {
            let value = number.as_f64().ok_or(PeerError::Invalid)?;
            if !value.is_finite() || (value.fract() == 0.0 && value.abs() > 9_007_199_254_740_991.0)
            {
                return Err(PeerError::Invalid);
            }
        }
        _ => {}
    }
    Ok(())
}
struct Bounded(usize);
impl Write for Bounded {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_FRAME_BYTES - self.0 {
            return Err(io::Error::other("app_peer_frame_limit"));
        }
        self.0 += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
