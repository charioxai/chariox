//! A command whose process is gone while OpenCode still shows it running.
//!
//! OpenCode 1.19 never notices that the process of a bash command ended when
//! that process is its own direct child and ended by a signal (a kill from
//! outside, its own timeout, the kernel stopping a runaway writer): the tool
//! stays `running`, the session stays busy and the turn never ends. It happens
//! with a single simple command, which the shell executes in place; a command
//! with a shell around it reports the kill as exit code 137 and is fine.
//!
//! The kernel pairs each running bash tool with its process, a direct child of
//! the `opencode serve` process, and ends the turn once that process has been
//! gone for a while. Anything it cannot pair with certainty is left alone.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::provider::opencode_client::OpenCodePart;

/// The result a stopped command shows in the transcript.
pub(super) const STOPPED_COMMAND: &str = "The command was stopped.";

/// A tool younger than this is not looked at: most commands end by then.
const FIRST_CHECK: Duration = Duration::from_secs(2);
const CHECK_INTERVAL: Duration = Duration::from_secs(1);
/// How long the process must stay gone while OpenCode still shows the tool
/// running: OpenCode completes a normal exit within milliseconds.
const GONE_FOR: Duration = Duration::from_secs(5);
/// A process may start a moment before OpenCode marks its tool running.
const START_SLACK_MS: u64 = 2_000;

/// A direct child of the OpenCode server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ChildProcess {
    pub pid: u32,
    pub started_ms: u64,
    pub argv: Vec<String>,
}

/// The live direct children of the `opencode serve` process listening on a
/// port, or `None` when that cannot be read (not Linux, server not found).
pub(super) trait ServerChildren: Send {
    fn children(&mut self, port: u16) -> Option<Vec<ChildProcess>>;
}

/// A bash tool OpenCode shows as running.
#[derive(Debug)]
struct RunningTool {
    command: String,
    input: Value,
    started_ms: u64,
    seen: Instant,
    process: Option<(u32, u64)>,
    gone_since: Option<Instant>,
}

/// A tool whose process is gone: its row is replaced by a stopped result.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct StoppedTool {
    pub part_id: String,
    pub input: Value,
}

pub(super) struct RunningTools {
    tools: BTreeMap<String, RunningTool>,
    checked_at: Option<Instant>,
    children: Box<dyn ServerChildren>,
}

impl std::fmt::Debug for RunningTools {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RunningTools")
            .field("tools", &self.tools)
            .finish_non_exhaustive()
    }
}

/// The processes already paired with a tool, by server port, pid and start:
/// a second tool never pairs with the same process, in this session or another.
static PAIRED: Mutex<BTreeMap<(u16, u32, u64), String>> = Mutex::new(BTreeMap::new());

impl RunningTools {
    pub(super) fn new() -> Self {
        Self::with_children(Box::new(ProcFs))
    }

    pub(super) fn with_children(children: Box<dyn ServerChildren>) -> Self {
        Self {
            tools: BTreeMap::new(),
            checked_at: None,
            children,
        }
    }

    /// Follows a tool part: a bash tool is running until its part says otherwise.
    pub(super) fn note(&mut self, part: &OpenCodePart, now: Instant, now_ms: u64) {
        if part.kind != "tool" || part.tool != "bash" {
            return;
        }
        let Some(state) = part.state.as_ref() else {
            return;
        };
        if state.status != "running" {
            self.tools.remove(&part.id);
            release(&part.id);
            return;
        }
        let command = state
            .input
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        self.tools.entry(part.id.clone()).or_insert(RunningTool {
            command,
            input: state.input.clone(),
            started_ms: state
                .time
                .as_ref()
                .and_then(|time| time.start)
                .unwrap_or(now_ms),
            seen: now,
            process: None,
            gone_since: None,
        });
    }

