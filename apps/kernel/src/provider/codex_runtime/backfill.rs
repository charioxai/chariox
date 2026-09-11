//! Authoritative Codex turn reconciliation retry policy.

use std::time::{Duration, Instant};

const CODEX_AUTHORITATIVE_BACKFILL_INTERVAL: Duration = Duration::from_millis(500);
// Fourteen exponentially spaced attempts cover more than an hour of silent
// model work while still giving every unchanged evidence version a hard end.
const CODEX_AUTHORITATIVE_BACKFILL_MAX_RECOVERY_ATTEMPTS: u8 = 14;

#[derive(Debug, Clone, Default)]
pub(super) struct CodexAuthoritativeBackfillGate {
    last_attempt_at: Option<Instant>,
    evidence_version: Option<Instant>,
    recovery_attempts: u8,
}

impl CodexAuthoritativeBackfillGate {
    pub(super) fn is_due(
        &mut self,
        has_active_turn: bool,
        evidence_version: Option<Instant>,
        now: Instant,
    ) -> bool {
        if !has_active_turn {
            return false;
        }

        let Some(last_attempt_at) = self.last_attempt_at else {
            self.record_attempt(evidence_version, now);
            return true;
        };
        let Some(evidence_version) = evidence_version else {
            return false;
        };

        let is_new_evidence = self.evidence_version != Some(evidence_version);
        if !is_new_evidence
            && self.recovery_attempts >= CODEX_AUTHORITATIVE_BACKFILL_MAX_RECOVERY_ATTEMPTS
        {
            return false;
        }

        let retry_interval = if is_new_evidence {
            CODEX_AUTHORITATIVE_BACKFILL_INTERVAL
        } else {
            CODEX_AUTHORITATIVE_BACKFILL_INTERVAL
                .saturating_mul(1_u32 << u32::from(self.recovery_attempts.saturating_sub(1)))
        };
        if now.duration_since(last_attempt_at) < retry_interval {
            return false;
        }

        self.record_attempt(Some(evidence_version), now);
        true
    }

    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    fn record_attempt(&mut self, evidence_version: Option<Instant>, now: Instant) {
        if self.evidence_version == evidence_version && evidence_version.is_some() {
            self.recovery_attempts = self.recovery_attempts.saturating_add(1);
        } else {
            self.evidence_version = evidence_version;
            self.recovery_attempts = u8::from(evidence_version.is_some());
        }
        self.last_attempt_at = Some(now);
    }
}
