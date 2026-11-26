use super::*;

/// What the kernel owner, or an agent acting for them through an access grant, asks of machine
/// acquisition. An agent in a Room reaches the same operations through `chariox.machines.*`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum MachineAcquisitionRequest {
    /// The machines the connected service offers now, with their stock.
    Options {},
    /// Acquire one: the named class and region, else the first option in stock. A repeated `request_id`
    /// returns the allocation the first try got.
    Acquire {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        class_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        region: Option<String>,
        /// The machine's hard stop in seconds: the service stops the machine then.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_s: Option<u64>,
        /// What the machine is for; the owner's card names it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        purpose: Option<String>,
    },
    Status {
        allocation_ref: String,
    },
    Release {
        allocation_ref: String,
    },
    /// The account's machine limits and the owner's limits for acquiring without asking.
    Limits {},
    /// Change one set of limits. A lower applies at once; a raise needs the person's say-so (see
    /// `machine_acquisition::limits`). A field left out keeps its value.
    SetLimits {
        #[serde(default)]
        scope: LimitScope,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machines_at_once: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        spend_per_day_micros: Option<u64>,
        /// The classes agents may acquire without asking (`scope` no_ask only): the named classes, or an empty list for the smallest only.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        classes: Option<Vec<String>>,
    },
}

/// Which limits a change is about: the account's (the connected service holds them) or the owner's
/// own limits for acquiring without asking (the kernel's `agents.offload_*` settings).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitScope {
    #[default]
    Account,
    NoAsk,
}

/// Both sets of limits as one read shows them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineLimitsReading {
    /// `None` when the connected service has no account limits.
    pub account: Option<AccountLimitsReading>,
    pub no_ask: NoAskLimitsReading,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountLimitsReading {
    pub machines_at_once: u32,
    /// How many machines count against it now, when the service says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machines_in_use: Option<u32>,
    pub spend_per_day_micros: u64,
    pub currency: String,
    pub version: u64,
}

/// The owner's limits for acquiring without asking: what they set, and what applies once the account
/// limits bound it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoAskLimitsReading {
    pub enabled: bool,
    pub machines_at_once: u32,
    /// `None` is no cap of its own.
    pub spend_per_day_micros: Option<u64>,
    pub classes: Option<Vec<String>>,
    pub effective_machines_at_once: u32,
    pub effective_spend_per_day_micros: Option<u64>,
    /// True when an account limit lowers a no-ask value.
    pub clamped: bool,
}

/// The answer to a [`MachineAcquisitionRequest`], and the payload of the `chariox.machines.*` tools.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MachineAcquisitionResult {
    Options {
        options: Vec<MachineOption>,
        default: Option<MachineAcquisitionOffer>,
        /// How many machines count against the account's limit now, before any is asked for.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account_machines: Option<AccountMachines>,
    },
    /// An acquired machine with the option it was acquired under; no duration is known up front.
    Acquired {
        allocation: ControlPlaneResponse,
        #[serde(flatten)]
        chosen: MachineAcquisitionChoice,
        estimate: MachineAcquisitionEstimate,
    },
    /// The allocation's state after a status read or a release.
    Machine(ControlPlaneResponse),
    /// Both sets of limits, after a read or a change.
    Limits { limits: MachineLimitsReading },
}

/// What the acquisition was made under: the named option, or the service's default offer. The answer
/// carries exactly one of `option` and `offer`, either of which may be null.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged, try_from = "MachineAcquisitionChoiceWire")]
pub enum MachineAcquisitionChoice {
    Option {
        option: Option<MachineOption>,
    },
    ServiceDefault {
        offer: Option<MachineAcquisitionOffer>,
    },
}

/// Which key the answer carries, kept apart from a null value: the outer `Option` is "key present".
#[derive(Deserialize)]
struct MachineAcquisitionChoiceWire {
    #[serde(default, deserialize_with = "key_present")]
    option: Option<Option<MachineOption>>,
    #[serde(default, deserialize_with = "key_present")]
    offer: Option<Option<MachineAcquisitionOffer>>,
}

fn key_present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

impl TryFrom<MachineAcquisitionChoiceWire> for MachineAcquisitionChoice {
    type Error = &'static str;

    fn try_from(wire: MachineAcquisitionChoiceWire) -> Result<Self, Self::Error> {
        match (wire.option, wire.offer) {
            (Some(option), None) => Ok(Self::Option { option }),
            (None, Some(offer)) => Ok(Self::ServiceDefault { offer }),
            _ => Err("an acquired machine names exactly one of option and offer"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineAcquisitionEstimate {
    pub kind: String,
    pub reason: String,
}

impl MachineAcquisitionEstimate {
    pub fn unknown() -> Self {
        Self {
            kind: "unknown".into(),
            reason: "No duration was specified; the total depends on when the machine is deleted"
                .into(),
        }
    }
}
