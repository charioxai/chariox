# AGENTS.md

## Project Map

Chariox spans two main repositories:

- `chariox`: open-source runtime. It owns the kernel, relay, local/remote TUI CLIs, provider adapters, iOS work, and planned Android client foundations.
- `chariox-cloud`: hosted control plane and web app. It owns browser auth, Cloud waiting room, hosted relay token issuance, staging deployment, and the web CLI UI.

## Runtime Architecture

The kernel is the runtime authority. It owns sessions, agents, provider runs, workspaces, worktrees, prompt history, terminal events, and state transitions.

Agents run through provider CLIs or adapters launched by the kernel. Chariox orchestrates them but must not become the provider credential store or provider-internal state owner.

Chariox supports Codex, OpenCode, and Claude through their official provider harnesses only: provider CLIs, provider-local servers, and documented provider-native hooks/protocols. Do not use provider SDKs, third-party agent services, or alternate hosted agent runtimes for Chariox provider execution.

The relay is transport only. It admits scoped connections and routes encrypted packets. It must not become a session authority and must not inspect prompts, outputs, workspace data, provider payloads, or session history.

Primary connectivity:

- Local TUI CLI: `local TUI <-> kernel <-> agent`
- Remote/orphaned TUI CLI: `remote TUI <-> relay <-> kernel <-> agent`
- Web CLI: `browser <-> hosted relay <-> kernel <-> agent`; Cloud is bootstrap/control plane only and must not proxy runtime terminal traffic.
- iOS client: native client planned to use the same kernel/relay protocol surfaces.
- Android client: planned, same architecture as iOS.

Cloud may authenticate users, issue relay tokens, select relay targets, and display waiting-room/control-plane state. Cloud must not fork kernel runtime behavior.

## Native Provider TUI Contract

Native provider TUI mode (`chariox codex`, `chariox opencode`, `chariox claude`) must reuse the normal Chariox runtime paths. For local native TUI, provider prompts enter the kernel through the same prompt path as Chariox clients, and Chariox-origin prompts go through the kernel-managed provider run so the provider TUI observes the same turns. For remote native TUI, provider TUIs and Chariox TUIs attach to the home kernel session; the home kernel dispatches to the worker through the existing leased-agent relay protocol; the worker kernel uses the existing provider adapter/server path. For slice-backed native TUI, provider TUIs still attach to the home kernel session; the slice is only the home-managed worker execution environment selected by `slice_ref`. Do not add a parallel prompt, permission, attachment, history, or relay authority path for native TUIs. See `docs/PROTOCOL.md` section `3.3.2 Native TUI Agents` and `docs/ARCHITECTURE.md` section `5.3.2 Native TUI Client Interface`.

For native TUI MCP/skills, keep standard home-worker and slice behavior distinct. Standard home-worker does not install or copy MCPs/skills across machines; the user/operator must make matching capabilities available on the worker. Slice-backed native TUI may transfer home skill packages to the child worker because the home kernel manages that execution environment. See `docs/PROTOCOL.md` section `3.3.2 Native TUI Agents` and `docs/M14B_NATIVE_TUI_VALIDATION_PLAN.md` for the current validation matrix.

Claude native TUI hidden prompt context must use the `UserPromptSubmit` hook `additionalContext` bridge, not visible PTY prompt injection; see `docs/PROTOCOL.md` section `3.3.2 Native TUI Agents`.

Native TUI permission prompts must resolve through one kernel-owned `RuntimeInteraction` projected to every Chariox TUI in the session; provider-native approval replies should route back to that interaction when the provider seam allows it.

## Protocol Change Rule

When changing `LocalDaemonRequest`, `LocalDaemonResponse`, relay terminal events, browser/kernel terminal transport semantics, or any serialized protocol shape that a CLI or app depends on:

1. Increment the shared local daemon protocol version in OSS.
2. Update protocol snapshot/hash tests so CI fails if the protocol shape changes without a version bump.
3. Update the web/native minimum supported protocol version only when that client depends on the new behavior.
4. Add or update a focused drill that exercises the changed protocol behavior.

Do not merge protocol shape changes without the version bump and test update.

## Implementation Rules

