use crate::session::{
    SavedPermissionDecision, SavedRoomGeneration, SavedRoomGenerationError, SavedRoomGenerationRef,
    SavedRuntimeImageIdentity, SavedSourceIdentity,
};

use super::SessionService;

impl SessionService {
    pub(crate) fn capture_saved_room_generation(
        &self,
        room_id: &str,
        generation_id: impl Into<String>,
        permissions: Vec<SavedPermissionDecision>,
        source: SavedSourceIdentity,
        runtime_image: SavedRuntimeImageIdentity,
        previous_generation: Option<SavedRoomGenerationRef>,
    ) -> Result<SavedRoomGeneration, SavedRoomGenerationError> {
        let snapshot = self.room_environments.snapshot(room_id).map_err(|error| {
            SavedRoomGenerationError::InvalidField {
                field: "room",
                message: format!("{error:?}"),
            }
        })?;
        SavedRoomGeneration::capture(
            generation_id,
            snapshot,
            permissions,
            source,
            runtime_image,
            previous_generation,
        )
    }

    pub(crate) fn saved_room_generations(&self) -> Vec<SavedRoomGeneration> {
        self.saved_room_generations.values().cloned().collect()
    }

    pub(crate) fn saved_room_generation(&self, room_id: &str) -> Option<SavedRoomGeneration> {
        self.saved_room_generations.get(room_id).cloned()
    }

    pub(crate) fn restore_saved_room_generations(
        &mut self,
        generations: Vec<SavedRoomGeneration>,
    ) -> Result<(), SavedRoomGenerationError> {
        let mut validated = std::collections::BTreeMap::new();
        for generation in generations {
            generation.validate()?;
            let room_id = generation.room.room_id.clone();
            if validated.insert(room_id.clone(), generation).is_some() {
                return Err(SavedRoomGenerationError::InvalidField {
                    field: "room",
                    message: format!("duplicate saved generation for Room `{room_id}`"),
                });
            }
        }
        self.saved_room_generations = validated;
        Ok(())
    }

    pub(crate) fn restore_saved_room_generation(
        &mut self,
        generation: SavedRoomGeneration,
    ) -> Result<(), SavedRoomGenerationError> {
        generation.validate()?;
        self.saved_room_generations
            .insert(generation.room.room_id.clone(), generation);
        Ok(())
    }
}
