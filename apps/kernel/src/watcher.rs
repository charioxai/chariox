//! Watchers: scripts the kernel supervises as notification sources of their owner. The agent
//! writes the script; the kernel runs it, keeps its memory and gives its events no meaning.
pub(crate) mod contract;
pub(crate) mod run;
#[cfg(test)]
mod tests;
