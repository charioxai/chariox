use serde::{Deserialize, Serialize};

use crate::agent::GitWorktreePlacement;
use crate::execution_lease::{ExecutionLease, LeasedAgent, RemoteWorkflowTurnContext};
use crate::history::{HistoryAttributionConfidence, HistoryEventKind, HistoryEventTurnContext};
use crate::io::WorkspaceIdentity;
use crate::mcp::CharioxMcpServerConfig;
use crate::session::{PromptCancellation, PromptCompletion, PromptOrigin, PromptSubmissionOutcome};
use crate::skill::CharioxSkillPackage;
use crate::terminal::TerminalOutputKind;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RelayManagedContextCapability(String);

impl RelayManagedContextCapability {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl std::fmt::Debug for RelayManagedContextCapability {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[redacted managed-context capability]")
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RelayManagedContextChunk(String);

impl RelayManagedContextChunk {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl std::fmt::Debug for RelayManagedContextChunk {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[redacted managed-context chunk]")
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RelayManagedSliceToken(String);

impl RelayManagedSliceToken {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl std::fmt::Debug for RelayManagedSliceToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[redacted managed-slice relay token]")
    }
}

/// Version 44 requires cancellable uploads and upload execution receipt recovery.
/// Version 45 requires cancellable browser configuration and execution receipt recovery.
/// Version 46 requires cancellable browser lifecycle operations and receipt recovery.
/// Version 47 carries the workspace kind in managed-context import receipts.
/// Version 48 carries private browser-cookie import commands and bounded results.
/// Version 50 carries bounded provider-run termination metadata across leased execution.
/// Version 51 carries kernel-owned project-environment setup dispatch and status.
/// Version 52 carries the source-selected managed-context plan in import arm requests.
/// Version 53 carries the home-authoritative setup attempt on worker redispatch.
/// Version 54 carries recipe and lockfile input attestations for worker setup.
/// Version 55 carries definition-derived executable path entries for worker setup.
/// Version 56 carries the originating home prompt for forwarded worker runtime tools.
/// Version 57 rejects worker kernels that cannot supply the required origin turn
/// for `chariox.send_agent_message`; mixed-version peers fail before dispatch.
/// Version 58 carries the home-owned Room browser capability in extension manifests.
/// Version 59 adds read-only exact-home-prompt receipt queries for leased workers.
/// Version 60 requires exact home-prompt and worker-run identities for leased cancellation
/// and carries durable exact-identity receipts for queued prompt steers.
/// Version 62 also persists worker admission receipts for original leased prompts.
/// Version 63 requires the leased setup worker to receive a home persistence
/// acknowledgment before validating a utility-generated Project definition.
/// Version 64 binds signed daemon admission to the canonical kernel subject.
/// Version 65 installs an authenticated selected Project environment on a leased worker.
/// Version 66 queries and adjusts the leased worker Project through its own authority.
/// Version 67 binds Computer credential input to its approved native display target (MP-08).
/// Version 68 updates/dismisses ephemeral login UI through the existing interaction bridge (MP-08/MP-10/MP-11).
/// Version 69 drops the workflow event capability flags (local protocol 365):
/// `RemoteWorkflowTurnContext` loses `event_context_enabled` /
/// `event_actions_enabled` and leased provider-run projections lose
/// `workflow_event_actions_enabled`, which a peer at v68 or below still requires.
/// Version70 combines Apps Phase1 and autonomous Room observation contracts (MP-08/MP-10).
/// MP-08/MP-10/MP-11: version 86 combines the multidomain and main worker shapes.
/// MP-08/MP-11: reserved peer89 shares the owned-desktop local contract.
/// MP-08/MP-10/MP-11: version 90 negotiates bounded stripes.
/// MP-08/MP-11: version 92 adds Room AT-SPI target actions and owner Computer revoke.
/// Older worker contracts are rejected before dispatch.
/// Version 101 (main) binds the embedded local response union to local protocol 491,
/// restores owner-private workflow notifications to the Browser/Computer peer
/// union and pairs exact home-prompt cancellation admission (MP-08/MP-10/MP-11).
/// Version 102 reconciles the agent-model union and its leased watcher contract (MP-08/MP-09/MP-10/MP-11).
/// Version 114 integrates the 102 agent-model/leased-watcher union, main's
/// identity-preserving renewal and the native display binary-event transport
/// (MP-08/MP-09/MP-10/MP-11), and includes main's 101 kernel ownership,
/// waiting-room and Room workflow contracts (#974). Generated consumers follow
/// this allocated version.
/// MP-08/MP-10/MP-11: allocated 133 carries the local 525 resumption union.
/// MP-08/MP-10/MP-11: allocated 136 carries the local 528 Browser state fields.
/// MP-08/MP-10/MP-11: allocated 137 carries local529 page display placement.
/// Allocated 138 carries the local 530 band gestures and auxiliary popups.
/// MP-08/MP-10/MP-11: allocated 140 negotiates chunks-v2 multiplexed transport.
/// Allocated143 carries535 delivery registration with525 Room resume+attach.
/// MP-08/MP-10/MP-11: allocated144 carries the visible-page default crop contract.
/// MP-08/MP-10/MP-11: allocated 148 carries the local 540 warm omnibox list.
/// MP-08/MP-10/MP-11: allocated 158 carries the local 550 mirror v2 wire contract.
/// Allocated 160 carries the local 552 `parked_input` reply field.
/// MP-08/MP-10/MP-11: allocated 164 carries the local 556 `state_wait`.
/// MP-08/MP-10/MP-11: allocated 175 carries the local 567 `fonts` op `absent`.
/// MP-08/MP-10/MP-11: allocated 202 carries the local 596 state revision order and `latest_prompt_count`.
/// Allocated 209 carries the local 603 dictation service/setup contract.
/// Allocated 216 carries the local 610 union.
/// Agent-model (145) and event-core (169) contracts were integrated in 570/178.
/// The Room terminal service (`RoomTerminal`, formerly provisional 177) and its peer forwards ride this number too.
/// Allocated 181 adds exact-home-check assessment wire identities.
/// Allocated 197 carries the outside `steer` event between peers.
/// Allocated 215 installs independent provider accounts over the encrypted peer channel.
/// Allocated 220 unions the staging 216 line with the external agents 215 line.
/// MP-08/MP-10/MP-11: allocated 232 carries the local 629 browser resize command.
/// Peer 232 also carries protocol 627 attachment byte sizes in shared history.
pub const RELAY_PEER_PROTOCOL_VERSION: u32 = 232;
