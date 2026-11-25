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
}

/// The answer to a [`MachineAcquisitionRequest`], and the payload of the `chariox.machines.*` tools.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MachineAcquisitionResult {
    Options {
        options: Vec<MachineOption>,
        default: Option<MachineAcquisitionOffer>,
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
