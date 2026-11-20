use crate::error::DaemonError;
use std::fmt;

#[derive(Debug)]
pub(crate) struct ReplayFailure {
    record: Option<ReplayRecord>,
    pub(crate) error: Box<DaemonError>,
}

#[derive(Debug)]
struct ReplayRecord {
    kind: String,
    event_id: Option<String>,
    sequence: u64,
}

impl ReplayFailure {
    pub(crate) fn at_event(
        kind: String,
        event_id: String,
        sequence: u64,
        error: DaemonError,
    ) -> Self {
        Self {
            record: Some(ReplayRecord {
                kind,
                event_id: Some(event_id),
                sequence,
            }),
            error: Box::new(error),
        }
    }

    pub(crate) fn at_snapshot(sequence: u64, error: DaemonError) -> Self {
        Self {
            record: Some(ReplayRecord {
                kind: "kernel checkpoint".into(),
                event_id: None,
                sequence,
            }),
            error: Box::new(error),
        }
    }
}

impl From<DaemonError> for ReplayFailure {
    fn from(error: DaemonError) -> Self {
        Self {
            record: None,
            error: Box::new(error),
        }
    }
}

impl fmt::Display for ReplayFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(record) = &self.record {
            write!(f, "record kind {:?}", record.kind)?;
            if let Some(id) = &record.event_id {
                write!(f, ", id {id:?}")?;
            }
            write!(f, ", sequence {}. ", record.sequence)?;
        }
        write!(f, "{}", self.error)
    }
}
