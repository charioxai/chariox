//! MP-08 / MP-09 / MP-10 / MP-11: observations and their cursor commit together.
use super::*;

pub(super) fn record(
    tx: &Transaction<'_>,
    id: &str,
    kernel: &str,
    online: bool,
    reason: String,
) -> Result<(), DaemonError> {
    let Some(mut part) = read(tx, id)? else {
        return Ok(());
    };
    // An in-flight observation may finish after a target was removed or its
    // grant ended. Only its still-followed home inbox may receive it.
    if !part.home || part.ended.is_some() || !part.targets.iter().any(|t| t.kernel == kernel) {
        return Ok(());
    }
    let previous = part.kernel_health.insert(kernel.into(), online);
    if previous == Some(online) {
        return Ok(());
    }
    if previous.is_some() || !online {
        let event = OutsideEvent {
            cursor: 0,
            kernel: kernel.into(),
            machine: String::new(),
            session_id: String::new(),
            agent_id: None,
            value: OutsideEventValue::KernelHealth { online, reason },
            summary: if online {
                "kernel online"
            } else {
                "kernel offline"
            }
            .into(),
            at_ms: outside_now_ms()?,
            refs: Default::default(),
        };
        append(
            tx,
            &mut part,
            &format!("health-{:032x}", rand::random::<u128>()),
            &event,
        )?;
        bound(tx, &mut part, inbox_limit()?)?;
    }
    put(tx, &part)
}
