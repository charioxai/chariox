//! MP-08 / MP-10 / MP-11: home authority and worker materialization for native composer turns.
#[path = "skills_leased_composer/home.rs"]
mod home;
#[path = "skills_leased_composer/worker.rs"]
mod worker;
pub(in crate::runtime::state) use home::prepare_composer_turn;
