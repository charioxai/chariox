mod agent;
mod capability;
mod system;
mod workflow;

use super::MetaCommandDoc;

pub(crate) use agent::AGENT_SPAWN_USAGE;

const COMMAND_GROUPS: &[&[MetaCommandDoc]] = &[
    agent::COMMANDS,
    workflow::COMMANDS,
    capability::COMMANDS,
    system::COMMANDS,
];

pub(super) fn commands() -> impl Iterator<Item = &'static MetaCommandDoc> {
    COMMAND_GROUPS.iter().flat_map(|commands| commands.iter())
}
