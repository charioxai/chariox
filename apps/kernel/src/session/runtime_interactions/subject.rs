//! A projected interaction belongs to a real agent or a kernel operation.
//! This identifies its subject; it does not grant permission to answer it.
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum RuntimeInteractionSubject {
    Agent { agent_id: String },
    KernelOperation { kernel_operation_id: String },
}

impl<'de> Deserialize<'de> for RuntimeInteractionSubject {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Fields {
            #[serde(default)]
            agent_id: Field,
            #[serde(default)]
            kernel_operation_id: Field,
        }
        let fields = Fields::deserialize(deserializer)?;
        let value = match (fields.agent_id, fields.kernel_operation_id) {
            (Field::Present(agent_id), Field::Absent) => Self::Agent { agent_id },
            (Field::Absent, Field::Present(kernel_operation_id)) => Self::KernelOperation {
                kernel_operation_id,
            },
            _ => {
                return Err(serde::de::Error::custom(
                    "interaction requires exactly one subject",
                ))
            }
        };
        if !value.valid() {
            return Err(serde::de::Error::custom("invalid interaction subject"));
        }
        Ok(value)
    }
}

// Explicit null is not an absent subject. This also prevents a second subject
// field from being silently accepted merely because its value is null.
#[derive(Default)]
enum Field {
    #[default]
    Absent,
    Present(String),
}
impl<'de> Deserialize<'de> for Field {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::Present)
    }
}

impl RuntimeInteractionSubject {
    pub(super) fn valid(&self) -> bool {
        let id = match self {
            Self::Agent { agent_id } => agent_id,
            Self::KernelOperation {
                kernel_operation_id,
            } => kernel_operation_id,
        };
        !id.trim().is_empty() && id.len() <= 128 && !id.chars().any(char::is_control)
    }
    pub fn agent_id(&self) -> Option<&str> {
        match self {
            Self::Agent { agent_id } => Some(agent_id),
            Self::KernelOperation { .. } => None,
        }
    }
    pub fn kernel_operation_id(&self) -> Option<&str> {
        match self {
            Self::KernelOperation {
                kernel_operation_id,
            } => Some(kernel_operation_id),
            Self::Agent { .. } => None,
        }
    }
}
