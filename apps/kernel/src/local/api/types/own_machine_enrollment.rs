//! Enrollment of a person's own machine through a single-use Cloud ticket.
use super::*;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueOwnMachineEnrollmentRequest {}
/// The only place the single-use code appears: a shell command for the new machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnMachineEnrollmentCommand {
    pub command: String,
    pub expires_at: String,
}
