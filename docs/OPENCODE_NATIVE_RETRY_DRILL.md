# Native OpenCode retry and cancellation drill

Run `node apps/cli/scripts/live-opencode-retry-drill.mjs` with these absolute paths:

- `CHARIOX_RETRY_KERNEL_BINARY`: compiled kernel under test.
- `CHARIOX_OPENCODE_BIN`: official OpenCode executable.
- `CHARIOX_KERNEL_CLIENT_DIST`: built `packages/kernel-client/dist` directory.

The drill creates an isolated local kernel, Room, attachment and OpenCode agent.
It submits through the public kernel client. Only the model HTTP endpoint is a
fixture, returning HTTP 429 with a known reason and a two-second retry interval.
No paid model credential, real provider profile or repository content is used.
OpenCode may still fetch its own metadata or packages over the network.

Assertions cover two native retry attempts projected through Chariox terminal
events, the actual reason, next-retry timestamp, no invented network failure,
an open prompt turn, kernel cancellation, native idle state and stopped foreground
model requests. OpenCode's separate title-generation request may continue after
native abort; the drill records this rather than treating it as a prompt retry.
Unknown or oversized request bodies count as foreground, so classification fails
closed. Only bounded request classification is retained, never request text.
Transient status text is deliberately read from terminal events, not
the condensed history outline, which excludes these statuses.

Evidence is written outside the repository under
`~/.codex/evidence/browser-computer-use/`. It includes kernel SHA-256, protocol,
provider version, drill source revision/dirty state and cleanup checks for the
kernel, provider children, listening ports and temporary profile. Kernel hashes
are streamed rather than loading the binary into memory. The source revision
identifies the drill, not the compilation provenance of an arbitrary supplied
kernel binary.

## Scope and cold-start regression

This is local kernel/native-provider protocol evidence, not rendered Web/TUI,
relay, Docker, managed deployment or successful model/tool-use acceptance.
Those remain separate requirements in the end-to-end plan.

The default is one Tokio worker, so normal runs also cover cold-prompt startup
without starving the kernel's MCP server. `CHARIOX_RETRY_WORKER_THREADS` allows
one to four workers for comparison. The original one-worker case stalled on kernel
SHA-256
`d1679c80774e094adef66bad9276a644888808dd56c9dc5acf9fa48f80aef0f1`.
The supplied runtime's actual SHA is recorded in each result; do not infer a fix
from a two-worker pass.
Set `CHARIOX_RETRY_PURE=1` to launch the same official CLI with `--pure`. The
one-worker stall occurred both with and without this option.

The startup stall came from synchronous cold-provider initialization in
`ensure_prompt_provider_run_for_agent`, called from `with_app_side_effect` on an
async worker. Initialization waits for MCP requests served by that same runtime.
Shared provider initialization now uses Tokio's blocking region on a multi-thread
runtime, allowing the sole async worker to keep serving the provider's MCP
handshake. Calls outside Tokio or on a current-thread test runtime keep their
existing synchronous behavior. This does not claim that a current-thread runtime
can service asynchronous MCP while a synchronous launch blocks it; the kernel
binary uses a multi-thread runtime even when configured with one worker.

The same live drill fails before this fix at prompt submission with zero model
requests, then passes after it with native retry, cancellation and cleanup checks.
