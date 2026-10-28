//! MP-08 / MP-10 / MP-11: an owner's refusal, an unanswered expiry and a Room
//! created before enrollment are distinct sudo outcomes.
use crate::error::DaemonError;

#[derive(Clone, Copy)]
pub(crate) enum SudoRequestFailure {
    Refused,
    Expired,
    /// The Room predates enrollment, so its owner is `local`, not the signed-in user.
    RoomOwnerLocal,
}

impl SudoRequestFailure {
    pub(crate) const ALL: [Self; 3] = [Self::Refused, Self::Expired, Self::RoomOwnerLocal];

    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Refused => "kernel_sudo_refused",
            Self::Expired => "kernel_sudo_expired",
            Self::RoomOwnerLocal => "kernel_sudo_room_owner_local",
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
            Self::RoomOwnerLocal => {
                "sudo refused: this Room was created before this kernel was enrolled, so its owner is `local`, not the signed-in user; re-create the Room now that the kernel is enrolled (enroll this kernel first if it is not)"
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
