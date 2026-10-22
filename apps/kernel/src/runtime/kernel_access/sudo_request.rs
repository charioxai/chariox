//! MP-08 / MP-10 / MP-11: an owner's refusal and an unanswered expiry differ.
use crate::error::DaemonError;

#[derive(Clone, Copy)]
pub(crate) enum SudoRequestFailure {
    Refused,
    Expired,
}

impl SudoRequestFailure {
    pub(crate) const ALL: [Self; 2] = [Self::Refused, Self::Expired];

    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Refused => "kernel_sudo_refused",
            Self::Expired => "kernel_sudo_expired",
        }
    }

    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Refused => {
                "sudo request refused; submit a new request only if you still need elevation"
            }
            Self::Expired => {
                "sudo request expired; submit a new request and approve it in a Chariox client"
            }
        }
    }

    pub(crate) fn error(self) -> DaemonError {
        DaemonError::MachineRequest {
            code: self.code().into(),
            message: self.message().into(),
        }
    }

    pub(crate) fn is_public_code(code: &str) -> bool {
        Self::ALL.into_iter().any(|failure| failure.code() == code)
    }
}
