//! Exact SDK handler registration from the same verified package as its catalog.
//!
//! This proves only the registration report. It does not prove native confinement,
//! current publisher trust, activation, capability approval or a living process.
//! The kernel combines this single-use result with its owned worker and current
//! activation transaction before constructing a callable App handle.

use crate::app_catalog::AppCatalog;
use chariox_app_package::{EventDirection, VerifiedPackage};
use serde_json::Value;
use std::{collections::BTreeSet, sync::Arc};

const LIFECYCLE: [&str; 7] = [
    "health_check",
    "startup",
    "suspend",
    "resume",
    "shutdown",
    "prepare_update",
    "configuration_change",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReadinessError {
    #[error("app_readiness_provenance")]
    Provenance,
    #[error("app_readiness_invalid")]
    Invalid,
    #[error("app_readiness_already_reported")]
    AlreadyReported,
}
type Result<T> = std::result::Result<T, ReadinessError>;

pub struct ReadinessContract {
    catalog: Arc<AppCatalog>,
    tools: BTreeSet<String>,
    incoming_events: BTreeSet<String>,
    reported: bool,
}

/// An opaque, single-use handler report, not a running/authorized App. It is not
/// Clone and has no public constructor. The kernel activation owner retains it
/// together with WorkerProcess and PeerTask for their entire callable lifetime.
pub struct RegisteredHandlers {
    catalog: Arc<AppCatalog>,
    lifecycle: BTreeSet<String>,
}

impl ReadinessContract {
    pub fn from_verified(package: &VerifiedPackage<'_>, catalog: Arc<AppCatalog>) -> Result<Self> {
        if package.package_digest() != catalog.package_digest() {
            return Err(ReadinessError::Provenance);
        }
        let declarations = package.declarations();
        let tools = declarations
            .tools
            .iter()
            .map(|tool| tool.name.clone())
            .collect();
        let incoming_events = declarations
            .events
            .iter()
            .filter(|event| {
                matches!(
                    event.direction,
                    EventDirection::Incoming | EventDirection::Both
                )
            })
            .map(|event| event.name.clone())
            .collect();
        Ok(Self {
            catalog,
            tools,
            incoming_events,
            reported: false,
        })
    }

    /// These exact names also populate the trusted bootstrap configuration.
    /// Outgoing-only declarations are absent: emitting is not incoming handling.
    pub fn tools(&self) -> impl Iterator<Item = &str> {
        self.tools.iter().map(String::as_str)
    }
    pub fn incoming_events(&self) -> impl Iterator<Item = &str> {
        self.incoming_events.iter().map(String::as_str)
    }

    /// Process worker.ready once, after validating its channel generation. A bad
    /// report consumes the attempt too; the activation owner must stop the worker.
    /// Current installation/signer checks belong to the activation transaction.
    pub fn accept(&mut self, params: Value) -> Result<RegisteredHandlers> {
        if std::mem::replace(&mut self.reported, true) {
            return Err(ReadinessError::AlreadyReported);
        }
        let Value::Object(mut fields) = params else {
            return Err(ReadinessError::Invalid);
        };
        if fields.len() != 3
            || !["tools", "events", "lifecycle"]
                .iter()
                .all(|key| fields.contains_key(*key))
        {
            return Err(ReadinessError::Invalid);
        }
        let tools = names(
            fields.remove("tools").ok_or(ReadinessError::Invalid)?,
            self.tools.len(),
        )?;
        let events = names(
            fields.remove("events").ok_or(ReadinessError::Invalid)?,
            self.incoming_events.len(),
        )?;
        let lifecycle = names(
            fields.remove("lifecycle").ok_or(ReadinessError::Invalid)?,
            LIFECYCLE.len(),
        )?;
        if tools != self.tools
            || events != self.incoming_events
            || lifecycle
                .iter()
                .any(|name| !LIFECYCLE.contains(&name.as_str()))
        {
            return Err(ReadinessError::Invalid);
        }
        Ok(RegisteredHandlers {
            catalog: self.catalog.clone(),
            lifecycle,
        })
    }
}

impl RegisteredHandlers {
    pub fn catalog(&self) -> &Arc<AppCatalog> {
        &self.catalog
    }
    pub fn supports_lifecycle(&self, event: &str) -> bool {
        self.lifecycle.contains(event)
    }
}

fn names(value: Value, maximum: usize) -> Result<BTreeSet<String>> {
    let Value::Array(values) = value else {
        return Err(ReadinessError::Invalid);
    };
    if values.len() > maximum {
        return Err(ReadinessError::Invalid);
    }
    let mut result = BTreeSet::new();
    for value in values {
        let Value::String(name) = value else {
            return Err(ReadinessError::Invalid);
        };
        // Verified declaration names are at most 64 bytes. Reject oversized
        // reports before comparisons or collection insertion.
        if name.is_empty() || name.len() > 64 || !result.insert(name) {
            return Err(ReadinessError::Invalid);
        }
    }
    Ok(result)
}