    /// Makes every tool look as if its process had been gone for `by` already.
    #[cfg(test)]
    pub(super) fn backdate_gone(&mut self, by: Duration) {
        for tool in self.tools.values_mut() {
            tool.gone_since = Some(Instant::now() - by);
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Forgets every tool, e.g. when the turn ended.
    pub(super) fn clear(&mut self) {
        for part_id in self.tools.keys() {
            release(part_id);
        }
        self.tools.clear();
        self.checked_at = None;
    }

    /// The tools whose process has been gone long enough, looked at once a second.
    pub(super) fn stopped(&mut self, port: u16, now: Instant) -> Vec<StoppedTool> {
        if self
            .tools
            .values()
            .all(|tool| now.saturating_duration_since(tool.seen) < FIRST_CHECK)
        {
            return Vec::new();
        }
        if self
            .checked_at
            .is_some_and(|at| now.saturating_duration_since(at) < CHECK_INTERVAL)
        {
            return Vec::new();
        }
        self.checked_at = Some(now);
        let Some(children) = self.children.children(port) else {
            return Vec::new();
        };
        let mut paired = PAIRED.lock().unwrap_or_else(|e| e.into_inner());
        // A claim on a process that ended is no claim.
        paired.retain(|(claimed_port, pid, started), _| {
            *claimed_port != port
                || children
                    .iter()
                    .any(|child| child.pid == *pid && child.started_ms == *started)
        });
        let mut stopped = Vec::new();
        let unpaired_tools = self
            .tools
            .values()
            .filter(|tool| tool.process.is_none())
            .count();
        for (part_id, tool) in &mut self.tools {
            if now.saturating_duration_since(tool.seen) < FIRST_CHECK {
                continue;
            }
            let alive = match tool.process {
                Some((pid, started)) => children
                    .iter()
                    .any(|child| child.pid == pid && child.started_ms == started),
                None => {
                    let candidates: Vec<&ChildProcess> = children
                        .iter()
                        .filter(|child| {
                            child.started_ms + START_SLACK_MS >= tool.started_ms
                                && !paired.contains_key(&(port, child.pid, child.started_ms))
                        })
                        .collect();
                    let by_command = candidates
                        .iter()
                        .find(|child| runs_command(&child.argv, &tool.command));
                    let chosen = match (by_command, candidates.as_slice()) {
                        (Some(child), _) => Some(*child),
                        (None, [only]) if unpaired_tools == 1 => Some(*only),
                        _ => None,
                    };
                    match chosen {
                        Some(child) => {
                            tool.process = Some((child.pid, child.started_ms));
                            paired.insert((port, child.pid, child.started_ms), part_id.clone());
                            true
                        }
                        // Several processes and none of them provably this
                        // tool's: leave it to OpenCode.
                        None => !candidates.is_empty(),
                    }
                }
            };
            if alive {
                tool.gone_since = None;
                continue;
            }
            let since = *tool.gone_since.get_or_insert(now);
            if now.saturating_duration_since(since) >= GONE_FOR {
                stopped.push(StoppedTool {
                    part_id: part_id.clone(),
                    input: tool.input.clone(),
                });
            }
        }
        stopped
    }
}

impl Drop for RunningTools {
    fn drop(&mut self) {
        for part_id in self.tools.keys() {
            release(part_id);
        }
    }
}

fn release(part_id: &str) {
    PAIRED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|_, paired_part| paired_part != part_id);
}

/// Whether a process is the one OpenCode started for `command`: the shell with
/// the command as its `-c` argument, or the command's own program once the
/// shell executed it in place.
fn runs_command(argv: &[String], command: &str) -> bool {
    if let [_, flag, script] = argv {
        if flag == "-c" && script == command {
            return true;
        }
    }
    let words = |text: &str| {
        text.split_whitespace()
            .map(|word| {
                word.chars()
                    .filter(|c| !matches!(c, '"' | '\'' | '\\'))
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
    };
    words(&argv.join(" ")) == words(command)
}

/// The kernel's view of /proc.
struct ProcFs;

impl ServerChildren for ProcFs {
    #[cfg(target_os = "linux")]
    fn children(&mut self, port: u16) -> Option<Vec<ChildProcess>> {
        let server = linux::server_pid(port)?;
        linux::children(server)
    }

    #[cfg(not(target_os = "linux"))]
    fn children(&mut self, _port: u16) -> Option<Vec<ChildProcess>> {
        None
    }
}

/// The port of an OpenCode server's base URL.
pub(super) fn server_port(base_url: &str) -> Option<u16> {
    base_url
        .rsplit(':')
        .next()?
        .trim_end_matches('/')
        .parse()
        .ok()
}

#[cfg(target_os = "linux")]
mod linux {
    use super::ChildProcess;

    pub(super) fn server_pid(port: u16) -> Option<u32> {
        let port = port.to_string();
        std::fs::read_dir("/proc")
            .ok()?
            .flatten()
            .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
            .find(|pid| {
                argv(*pid).is_some_and(|argv| {
                    argv.iter().any(|arg| arg == "serve")
                        && argv
                            .windows(2)
                            .any(|pair| pair[0] == "--port" && pair[1] == port)
                        && argv
                            .first()
                            .is_some_and(|program| program.rsplit('/').next() == Some("opencode"))
                })
            })
    }

    /// The live direct children of `pid`, from the kernel's own children lists.
    pub(super) fn children(pid: u32) -> Option<Vec<ChildProcess>> {
        let mut pids = Vec::new();
        for task in std::fs::read_dir(format!("/proc/{pid}/task"))
            .ok()?
            .flatten()
        {
            let list = std::fs::read_to_string(task.path().join("children")).ok()?;
            pids.extend(
                list.split_whitespace()
                    .filter_map(|pid| pid.parse::<u32>().ok()),
            );
        }
        let boot = boot_time_ms()?;
        let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        if ticks <= 0 {
            return None;
        }
        Some(
            pids.into_iter()
                .filter_map(|pid| {
                    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
                    let mut fields = stat[stat.rfind(')')? + 1..].split_whitespace();
                    (!fields.next()?.starts_with('Z')).then_some(())?;
                    let started = fields.nth(18)?.parse::<u64>().ok()?;
                    Some(ChildProcess {
                        pid,
                        started_ms: boot + started * 1000 / ticks as u64,
                        argv: argv(pid)?,
                    })
                })
                .collect(),
        )
    }

    fn argv(pid: u32) -> Option<Vec<String>> {
        let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
        Some(
            raw.split(|byte| *byte == 0)
                .filter(|arg| !arg.is_empty())
                .map(|arg| String::from_utf8_lossy(arg).into_owned())
                .collect(),
        )
    }

    fn boot_time_ms() -> Option<u64> {
        std::fs::read_to_string("/proc/stat")
            .ok()?
            .lines()
            .find_map(|line| line.strip_prefix("btime "))?
            .trim()
            .parse::<u64>()
            .ok()
            .map(|seconds| seconds * 1000)
    }
}

#[cfg(test)]
mod tests;
