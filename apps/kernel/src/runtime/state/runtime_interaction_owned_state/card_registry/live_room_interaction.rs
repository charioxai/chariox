use crate::session::RuntimeInteraction;

/// Owns the live publication permit for one Room card.
pub(crate) struct LiveRoomInteraction {
    presentation: RuntimeInteraction,
}

impl LiveRoomInteraction {
    pub(super) fn admitted(presentation: RuntimeInteraction) -> Self {
        Self { presentation }
    }

    pub(super) fn presentation(&self) -> &RuntimeInteraction {
        &self.presentation
    }

    pub(crate) fn into_presentation(self) -> RuntimeInteraction {
        self.presentation
    }
}
