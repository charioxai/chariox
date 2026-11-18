//! The person's subscriptions on this kernel (docs/NOTIFICATION_CENTER_PLAN.md section 6): one
//! row per subscription whose subscriber lives here, with what it picked, its counters and its
//! health, and the requests that pause, move, test, trace and remove it.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListNotificationSubscriptionsRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateNotificationSubscriptionRequest {
    pub subscription_id: String,
    pub change: NotificationSubscriptionChange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum NotificationSubscriptionChange {
    /// Pause holds back delivery; the row, its counters and its trace stay.
    Paused { paused: bool },
    /// Another agent of the same Room takes the subscription over, with its prompt and options.
    Subscriber { agent_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoveNotificationSubscriptionRequest {
    pub subscription_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestNotificationSubscriptionRequest {
    pub subscription_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadNotificationTraceRequest {
    pub subscription_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationSubscriptionRow {
    pub subscription: WorkflowNotificationSubscription,
    /// The source as its kernel lists it; none when it is gone.
    pub source: Option<WorkflowNotificationSourceSummary>,
    /// The agent's name, or the workflow's alias; none when the subscriber is gone.
    pub subscriber_name: Option<String>,
    /// What an agent was asked to do when the notification arrives.
    pub prompt: Option<String>,
    pub options: Vec<NotificationOptionValue>,
    pub once: bool,
    pub paused: bool,
    pub fired: u64,
    pub last_fired_at_ms: Option<u64>,
    pub health: NotificationSubscriptionHealth,
}

/// The one thing the row needs the person to know, the most pressing first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSubscriptionHealth {
    Ok,
    Paused,
    /// A delivery came back to the subscription that caused it and was dropped.
    LoopStopped,
    /// The source no longer declares an option the subscription picked; delivery waits.
    OptionsChanged,
    SubscriberGone,
    SourceGone,
}

/// One occurrence's line in a subscription's trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationTraceLine {
    pub occurrence_id: String,
    pub at_ms: u64,
    pub outcome: NotificationTraceOutcome,
    /// The first line of the occurrence's message.
    pub title: String,
    /// Why it was filtered out, worded for the picked option: "Not PR number 976".
    pub detail: Option<String>,
    /// The prompt the delivery queued for the agent.
    pub turn_ref: Option<String>,
}
