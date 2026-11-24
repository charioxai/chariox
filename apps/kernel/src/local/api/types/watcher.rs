//! A watcher: a supervised script on this kernel that is a notification source of its owner.
use super::*;

/// How the kernel runs a watcher's command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum WatcherMode {
    /// Run every `every_s` seconds; each run reads, reports and exits.
    Poll { every_s: u64 },
    /// Stay running (tail, file watch, socket); restarted with backoff when it ends.
    Stream,
}

/// What happens to an event the source has not delivered yet when a newer one arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatcherCoalesce {
    #[default]
    Every,
    /// A newer event with the same key replaces a held one.
    LatestPerKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WatcherEventDeclaration {
    pub name: String,
    #[serde(default)]
    pub keyed: bool,
}

/// What the agent declares about a watcher. The person sees `name` and `sentence` only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WatcherManifest {
    pub name: String,
    pub sentence: String,
    pub mode: WatcherMode,
    /// The argv, run in the watcher's package directory.
    pub command: Vec<String>,
    pub events: Vec<WatcherEventDeclaration>,
    #[serde(default)]
    pub coalesce: WatcherCoalesce,
    /// Seconds of quiet the source waits for before it fires what it holds.
    #[serde(default)]
    pub debounce_s: u64,
}

/// The one file of a watcher's package besides its manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WatcherScript {
    pub file_name: String,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatcherState {
    /// Created or changed and not turned on: nothing runs on a schedule, nothing is delivered.
    Draft,
    Active,
    Paused,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum WatcherHealth {
    Ok,
    /// The last run failed, or the stream keeps ending; `reason` is for the agent that repairs it.
    Failing {
        since_ms: u64,
        reason: String,
    },
    /// No good run for three periods.
    Gap {
        since_ms: u64,
    },
    /// Over the rate limit: the source holds its events and fires one a minute.
    Noisy {
        since_ms: u64,
    },
}

/// One event a run reported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WatcherEvent {
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WatcherSummary {
    pub watcher_id: String,
    /// The notification source subscribers follow.
    pub source_id: String,
    pub kernel_id: String,
    pub session_id: String,
    pub creator_agent_id: Option<String>,
    pub manifest: WatcherManifest,
    pub state: WatcherState,
    pub health: WatcherHealth,
    /// What the source sees now, in words.
    pub reading: Option<String>,
    pub last_check_ms: Option<u64>,
    pub last_fired_ms: Option<u64>,
    pub fired_count: u64,
    /// The dry run of the current manifest and script, which turning it on requires.
    pub dry_run: Option<WatcherDryRun>,
}

/// What a watcher would have fired in the last 24 hours. Nothing was delivered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WatcherDryRun {
    pub reading: Option<String>,
    /// False when what it watches keeps no history: `events` then only says what it would fire now.
    pub history: bool,
    pub events: Vec<WatcherEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum WatcherRequest {
    Create {
        session_id: String,
        manifest: WatcherManifest,
        script: WatcherScript,
    },
    /// A change returns the watcher to draft: it applies once a dry run passed and it is activated.
    Update {
        watcher_id: String,
        manifest: WatcherManifest,
        script: WatcherScript,
    },
    DryRun {
        watcher_id: String,
    },
    Activate {
        watcher_id: String,
    },
    Pause {
        watcher_id: String,
        paused: bool,
    },
    CheckNow {
        watcher_id: String,
    },
    /// Names it without changing what it does, so its dry run still counts.
    Rename {
        watcher_id: String,
        name: String,
    },
    Remove {
        watcher_id: String,
    },
    List {},
}
