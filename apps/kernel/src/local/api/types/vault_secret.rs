//! MP-08 / MP-09 / MP-10 / MP-11: an outside agent asks the owner for a Vault value.
//! It learns the link and the outcome, never the value.
use super::*;

/// Protocol 929: the owner types the value in a connected client or on the link's page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestVaultSecretRequest {
    pub credential_id: String,
    /// What the owner is asked for, in the agent's words (shown after the kernel's own sentence).
    pub label: String,
    /// The API hosts that may receive the key (`host` or `host:port`, lowercase): the handle
    /// is bound to them. Required; an empty list is refused.
    #[serde(default)]
    pub hosts: Vec<String>,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetVaultSecretStatusRequest {
    pub interaction_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultSecretStatus {
    Pending,
    Stored,
    Cancelled,
    TimedOut,
}

/// MP-08 / MP-09 / MP-10 / MP-11: what the owner approves for one payment-card fill.
/// `origin`, the entry and the card come from the kernel; `claim` is the asking agent's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultPaymentApproval {
    pub origin: String,
    pub entry: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_last4: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_brand: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim: Option<VaultPaymentClaim>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_by_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// The merchant and amount as the asking agent states them; the kernel checks only their form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultPaymentClaim {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merchant: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
}

impl VaultPaymentClaim {
    pub(crate) fn validate(&self) -> Result<(), crate::error::DaemonError> {
        let merchant = self.merchant.as_deref().is_none_or(|merchant| {
            !merchant.trim().is_empty()
                && merchant.chars().count() <= 100
                && !merchant.chars().any(char::is_control)
        });
        let amount = self.amount.as_deref().is_none_or(|amount| {
            let (whole, cents) = amount.split_once('.').unwrap_or((amount, "0"));
            (1..=12).contains(&whole.len())
                && (1..=4).contains(&cents.len())
                && whole
                    .bytes()
                    .chain(cents.bytes())
                    .all(|b| b.is_ascii_digit())
        });
        let currency = self.currency.as_deref().is_none_or(|currency| {
            currency.len() == 3 && currency.bytes().all(|b| b.is_ascii_uppercase())
        });
        if merchant && amount && currency {
            Ok(())
        } else {
            Err(crate::error::DaemonError::LocalTransport {
                operation: "vault_payment_claim",
                message: "payment takes a merchant name up to 100 characters, an amount like 12.50 and a 3-letter currency code".into(),
            })
        }
    }
}
