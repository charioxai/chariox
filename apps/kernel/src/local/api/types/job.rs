use super::*;

/// Resource hints, interpreted by the connected machine service.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobNeeds {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_gib: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpus: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JobCommand {
    Argv(Vec<String>),
    Shell(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunJobRequest {
    pub command: JobCommand,
    #[serde(default)]
    pub cwd: Option<String>,
    /// Project environment names. Values never belong in a tool request.
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub needs: JobNeeds,
    #[serde(default = "job_timeout")]
    pub timeout_s: u64,
    #[serde(default)]
    pub wait_s: u64,
    #[serde(default)]
    pub outputs: Vec<String>,
}

fn job_timeout() -> u64 {
    3600
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobPlacement {
    Local,
    Remote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobOutcome {
    Completed,
    TimeLimit,
    MemoryLimit,
    OutsideEnd,
    MachineLost,
    CapacityUnavailable,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRanOn {
    pub machine: String,
    pub ip: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobResult {
    pub exit_code: Option<i32>,
    pub output_tail: String,
    pub log: String,
    pub files_back: Vec<String>,
    pub conflicts: Vec<String>,
    pub ran_on: JobRanOn,
    pub duration_ms: u64,
    pub outcome: JobOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobView {
    pub job_id: String,
    pub agent_id: String,
    pub command: String,
    pub state: String,
    pub placement: JobPlacement,
    pub started_at_ms: u64,
    pub machine_label: Option<String>,
    pub address: Option<String>,
    pub class_id: Option<String>,
    pub rate_per_minute: Option<JobMachineRate>,
    pub estimate_micros: Option<u64>,
    pub deleted: bool,
    pub result: Option<JobResult>,
}