- Work on the `main` branch unless the user explicitly states otherwise.
- Run GitHub CI only when the feature, plan, or goal is fully implemented and ready to merge into `main`; use focused local or remote checks and `[skip ci]` commits until then.
- Keep core behavior below clients, in kernel services and shared protocol contracts.
- Do not implement behavior only in the web app or only in the TUI unless explicitly marked temporary.
- Prefer one shared protocol path across local TUI, remote TUI, web, and native clients.
- Hosted Cloud relay runtime should use the Caddy-fronted `wss://` relay URL for browser, kernel, remote TUI, and kernel-to-kernel remote-agent connections. Local and self-hosted relay setups may keep using `ws://`.
- Use heartbeat freshness for relay target selection; stale targets must not be treated as online.
- Preserve local/dev/self-host compatibility where practical, but fail loudly when hosted Cloud configuration violates the runtime architecture.
- The independent reviewer services and state under `~/.chariox-reviewer` are shared infrastructure, not an unrelated-worktree cleanup target: do not kill, disable, restart, or prune them unless the reviewer itself is being repaired or the user explicitly asks to stop it.
- Be lean, don't over engineer and delete all old/unnecessary code along the way.
- Keep coordinators as wiring only; move policy, state mutation, rendering, transport I/O, and protocol adapters into named responsibility modules before a file becomes a mega-file.
- Always clean up temporary drill artifacts, orphaned provider processes, and large build outputs you no longer need before handing work back.
- Store screenshots and other validation evidence under `/Users/miguel/.codex/evidence/<task>/`, never inside a repository or in Git.
- Keep source checkouts and worktrees outside `~/.chariox` (for example under `~/chariox-worktrees/<task>/`). A kernel without an explicit `CHARIOX_HOME` protects all of `~/.chariox` and rejects provider working directories inside it.
- Store persistent local development and drill state under `~/.chariox/dev/<task-or-kernel>/`. Set `CHARIOX_HOME` to an explicit absolute subdirectory there; use `mktemp -d` for disposable state. Never point `CHARIOX_HOME`, `CHARIOX_LOG_DIR`, kernel state, or drill scratch roots inside a repository.
- Never create `.arroba` directories or other paths using the retired product name. A workspace `.chariox/` directory is allowed only for explicit user-authored workspace-scoped capabilities or source; automatic logs, runtime mailboxes, generated workflow code, test state, and drill artifacts must remain outside repositories.

## Validation: real live drills

Validation means **real live drills: the exact scenarios real users encounter, with real providers and real everything**:
- real public sites, services and accounts;
- real official provider agents (Codex, Claude, OpenCode);
- real clients (the web app in a real desktop browser, including Retina/DPR 2, and the real TUI);
- the real hosted relay and real networks with realistic uplink and latency;
- real machines;
- realistic durations.

Synthetic fixtures, local mocks (mail catchers, local OIDC, fixture pages, stub agents) and component tests are supplementary regression checks only. They never count as acceptance.

A feature is done only when its real live drills pass with explicit usability thresholds:
- coverage of the listed real sites and services;
- input-to-visible latency;
- smoothness;
- success on every listed case.

Every drill must be shown to fail before the change and pass after, with screenshots and logs. Treat any fallback, degraded mode or "known limitation" as a blocker until it is checked under real conditions. If a real resource is missing (account, credential, machine), report the feature as blocked and name the exact owner action; never substitute a mock and call it accepted.

Nothing is staged or handed to the owner for testing until the feature is completely finished and validated this way.

## Provider-Native Permission Visibility

Native provider permission prompts are surfaced to the user out-of-band through Chariox runtime interactions. Do not infer that no approval prompt appeared just because a shell/tool result lacks `approval requested` or `approved` metadata. The result visible to the agent normally contains only the provider tool execution outcome, such as stdout/stderr, exit code, and status after the user has already answered the prompt.

## Claude credential transfer for Linux runners

Claude Code on macOS stores a live login in a profile-scoped Keychain service named `Claude Code-credentials-<hash>`, where `<hash>` is the first eight hexadecimal characters of SHA-256 over `CLAUDE_CONFIG_DIR`. Older default profiles may use `Claude Code-credentials`. Linux runners and containers expect the credential at `~/.claude/.credentials.json`.

Chariox must export the exact profile-scoped Keychain item automatically when materializing a linked or managed Claude account. It may use the legacy unscoped item only for the default profile. Reject empty or non-refreshable credentials before provisioning instead of copying them. For a manual runner drill outside the managed-context path, export the matching scoped item to a temporary local file, copy it into the runner credential profile home as `.claude/.credentials.json`, set mode `600`, verify with `HOME=<profile-home> claude auth status`, and delete the temporary local file. Never print the credential payload or commit it.

## Key retention and rotation

- Private signing keys and provider credentials are durable assets, never build
  artifacts. Keep them outside repositories, worktrees, task scratch, evidence,
  container layers, and compiler output. Use a dedicated `~/.chariox/keys/`
  directory with mode 0700 and private files with mode 0600.
- Before using a key, verify its public fingerprint against the approved public
  inventory. Never print private material, put it in prompts/logs, or transfer it
  to a builder or deployment target. Targets receive public verification pins only.
- Before first use, create a separately protected backup and prove restoration
  by deriving its public key and signing/verifying a synthetic challenge. Record
  only public fingerprints, locations, owner, purpose, date, and backup scope.
  A same-machine backup does not protect against disk or machine loss.
- Before cleanup, inventory credentials and protected paths; exclude key stores,
  backups, credential profiles, and shared reviewer state. Never recursively
  delete a parent containing them. Stop if ownership or contents are unclear.
- Key loss is not permission to substitute another key. Obtain explicit rotation
  authorization, generate a new epoch, update reviewed public trust pins and
  signing inputs, and test acceptance of the new key plus rejection of the old
  key. Never edit an already signed artifact or claim rotation is deployed before
  the consuming systems have been verified. Preserve public rotation history.
- Keep host-specific key inventories, backup locations, and recovery gaps in
  protected operator-local notes, not public repositories. Exclude all recorded
  key and backup locations from artifact/worktree cleanup. Machine-loss recovery
  remains unproven until an approved off-device backup has passed a restore test.

## Coding style

Simple: be clean and minimalistic. Strive for simplest solution always.
