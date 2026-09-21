# Browser and computer use end-to-end productization plan

## Status and authority

This is the canonical execution and validation plan for Linear map
[CHA-16](https://linear.app/chariox/issue/CHA-16/productize-shared-browser-and-computer-use)
and sub-issues CHA-17 through CHA-31.

It consolidates and extends:

- `docs/M18_SLICE_SAVED_STATE_PLAN.md`
- `docs/M19_USER_OWNED_SLICE_SAVE_RESTART_PLAN.md`
- `docs/M20_DOCKER_SLICE_BROWSER_STATE_VALIDATION_PLAN.md`
- `docs/M21_VIEW_TAB_RUNTIME_MCP_DRILLS_PLAN.md`
- `docs/M22_VIEW_FEATURE_A_PLUS_PLAN.md`
- `docs/AGENT_SLICE_OFFICE_WORK_DRILLS.md`
- `docs/M28_ENVIRONMENT_CONTEXT_MATERIALIZATION_PLAN.md`
- the current architecture and protocol contracts in `docs/ARCHITECTURE.md`
  and `docs/PROTOCOL.md`

The plan covers product UI required to use Browser and Computer capabilities,
including the Cloud View page and TUI projections. It does not cover the
separate OS-shell redesign prototype except where that prototype reveals a
product requirement that must exist in the production Web terminal.

The two main repositories are:

- `chariox`: kernel, relay, slice runtime, browser controller, computer tools,
  local and remote TUI clients, provider adapters, protocol, CLI drills, and
  self-hosted behavior.
- `chariox-cloud`: hosted control plane, waiting room, managed-machine
  provisioning, web terminal, View UI, hosted relay bootstrap, OpenShip-based
  staging and deployment, and browser acceptance drills.

If implementation introduces another maintained repository, such as a Chariox
fork or packaging repository for Selkies, that repository must follow the same
isolated-PR, review, evidence, resource, and cleanup rules in this plan.

## Goal

Ship one Chariox-owned shared browser and computer environment per Room that is
usable by humans and every agent in that Room.

Work proceeds in three ordered phases. A later phase cannot compensate for a
failure in an earlier one.

1. Implement and prove the complete functional contract locally. Run the
   kernel, relay, headed slices, browser controller, viewer, Web terminal,
   local TUI, remote TUI, providers, persistence, vault, concurrency,
   resiliency, and cleanup paths on controlled local infrastructure first.
   Local proof includes every functional matrix row that does not inherently
   require a remote machine. Do not use a managed-machine run to discover basic
   product defects that a local test should have caught.
2. Deploy the locally proven implementation to a fresh Chariox-managed machine
   through the in-house OpenShip stack. Repeat the applicable functional,
   security, reconnect, persistence, resiliency, Web, and TUI drills through
   the hosted Caddy-fronted `wss://` relay. This phase proves portability,
   deployment, hosted transport, machine lifecycle, and production topology.
3. Start benchmarking only after the complete local and managed-machine
   functional gates pass. Build reproducible submissions for every relevant
   public browser-use and computer-use benchmark, compare Chariox with the
   published leaders under equivalent conditions, profile failures, and
   optimize without weakening correctness. The product goal is unambiguous:
   Chariox must rank first on every relevant public benchmark, not merely beat
   its own baseline. When a benchmark tests a capability Chariox claims to
   support, it belongs in the campaign unless its rules make a fair Chariox
   submission impossible. Record and justify any exclusion.

The complete local flow must prove that one Room agent launches through the
normal kernel and provider adapter path, drives the exact browser shown in the
Web View, and remains observable and controllable from local and remote TUI
clients. Human takeover must not create a second browser session. Save and
restart must preserve the browser profile, installed programs, desktop state,
provider thread, and idle agents through a full container destroy and recreate.
Vault-backed credentials must remain absent from model context, terminal
transcripts, relay and Cloud data, logs, traces, screenshots, and helper output.
Client disconnect and reconnect must not duplicate provider runs or split Room
state. Every drill must remove everything it created.

## Product and architecture decisions

### One Room environment

A Room owns one shared browser and computer environment unless the user
explicitly creates another environment. All Room users and agents may inspect
and act in it according to kernel-owned permissions and interaction policy.

The environment contains:

- a shared browser tab registry with stable tab IDs
- a canonical browser viewport
- the complete graphical desktop
- installed programs and files
- clipboard and input state
- browser profile and authenticated sessions
- actor presence, pointers, action attribution, and history

### Browser and Computer are two modes, not two implementations

Browser mode provides structured tabs, address bar, history, accessibility and
DOM observations, lifecycle events, and first-party browser actions.

Computer mode exposes the full graphical environment and provides screenshot,
OCR, coordinate, mouse, keyboard, clipboard, and desktop-application control.

Both modes must use the same kernel authority, display stream, input path,
presence model, permissions, persisted state, and relay path. Structured
browser control is preferred. Computer control is the fallback for browser
chrome, graphical applications, unsupported page structures, and recovery.

### Kernel authority

The kernel owns Rooms, sessions, agents, provider runs, tab identity,
interaction serialization, permissions, actor attribution, saved state, and
history. The relay transports encrypted packets and display data but never
becomes browser, session, prompt, or history authority. Cloud authenticates,
provisions, issues scoped relay tokens, and renders control-plane state, but it
does not proxy or inspect runtime terminal traffic.

### Browser controller

Replace the one-process-per-action CDP helper with one long-running,
kernel-managed Chariox Browser Controller. It may use Playwright Core or an
equivalently complete CDP implementation internally. Agents receive stable
first-party Chariox tools, not raw provider-native or third-party browser tools.

The controller must own:

- stable tab and target IDs
- accessibility and DOM observations
- stable, invalidatable element references
- auto-waiting and navigation lifecycle
- nested frames and shadow DOM
- dialogs, popups, permissions, downloads, and uploads
- console and network events
- browser health, crash recovery, and graceful shutdown
- bounded connection, command, and operation timeouts
- one canonical viewport shared by all attached terminals

### Display technology

Selkies is the leading OSS replacement for noVNC. The first production path
should use Selkies WebSocket transport through the existing Chariox relay. It
avoids making STUN, TURN, UDP, and WebRTC prerequisites for the initial
cutover. WebRTC remains an optional later optimization only if benchmarks show
that its latency benefit justifies its operational complexity.

noVNC remains behind an explicit development or rollback flag until Selkies
passes the complete validation matrix. It must not remain the default after the
cutover gate passes.

### Actor and input model

Every input action must identify:

- human or agent actor ID
- Room and environment ID
- tab ID when applicable
- action type and target
- start, completion, cancellation, and outcome
- pointer coordinates when applicable

Reads may occur concurrently. Mutating actions serialize per tab or desktop
input target. Mutations on different tabs may run concurrently. Human takeover
must pause or cancel the active agent interaction on that target immediately.
Pointers and presence render in a Chariox-owned overlay above the stream, not
inside webpage DOM.

## Mandatory development discipline

### Isolated PRs and worktrees

Every independently reviewable subtask must use an isolated branch and
worktree. Do not implement this plan directly in a dirty main checkout.

Each impacted repository gets its own PR. A cross-repository feature therefore
normally produces at least:

- one OSS PR in `chariox`
- one Cloud PR in `chariox-cloud`

Do not combine unrelated milestones in one PR. Do not hide an OSS protocol
change inside a Cloud UI PR. Do not leave a cross-repository PR dependent on an
unpublished local commit. Record the exact paired PR and commit SHA in both PR
descriptions.

Work from `main` unless the user explicitly changes the branch policy. Rebase
or merge the latest `main` before final validation, then rerun the relevant
matrix at the exact reviewed head.

Keep implementation worktrees source-only. Git worktrees must share the main
repository object database and must not each acquire a Cargo `target`,
`node_modules`, package cache, browser profile, container image, or slice state.
Put the smallest necessary test binary or generated file in a task-specific
disposable directory outside every repository, then remove it immediately
after validation. Use final-head remote CI for full builds while the laptop is
under disk pressure. Remove a worktree as soon as its PR merges, and keep only
the worktrees needed for active or externally gated PRs.

### Commit, push, and review after every completed subtask

After every subtask that leaves the tree in a coherent state:

1. Run the narrow unit and integration tests for that subtask.
2. Inspect the diff for unrelated changes, secrets, generated artifacts, and
   accidental protocol changes.
3. Commit the completed subtask with a message that states the behavior and the
   reason for the change.
4. Push the commit immediately.
5. Update the PR body and evidence index with the exact commit SHA and tests.
6. Request or trigger the independent reviewer on that exact head.
7. Read every reviewer comment. Fix valid findings in a new commit, push it,
   and request review again.
8. Continue until the reviewer has no remaining comments on the current exact
   head.

Do not claim a commit is reviewed when comments apply to an older SHA. Do not
force-push away reviewed evidence unless recovery requires it. If the head
changes, repeat review. A subtask is not complete while the reviewer has an
unaddressed comment, a question without evidence, or a stale-head review.

### Final-head CI only

Do not run the repository-wide GitHub CI suite for intermediate implementation
or reviewer-fix commits. Use the narrow local unit and integration checks above
while the PR is still evolving. Keep the PR out of the ready-to-merge CI path
until its exact head has passed focused validation and the independent reviewer
has no remaining comments.

Run GitHub CI once for that final, reviewer-clean head immediately before
merge. If final CI finds a defect, fix it in a new coherent commit, repeat the
exact-head reviewer loop, and then run the final CI gate again. Do not treat a
successful run for an older SHA as evidence for a changed head.

Repository workflow triggers must enforce this policy: ordinary PR
`synchronize` events must not start the expensive CI jobs, while the explicit
ready-for-review/final-gate transition may start them. Reviewer automation is
independent of this CI gate and must continue to receive the commits it needs
to review.

### Reviewer independence and outages

The implementation agent must not communicate or coordinate with the other
agent performing review. Do not send it messages, ask it for status, direct its
work, interrupt it, or use it as an implementation sub-agent. Review must remain
independent and arrive through the established reviewer and PR-comment path.

The independent reviewer services and state under `~/.chariox-reviewer` are
shared infrastructure owned outside this implementation program. The
implementation agent is not responsible for operating, diagnosing, repairing,
restarting, stopping, pruning, or modifying that infrastructure. Do not touch it
unless the user explicitly changes this instruction and assigns reviewer repair
as a separate task.

If the reviewer is delayed, unavailable, or temporarily stops posting comments,
record the exact commit SHA awaiting review and continue useful independent work
on another in-scope subtask or PR. Do not idle the whole program, do not weaken
the review gate, and do not mark the waiting commit reviewed. When comments
appear again, return to that exact-head review, address every finding in a new
commit, push it, and repeat the normal review loop until the reviewer has no
remaining comments on the current head.

### Protocol change rule

Any change to `LocalDaemonRequest`, `LocalDaemonResponse`, display events,
relay terminal events, browser/kernel transport, or another serialized client
contract must:

1. increment the shared OSS protocol version
2. update protocol snapshot or hash tests
3. update minimum supported client versions only when the client depends on the
   new contract
4. add a focused drill that crosses the changed boundary
5. prove backward rejection or compatibility behavior explicitly

No protocol-shape PR may merge without those items.

### Evidence

Store screenshots, traces, benchmark output, logs, manifests, and comparison
reports outside repositories under:

```text
/Users/miguel/.codex/evidence/browser-computer-use/<milestone>/<run-id>/
```

Store persistent local development state under:

```text
~/.chariox/dev/browser-computer-use/<task-or-kernel>/
```

Use `mktemp -d` for disposable state. Never point `CHARIOX_HOME`, kernel state,
logs, drill scratch, browser profiles, or slice homes inside a repository.

Each evidence manifest must include:

- exact OSS and Cloud commit SHAs
- exact provider, browser, Selkies, Docker, kernel, relay, and OS versions
- topology and machine identity without secrets
- test or drill command
- start and finish timestamps
- result and failed assertion, if any
- resource samples before, during, and after
- cleanup result
- links to screenshots, traces, logs, and benchmark data

## PR and milestone sequence

### Milestone 0: local functional harness and red-capable drills

Linear: CHA-18, with inputs from CHA-23, CHA-25, CHA-26, and CHA-31.

Create the local functional and failure-reproduction harness before replacing
the current implementation. It must run against the current noVNC and one-shot
CDP stack, fail for the missing product behavior, and run against each later
implementation. This milestone establishes correctness tests, not benchmark
rankings or optimization targets.

Deliverables:

- reproducible local Mac and local Linux or Docker test profiles
- current noVNC display behavior capture
- current structured-browser, screenshot, OCR, mouse, and keyboard behavior
  capture
- per-slice memory, CPU, disk, and process measurements used only to keep the
  development host safe
- deterministic browser-use functional task subset
- deterministic computer-use functional task subset
- Chariox-specific shared-browser and takeover tasks
- fault-injection controls for relay loss, controller crash, streamer crash,
  browser crash, stale endpoint, queue saturation, and memory pressure
- explicit pass/fail assertions for state, authority, user-visible behavior,
  recovery, and cleanup

This milestone must not change production behavior. It must not begin the
public benchmark campaign or tune implementation choices for scores. Its only
performance checks are safety timeouts and resource ceilings needed to prevent
a hung test or exhausted development machine.

### Milestone 1: shared Room browser contract

Linear: CHA-20 and CHA-21.

Define the kernel-owned environment, tab, actor, action, viewport, interaction,
and recovery contracts. Update architecture, protocol, and context documents
before clients implement divergent assumptions.

Deliverables:

- environment identity and lifecycle
- shared tab registry
- canonical viewport and resize ownership
- read and mutation concurrency policy
- human takeover and cancellation semantics
- Browser and Computer mode projections
- actor and action event model
- history and reconnect semantics
- backward-compatibility policy for old clients
- protocol version and snapshot update if serialization changes

Acceptance requires focused kernel tests and one same-host home/kernel/worker
drill with two agents and two clients observing one environment.

### Milestone 2: long-running Browser Controller

Linear: CHA-19 and CHA-27.

Implement the controller below client layers. Keep provider adapters unaware of
browser implementation details.

Subtasks:

1. Controller process ownership, health, startup, shutdown, and restart.
2. Stable tab registry and canonical viewport.
3. Accessibility and DOM snapshots with invalidatable element references.
4. Locator actions with auto-waiting and bounded timeouts.
5. Frames, shadow DOM, dialogs, popups, downloads, uploads, and permissions.
6. Console, network, page, and browser lifecycle events.
7. Runtime MCP adapters preserving current public tool names where possible.
8. Compatibility adapter for old one-shot calls during migration.
9. Actor attribution and per-tab mutation serialization.
10. Crash recovery without tab or authority duplication.

Every subtask gets its own coherent commit and exact-head reviewer loop.

### Milestone 3: Selkies slice runtime and viewer-agnostic transport

Linear: CHA-17 and CHA-23.

Implement Selkies in the Linux slice runtime with software encoding as the
portable baseline. Hardware encoding is an optional optimization and must have
an explicit capability probe and fallback.

Subtasks:

1. Pin and verify Selkies source or package version and licenses.
2. Add container dependencies without unnecessary image growth.
3. Launch, health-check, stop, and clean up Selkies with the slice lifecycle.
4. Expose a viewer-agnostic display endpoint kind and capability set.
5. Route WebSocket display transport through the existing relay boundary.
6. Preserve strict hosted HTTPS and trusted-origin endpoint validation.
7. Implement reconnect, expiry, retry, stale-response rejection, and bounded
   backpressure.
8. Keep noVNC behind an explicit fallback flag during rollout.
9. Add streamer and browser graceful shutdown before state capture.
10. Validate resize, clipboard, input, full desktop, and browser-content modes.

The relay must not decode display content or become session authority.

### Milestone 4: actor overlay and shared input

Linear: CHA-22.

Move pointers and action attribution out of page DOM. Implement a kernel-owned
input arbitration path used by agents and humans.

Acceptance:

- pointer remains visible over browser chrome, iframes, navigation, and desktop
  programs
- stable color and actor name per human or agent
- human takeover pauses or cancels an active agent interaction
- same-tab mutations serialize without deadlock
- different-tab actions can proceed concurrently
- TUI shows actor, tab, action, and outcome events even when it cannot render
  the graphical pointer
- replay and traces identify the actor and target without recording secrets

### Milestone 5: Web terminal View productization

Linear: CHA-24.

Implement the Cloud UI against the released OSS protocol. Cloud must not add a
parallel browser authority.

The View page must support:

- selected managed machine, slice, Room, and agent identity
- stable display while endpoint metadata refreshes
- Browser and Computer mode switching
- tab list, address, activity, actor presence, and focus
- selected-agent prompt and shared transcript behavior
- user pointer and input takeover
- Save and restart agents
- Save and shut down slice
- Start slice
- Open display and Retry
- explicit loading, reconnecting, degraded, unavailable, local-only,
  unsupported-client, unsafe-endpoint, and permission states
- keyboard access and accessible labels
- no layout jump during refresh or retry

Fixture-driven tests remain useful but cannot close this milestone. A real
headed slice must pass through the hosted relay.

### Milestone 6: TUI parity

Linear: CHA-21, CHA-25, CHA-27, and CHA-28.

Both local and remote Chariox TUIs must use the same kernel contracts as the
Web terminal.

The TUI does not need to reproduce a graphical desktop inside a text terminal.
It must expose the same product state and actions in a form appropriate to a
TUI:

- environment and viewer status
- current browser tab, URL, title, and activity
- actor and input-lock status
- browser and computer tool traces
- screenshot capture and artifact path
- open-viewer URL or OS-open action when graphical observation is required
- start, stop, save/restart, save/shutdown, retry, and reconnect actions
- permission and takeover interactions
- actionable errors and protocol-version diagnostics

Required terminal scenarios:

- local TUI attached to a local headed slice
- local TUI attached to a managed remote-machine slice through relay
- remote or orphaned TUI attached through relay
- Web and TUI attached simultaneously to the same Room
- prompt from TUI drives the browser visible in Web
- Web human takeover is visible in TUI activity and traces
- TUI disconnect and reconnect preserves browser and provider state

Provider-native TUIs remain clients of the normal kernel path. They must not
gain a separate browser, prompt, permission, history, or relay authority path.

### Milestone 7: persistence and user-owned lifecycle

Linear: CHA-26 and CHA-28.

Complete deterministic state capture and agent relaunch.

Persist and validate:

- Chromium profile
- cookies, localStorage, IndexedDB, service-worker cache, and Cache Storage
- downloads and application configuration
- installed graphical programs
- machine ID, hostname, user, UID/GID, home, and keyring behavior
- Browser Controller and Selkies graceful shutdown
- slice home archive and committed image
- provider session IDs and relaunch manifests
- idle agent identity and Room attachment
- saved display mode and canonical viewport where appropriate

Reject save deterministically when attached agents are busy. Reconcile stale
attachments. Relaunch only through the kernel-managed provider path. Never
silently create a blank provider thread when resume fails.

### Milestone 8: vault and secret safety

Linear: CHA-29.

Validate secret insertion through both Browser and Computer paths without
making secrets visible to agents or infrastructure.

Required cases:

- selector-based password field
- opaque field ID returned by browser discovery
- expected-host success
- expected-host mismatch
- navigation between discovery and paste
- iframe and popup target
- masked and unmasked field behavior
- user-created and agent-generated credentials
- locked-vault unlock interaction
- wrong user, Room, slice, or managed machine
- log, history, trace, screenshot, clipboard, and helper-output leak scan
- relay and Cloud proof that secret payloads are not inspected or stored

### Milestone 9: managed-machine deployment

Linear: CHA-31, with CHA-17 through CHA-30 as prerequisites.

Use the in-house OpenShip-based Chariox managed-machine stack. Scalingo is not
part of this plan.

Do not start this milestone until the implementation has passed the complete
local functional, security, persistence, concurrency, Web, TUI, resiliency,
regression, and cleanup gates. Managed infrastructure validates the same
reviewed build in the production topology. It is not the primary development
loop.

Provision a fresh managed Linux machine through Chariox Cloud. Do not reuse a
manually prepared snowflake host for acceptance.

The managed-machine drill must prove:

1. Cloud provisions the machine and installs the current reviewed kernel,
   Docker slice runtime, Browser Controller, and Selkies dependencies.
2. The machine registers through the hosted Caddy-fronted `wss://` relay.
3. Heartbeat freshness controls online selection and stale targets are
   rejected.
4. A headed slice starts with explicit CPU, memory, process, and disk limits.
5. All three providers can launch through official provider harnesses when
   credentials are configured.
6. Web View displays the environment through the hosted relay.
7. Local and remote TUI clients attach to the same Room.
8. A browser task and a non-browser desktop task complete.
9. Vault-backed login and save/restart work.
10. The machine survives client disconnect and reconnect.
11. Multiple viewers and slices respect admission and resource budgets.
12. The entire drill can tear down the slice and managed machine without
    residue in Cloud, OpenShip, the relay registry, or the host.

### Milestone 10: public benchmarks and score-driven optimization

This milestone starts only after Milestone 9 and every local and remote
functional gate pass. Freeze a functionally accepted reference build before
collecting benchmark baselines. Performance work must preserve that build's
correctness, security, recovery, compatibility, and cleanup behavior.

Deliverables:

1. Inventory every maintained and relevant public benchmark for browser use,
   computer use, web agents, office work, and multimodal desktop control.
   BrowserGym, WebArena, WorkArena, and OSWorld are initial candidates, not a
   closed list. Refresh the inventory against the benchmark maintainers before
   the campaign starts.
2. Record each benchmark's version, task set, rules, environment, model policy,
   allowed tools, time and token limits, scoring method, public leaderboard,
   current leading score, and reproducibility requirements.
3. Publish a reason for excluding any benchmark that tests a claimed Chariox
   capability. "The score is difficult to improve" is not a valid reason.
4. Build reproducible Chariox runners that use the production kernel,
   Browser Controller, Computer path, and official provider harnesses. Do not
   add benchmark-only authority or tool behavior.
5. Run the frozen functionally accepted build first. Retain raw task results,
   traces, failures, costs, latency, and machine resource data.
6. Classify every failure as perception, element grounding, planning, action,
   navigation, browser state, desktop input, concurrency, recovery, provider,
   environment, or benchmark infrastructure.
7. Optimize the shared product implementation. After every optimization,
   rerun the affected functional and regression gates before accepting its
   score.
8. Submit under the public benchmark rules and verify the published result.
9. Repeat until Chariox ranks first on every relevant public benchmark. Track
   leaderboard changes and reopen optimization work when another system takes
   the lead before the release cutoff.

Benchmark evidence must distinguish official public scores from local
reproductions. Never claim first place from an incomparable model, environment,
task subset, private fork, or locally modified scoring rule.

### Milestone 11: rollout and noVNC removal

After all functional, benchmark, and resource gates pass:

1. Enable Selkies for internal and staging users behind a server-controlled
   capability flag.
2. Run a soak period with noVNC fallback available.
3. Compare error rate, reconnect rate, task success, latency, CPU, memory,
   bandwidth, and support incidents against the baseline.
4. Make Selkies the default only after explicit sign-off.
5. Retain noVNC fallback for one rollback window.
6. Remove noVNC packages, code, fixtures, capability names, and documentation
   in a dedicated cleanup PR after the rollback window.

## Resource safety gates during functional implementation

Do not benchmark or optimize before functional acceptance. Resource checks run
from the first local drill for a different reason: they stop slices, browsers,
encoders, and providers from exhausting a development or managed machine.
These checks enforce safety and detect leaks. They do not rank implementations.

### Resource behavior

Measure idle, browser-active, video-active, OCR-active, save, restore, and
multi-viewer states separately.

Record:

- resident memory per kernel, worker, browser, streamer, and slice
- peak and sustained CPU
- GPU encoder use when available
- disk image and home-volume growth
- network bandwidth by frame rate and resolution
- file descriptors, processes, threads, and WebSocket connections
- swap use and major page faults
- time and temporary disk required for save, backup, restore, and image commit

Before concurrent functional drills, set conservative admission limits for the
local Mac and each managed-machine size. Chariox must reject or queue a new
headed slice before the host becomes unsafe. It must not rely on the operating
system OOM killer as normal admission control. Ratify final production limits
during the post-functional benchmark and soak campaign.

## Resource monitoring during every live drill

Slices, Chromium, video encoders, and provider processes use substantial
memory. Resource monitoring is part of correctness, not optional diagnostics.

Before a drill:

- record total and available memory, swap, load, CPU count, and disk space
- list Chariox kernels, relays, workers, provider processes, containers,
  volumes, and listeners
- record existing managed-machine and slice inventory
- calculate the maximum safe concurrent slices for that host
- estimate the peak memory and temporary disk required by the next operation
  from prior measurements or a conservative first-run estimate, and confirm
  that the host has that capacity plus a recovery reserve
- reduce concurrency if another Chariox validation or reviewer workload is
  active
- do not impose a fixed free-space percentage as a global implementation or
  test gate; keep source work and tests moving when their measured resource
  cost fits safely

During a drill:

- sample host and per-container CPU and memory at a fixed interval
- sample disk, swap, network, open files, and process count
- watch kernel, relay, Browser Controller, streamer, Chromium, and provider
  liveness
- stop adding concurrent slices when the forecast peak would consume the
  measured recovery reserve; continue the drill serially when possible
- abort gracefully if sustained swap, OOM risk, disk pressure, or runaway
  encoding threatens the machine
- never hide a resource failure by retrying until the host recovers

After a drill:

- prove all drill-owned processes, containers, networks, volumes, ports,
  tunnels, and machines are gone
- record recovered memory and disk
- investigate unexplained residual growth before running another drill

On a developer laptop, use one headed slice at a time unless the explicit test
requires concurrency. Run provider matrices sequentially when parallelism does
not test product behavior. A low free-space percentage alone must not pause
implementation. Prefer source-only worktrees, shared dependency caches,
targeted builds, serial drills, and immediate cleanup of drill-owned artifacts.
Pause only the specific operation whose forecast peak presents a credible
crash, OOM, or disk-exhaustion risk, and resume it after making room or moving
it to a suitably sized managed machine. On managed machines, use cgroup limits
and the planned admission controller even when the host appears to have spare
memory.

## Validation matrix

Every cell must have an automated test, executable drill, or documented reason
that it cannot apply. A single happy-path screenshot does not close a row.

Execute the matrix in tiers. Tier 1 uses a local kernel and local Docker slices.
Tier 2 adds a same-host relay so Web, local TUI, and remote TUI paths can be
tested without a remote machine. Tier 3 starts only after Tiers 1 and 2 pass and
repeats the applicable rows on OpenShip-provisioned managed infrastructure.
Tier 4 covers self-hosted remote compatibility. Benchmark work is outside these
functional tiers and starts only after all four pass.

### Providers

| Provider | Structured browser | Computer fallback | Save/restart thread | Permissions | Attachments | TUI + Web |
| --- | --- | --- | --- | --- | --- | --- |
| Codex official harness | required | required | required | required | required | required |
| OpenCode official harness | required | required | required | required | required | required |
| Claude Code official harness | required | required | required | required | required | required |

Provider runs must use official provider CLIs, local servers, and documented
hooks or protocols. Do not substitute provider SDKs, hosted agent services, or
third-party agent runtimes.

### Topologies and terminals

| Topology | Web terminal | Local TUI | Remote TUI | Web + TUI together | Client reconnect |
| --- | --- | --- | --- | --- | --- |
| Local kernel + local Docker slice | required | required | n/a | required | required |
| Same-host home/worker through relay | required | required | required | required | required |
| Chariox-managed remote machine | required | required | required | required | required |
| Self-hosted remote worker | required | required | required | required | required |
| Headless slice | explicit unavailable state | status/control | status/control | required | required |
| Stale or offline target | explicit error | explicit error | explicit error | required | required |

### Browser behavior

| Case | Required proof |
| --- | --- |
| Fresh profile | tab registry, navigation, storage, download, and history initialize correctly |
| Multiple tabs | stable IDs and correct targeting through focus changes |
| Popup and new window | registered, attributable, controllable, and closable |
| Iframe and nested iframe | structured discovery and action work or deterministic fallback occurs |
| Shadow DOM | discovery, action, and stale-reference behavior |
| Dialog | accept, dismiss, prompt value, timeout, and trace |
| Download | progress, path, cancellation, persistence, and safe filename |
| Upload | local, transferred, remote, missing, and denied file |
| Permissions | allow, deny, timeout, multi-client projection, and provider-native reply |
| Browser crash | controller notices, restarts, and avoids duplicate tabs or actions |
| Page navigation during action | cancellation or retargeting is deterministic |
| Stale reference | actionable error followed by rediscovery |
| Console and network | bounded capture, redaction, and no secret leakage |
| Service worker | registration, cache, offline state, and save/restore |
| OAuth or external login | redirects, popups, callback, and service-side reauth distinction |

### Computer behavior

| Case | Required proof |
| --- | --- |
| Screenshot | exact active display and bounded result size |
| OCR and find text | success, no-match, multiple match, scaling, and non-English sample |
| Mouse | move, click, double-click, right-click, drag, and scroll |
| Keyboard | text, shortcut, key chord, repeat, cancel, and non-US character sample |
| Text selection | drag selects text without moving the containing pane or window |
| Browser chrome | tabs, address bar, download shelf, and permission bubble |
| Non-browser app | application remains focused and fully controllable |
| Clipboard | human-to-slice, agent-to-slice, slice-to-human, and secret restrictions |
| Resize | canonical viewport, desktop resolution, aspect ratio, and all viewers |
| Stream loss during input | action cancels or completes exactly once |
| Controller/streamer restart | desktop survives and input authority is not duplicated |

### Persistence and lifecycle

| Case | Required proof |
| --- | --- |
| Save with no agents | image and home state restore |
| Save with idle agents, restart mode | same agent and provider thread relaunch |
| Save with idle agents, shutdown mode | agents remain stopped until explicit start |
| Save with busy agent | deterministic rejection and no partial state |
| Stale attachment | reconciliation before save |
| Browser graceful close | no profile corruption or stale lock |
| Full container removal | restored state uses committed image and home archive |
| Home volume removal | recreated volume has expected persisted content |
| Installed application | binary, config, launcher, and user data survive |
| Cookie and web storage | cookie, localStorage, IndexedDB, Cache Storage, service worker |
| External service reauth | recorded as service invalidation, not silent state loss |
| Repeated save cycles | no unbounded image, archive, or profile growth |
| Reset | returns to clean base without deleting unrelated backups |
| Backup and restore | named backup, integrity check, and rollback |
| Save failure | active slice remains usable and prior saved state remains valid |

### Multi-actor and multi-client behavior

| Case | Required proof |
| --- | --- |
| Human observes agent | pointer, action, target, and result visible |
| Human takeover | active agent input pauses or cancels immediately |
| Two agents inspect one tab | concurrent reads do not block |
| Two agents mutate one tab | deterministic serialization and visible queue |
| Two agents use different tabs | actions proceed concurrently |
| Web and TUI prompt same agent | kernel ordering and transcript remain coherent |
| Two Web viewers | same stream and viewport without duplicate browser |
| Web plus remote TUI reconnect | one Room, one provider run, one browser state |
| Client resizes | canonical viewport ownership prevents resize fights |
| Permission prompt | one kernel interaction projected to every Chariox terminal |

### Security and isolation

| Case | Required proof |
| --- | --- |
| Wrong user | cannot discover or attach to environment |
| Wrong Room or slice | scoped token rejected |
| Expired token | viewer and input rejected, refresh path works |
| Stale machine heartbeat | target not selected |
| Unsafe endpoint origin | Cloud refuses embed |
| Host mismatch on secret paste | kernel rejects before resolving credential |
| Trace and log scan | no secret, passphrase, vault key, auth header, or clipboard residue |
| Relay inspection | relay routes opaque packets only |
| Cloud inspection | Cloud remains bootstrap/control plane, not runtime proxy |
| Cross-tenant concurrent use | no tab, stream, event, screenshot, or secret leakage |
| Malicious page | pointer overlay, tool references, downloads, and clipboard remain bounded |
| Capability grants | agent receives only granted browser, computer, vault, and file tools |

### Resiliency and failure injection

Resiliency is a release gate, not an incidental result of happy-path tests.
Each case must record the fault trigger, detection signal, user-visible state,
automatic recovery behavior, manual recovery path, recovery time, data loss or
duplication, resource delta, and cleanup result. Establish recovery time and
recovery point objectives from the first functionally correct local run. Ratify
them before rollout, and fail the release if a later implementation regresses
them without an explicitly approved tradeoff.

| Failure | Expected behavior |
| --- | --- |
| Relay disconnect | bounded reconnect, no duplicate action or provider run |
| Kernel restart or SIGKILL | durable Room and provider reconciliation |
| Worker restart | target becomes stale, then recovers through normal registration |
| Browser Controller crash | browser health changes, actions fail clearly, recovery is bounded |
| Chromium crash | controller detects loss and does not reuse stale references |
| Selkies crash | View degrades and retries without blocking kernel work |
| Viewer closes mid-action | agent action ownership remains deterministic |
| Queue saturation | fail-fast or bounded backpressure, relay readers stay live |
| Slow viewer | other viewers and agents remain unaffected |
| Network latency and loss | measured degradation and no protocol corruption |
| Low memory | admission rejection or graceful stop before OOM |
| Low disk | save and download fail before corrupting state |
| Save interrupted | previous saved state remains valid |
| External service forces reauth | exact prompt captured and product remains usable |
| Managed-machine hard reboot | reconnects through fresh heartbeat without a duplicate kernel, Room, or provider run |
| Managed machine becomes permanently unavailable | target is declared stale, user receives an actionable state, and another healthy machine remains selectable |
| Two managed machines partition | neither relay nor Cloud invents split-brain Room authority |
| Browser profile or saved-state corruption | restore fails safely, prior known-good generation remains recoverable, and corrupt state is quarantined |
| Save succeeds but acknowledgement is lost | retry is idempotent and does not create divergent generations |
| Restore interrupted after container creation | partial runtime is removed or reconciled before the next attempt |
| Controller restarts during queued mutations | completed actions are not repeated and incomplete actions fail or resume deterministically |
| Human takeover during reconnect | ownership does not revert silently to the agent |
| Web disconnect while TUI remains | TUI and kernel continue; Web reconnects to the same authoritative state |
| TUI disconnect while Web remains | Web and kernel continue; TUI replay does not duplicate interactions |
| DNS, TLS, or relay endpoint failure | connection fails loudly, uses no unsafe fallback, and recovers after the endpoint is healthy |
| Clock skew and expired tokens | refresh or rejection is deterministic and does not bypass authorization |
| Memory pressure with active slices | admission closes first; selected work is stopped gracefully before host OOM |
| Disk pressure during download and save | reserved headroom protects kernel state and last known-good save |
| Process and file-descriptor exhaustion | bounded failure preserves terminal traffic and emits an actionable diagnostic |
| Corrupt or unsupported protocol event | compatible clients continue where safe; incompatible clients fail with the minimum-version message |

Run faults at the beginning, middle, and commit boundary of state-changing
operations. Repeat the recovery path from Web, local TUI, and remote TUI, and
with each provider where provider-run reconciliation is involved. A recovery is
not successful merely because the UI reconnects: the test must verify the Room,
tab registry, provider thread, action ledger, permissions, files, browser
profile, and saved-state generation against pre-fault invariants.

### Scale and soak

Run the local cases first. Repeat the applicable cases on managed machines only
after the local scale and leak checks pass. Run at least:

- one Room, one browser, one user, one agent
- one Room, one browser, one user, three agents
- one Room, one browser, two Web viewers, one local TUI, one remote TUI
- one managed machine with the maximum safely admitted headed slices
- two managed machines with relay target selection and failure of one machine
- eight-hour active browser/controller/stream soak
- twenty-four-hour idle authenticated browser soak
- repeated disconnect/reconnect loop
- repeated save/restart loop
- repeated create/delete loop with leak comparison after every cycle

Scale is a resource-bound admission problem. The test must prove graceful
rejection at the limit, not merely find the point where the machine crashes.

## Regression matrix for adjacent Chariox features

Browser/computer work touches kernel, relay, slice, Cloud, provider, security,
and persistence paths. Every milestone must select and run the relevant rows.

| Chariox feature | Regression risk | Required validation |
| --- | --- | --- |
| Local and remote sessions | new environment events disturb attachment or focus | attach, detach, focus, reconnect, end, and delete |
| Provider runs | controller or save lifecycle duplicates or loses runs | launch, prompt, cancel, resume, crash, and cleanup for all providers |
| Provider-native TUI | parallel authority or missing interaction projection | local, remote, and slice native-TUI drills |
| Permissions | browser action creates a second approval path | one `RuntimeInteraction` across every terminal |
| Prompt attachments | uploads and downloads conflict with attachment transport | local, remote, and slice byte-transfer drills |
| MCP and skills | controller changes capability rendering | agent-scoped grants, revoke, collision, and isolation drills |
| Vault | browser controller or traces reveal secrets | full M26 and CHA-29 leak matrix |
| History and Recall | browser/action events corrupt or overwhelm history | persistence, filtering, replay, and bounded event volume |
| Workflows | browser tools block scheduler or relay readers | concurrent workflow and browser task with cancel/retry |
| Workflow endpoints | managed-machine work changes routing | connected ingress and deployment invocation drill |
| Workspace Live Sync | downloads/uploads and slice files collide with sync | managed, tracked, cross-branch, conflict, and permission drills |
| Managed remote kernels | headed slices affect heartbeat and admission | provision, restart, stale heartbeat, resource cap, teardown |
| Slice lifecycle | streamer/controller leave orphan state | create, start, stop, save, restart, reset, backup, delete |
| Multi-user collaboration | shared browser leaks across tenants | two-user same Room and cross-Room isolation |
| Relay identity and transport | display load starves terminal traffic | terminal prompt under full display load and stalled viewer |
| Cloud waiting room | new display states break machine selection | local, hosted, stale, headless, and protocol-incompatible targets |
| Notifications | browser and lifecycle events produce duplicates or secrets | reconnect deduplication, redaction, and actor attribution |
| Agent apps and deployments | controller packages alter deployment images | build, deploy, rollback, and image-size comparison |
| CLI and TUI rendering | new events break old terminals | minimum protocol, unknown event, resize, trace, and reconnect |
| iOS and planned Android clients | protocol changes break native projections | protocol snapshots, minimum version, client build/tests |
| Self-hosted mode | hosted endpoint assumptions break local users | local `ws://`, local-only viewer, and explicit open behavior |
| Reviewer infrastructure | resource drills disturb shared reviewer services | inventory before/after; never stop shared reviewers |

## Acceptance drills

### Drill A: deterministic local browser state

Run a local authenticated fixture through first-party browser tools. Set cookie,
localStorage, IndexedDB, Cache Storage, and service-worker state. Install a
small graphical program. Save, remove the container and home volume, restore,
and verify every marker through the same agent.

### Drill B: real service and vault

From Web View, ask a slice agent to sign into Gmail or a comparable public
service using a vault-backed credential. Send a first message, save and restart
agents, verify the same provider thread and authenticated browser, then send a
second message. If the service forces reauth, capture the exact prompt and
compare it with the deterministic fixture.

### Drill C: TUI-to-Web shared browser

Using a local kernel, a local Docker slice, and a same-host relay, attach local
TUI, remote TUI, and Web to one Room. Prompt from the TUI to open and edit a
page. Observe the same action in Web. Take over in Web, then verify TUI activity
and traces reflect the human actor and final browser state. Milestone 9 repeats
this unchanged drill against the OpenShip-provisioned machine and hosted relay.

### Drill D: non-browser office work

Ask the agent to open a graphical editor or another installed desktop
application, manipulate a document with screen and input tools, save it, then
attach it to an email. Verify browser focus is not forced during desktop work.

### Drill E: concurrency and arbitration

Use three agents. Two inspect one tab while the third works in another tab.
Queue two mutations on the same tab, then perform human takeover. Prove reads,
serialization, cancellation, attribution, and independent-tab concurrency.

### Drill F: managed-machine recovery

Provision a fresh OpenShip-backed managed machine, complete a browser task,
disconnect clients, restart the worker or machine, reconnect, and continue the
same Room and provider thread. Repeat with relay interruption and stale target
selection.

### Drill G: office-work suite

Complete the five scenarios from `docs/AGENT_SLICE_OFFICE_WORK_DRILLS.md`:

- email-gated SaaS onboarding
- vendor research and CRM-like entry
- document intake and follow-up email
- public API workflow with an agent-created extension
- support ticket lifecycle

### Drill H: load, soak, and cleanup

Run the approved maximum slice count, multiple viewers, Web and TUI traffic,
and a normal workflow concurrently. Sample resources and latency, inject one
slow viewer, stop every task, and compare the final process/container/volume/
port/resource inventory with the initial inventory.

### Drill I: resiliency campaign

Execute every locally reproducible resiliency case on local Docker first while
Web, local TUI, and remote TUI are attached through the same-host relay. Do not
provision remote infrastructure until that campaign passes and cleans up.
Then repeat the applicable matrix on a fresh OpenShip-provisioned managed
machine. Inject relay, kernel, worker, controller, Chromium, Selkies, network,
host, memory, disk, token, and persistence faults at deterministic operation
boundaries. The remote pass also includes a managed-machine hard reboot and a
two-machine partition scenario. For every fault, prove bounded detection and
recovery, single Room and provider authority, no duplicated or silently lost
completed action, preservation of the last known-good saved state, accurate
user-facing status in every client, and full resource cleanup. Retain recovery
time, recovery point, memory, CPU, disk, connection metrics, and the event
timeline as functional evidence. Performance comparison waits for Milestone 10.

## Cleanup contract

Every test and drill must own an explicit cleanup ledger. Cleanup runs on
success, assertion failure, timeout, interruption, and provider failure.

Clean up all drill-owned:

- Docker containers, images created only for the drill, volumes, and networks
- slice records, state archives, backups, and temporary homes
- Browser Controller, Selkies, Chromium, Xvfb, noVNC fallback, and provider
  processes
- kernels, workers, relays, sessions, agents, workflows, and temporary grants
- SSH processes, port forwards, tunnels, listeners, and temporary firewall
  rules
- managed machines, OpenShip deployments, temporary DNS, and Cloud records
- downloaded files, upload fixtures, screenshots not retained as evidence, and
  temporary credentials
- environment variables, shell profiles, browser profiles, and Keychain export
  files created for Linux provider authentication

Never print credential payloads. Temporary credential files must have mode 600
and must be deleted after verification. Do not prune unrelated Docker state,
shared kernels, active development slices, or reviewer infrastructure.

The final cleanup assertion must prove:

- no drill-owned container or managed machine remains
- no drill-owned process or listener remains
- no drill-owned active kernel target or heartbeat remains
- no temporary credential or secret-bearing file remains
- memory, swap, and disk return to the expected post-run range
- retained evidence contains no secret

If cleanup is incomplete, the drill is failed even when its functional
assertions passed.

## PR completion gate

No PR in this program is ready to merge until:

- scope is limited to one coherent milestone and one repository
- every completed subtask has its own committed and pushed checkpoint
- the exact head has no remaining reviewer comments
- required protocol version and snapshots are updated
- unit, integration, compatibility, and selected live drills pass
- Web and TUI implications are tested or explicitly not applicable
- managed-machine implications are tested when the milestone reaches that path
- resource safety and cleanup evidence is attached
- performance comparison is attached only for Milestone 10 and later; earlier
  functional PRs must not be held open for benchmark tuning
- security and secret scans pass
- cleanup assertions pass
- documentation, Linear issue, PR body, and evidence manifest agree
- rollback behavior is documented and tested

## Program completion gate

CHA-16 is complete only when:

- Selkies is the reviewed default and noVNC is removed after its rollback
  window
- the long-running Browser Controller owns the shared tab registry
- Browser and Computer modes use one Room environment and authority path
- Web and local/remote TUI clients prove parity against one live environment
- all three providers pass structured browser and computer fallback drills
- user takeover, multi-agent concurrency, permissions, and actor traces pass
- state, installed programs, browser authentication, and provider threads
  survive save/restart and full recreation
- vault-backed public-service work passes leak scans
- the OpenShip-backed Chariox managed-machine drill passes from provisioning
  through teardown
- Chariox has a verified first-place public result on every relevant maintained
  browser-use and computer-use benchmark, with all inclusions and exclusions
  recorded
- benchmark-driven optimizations preserve every local and remote functional,
  security, resiliency, compatibility, and cleanup gate
- resource, scale, and soak gates pass on local and managed infrastructure
- adjacent Chariox regression matrix passes
- exact-head reviewers have no comments on every merged PR
- all temporary infrastructure and local artifacts are cleaned up
