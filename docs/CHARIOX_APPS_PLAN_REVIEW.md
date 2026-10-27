# Chariox Apps plan review

Reviewed on 2026-09-05. This is a design review, not implementation certification. The findings have now been incorporated into the implementation plan with the user decisions below. The numbered findings preserve the reasoning at review time; the updated plan governs implementation.

Scope: the complete [Apps implementation plan](/Users/miguel/arroba-cloud/docs/CHARIOX_APPS_IMPLEMENTATION_PLAN.html), both project glossaries, the [shared Environment plan](BROWSER_COMPUTER_USE_END_TO_END_PLAN.md), [Environment protocol](PROTOCOL.md#412-shared-room-environment-protocol-direction), [event protocol](EVENT_TRIGGER_PROTOCOL.md), and selected current launch and event code. Platform claims were checked against primary sources in the accompanying [platform review](CHARIOX_APPS_PLATFORM_REVIEW.md).

## Incorporation and user decisions

- The plan now has independent Phase 1 and Phase 2 release matrices. Phase 1 ships only after all its required validation passes; Phase 2 reruns Phase 1 regressions and adds its own gates.
- Chromium sandbox restoration includes the reported Google-session persistence regression, treated as an investigation hypothesis until reproduced and explained.
- General multi-file filesystem transactions are deferred. Ordinary private file APIs, structured-state transactions and browser IndexedDB remain supported.
- R5 is resolved with medium-independent human/agent operation. Bindings select tool discovery; critical actions use kernel-owned human validation and parameter-bound effect enforcement. They do not introduce a separate agent UI permission lane.
- R7 is resolved as a full Slack proof and cutover: keep existing behavior only until the new generic contract passes verification, then migrate and remove obsolete code. No permanent Slack exception is retained.
- Remaining findings are incorporated into resource qualification, generation-safe updates, event/outbox semantics, viewer accessibility, network compatibility, package trust, runtime maintenance and phase-specific tests. These are implementation obligations, not claims of tests already passing.

### Subsequent App/kernel decisions

The implementation plan now also includes the agreed core App work experience. One binding mechanism reuses explicit selection, foreground activation and existing agent self-grant/YOLO/Ask policy. Apps reserve a trusted conversation panel but never receive transcript subscriptions. Named information sets require explicit user consent before correlated model outputs are delivered through existing output/handoff validation mechanisms. Ordinary App functions are sufficient for selection, objects and semantic checks. Phase 1 includes generic asset operations and a second agent resuming App-owned task state; coordinated cross-App work is Phase 2. See P1.16–P1.19, P2.10 and their phase-specific validation rows. These decisions supersede broader transcript-access or separate permission-system options discussed during review.

## Assessment

Keep the main architecture. Rust kernel authority, one sandboxed Node backend per installation, App views in managed Chromium, terminal projections, one runtime MCP, and workflow-owned agent activation fit Chariox. The two phases remain sensible, with macOS and Linux kernels first and Windows second.

The document is a good statement of product requirements. It is not yet a complete implementation contract. Several guarantees lack an enforcing mechanism, and a few statements are technically too strong. My earlier contributions to this plan introduced some of these overstatements, especially the storage transaction guarantees, blanket Node compatibility language, and assumptions about Chromium isolation.

Resolve findings R1 through R8 before committing the corresponding implementation. They do not require abandoning the architecture. R9 through R12 refine compatibility, distribution, operations, and delivery scope.

## Findings and changes

### R1. Enable Chromium isolation on the actual App view execution path

Priority: high. Evidence: plan UI and V-UX-01; [current Linux launcher](/Users/miguel/arroba/apps/kernel/slice-linux-docker/docker/slice-screen.sh:227) and [fallback launch](/Users/miguel/arroba/apps/kernel/slice-linux-docker/docker/slice-screen.sh:593).

The plan says Chromium provides renderer isolation, but these existing launch paths use `--no-sandbox`. A container around the shared Environment does not replace renderer isolation between Apps, browser services, and other content within that Environment. The plan must explicitly depend on a sandbox-enabled production browser configuration, including nested container support where used. This is a finding about the inspected paths, not proof that every Chariox browser launch uses them.

Specify the installation origin/site and storage partition mechanism. Do not assume different URL paths or subdomains alone provide the desired process and cookie boundaries. Cover service workers, nested frames, external navigation, downloads, WebRTC, and direct requests made by the view. CSP is an additional browser policy, not the entire installation network boundary. Service calls must not escape the backend's restrictions through the view. See [Chromium security FAQ](https://chromium.googlesource.com/chromium/src/+/main/docs/security/faq.md) and [CSP specification](https://www.w3.org/TR/CSP3/).

### R2. Finish the platform resource contract

Priority: high. Evidence: plan worker enforcement, P1.03, V-RUN-01 and V-RUN-02.

Seatbelt access restrictions, process timeouts, and a V8 heap limit do not establish the promised macOS total-memory ceiling. Likewise, naming Linux namespaces and cgroups does not specify how an ordinary installation gets the required namespace and cgroup access. Chromium proves that cross-platform containment is practical; it does not supply Chariox's resource guarantees automatically.

Name the shipped launcher, required privileges or delegated resources, supported OS versions, and concrete CPU, total-memory, thread, file descriptor, storage, and broker limits. Include Buffer and native allocations, browser renderers, browser storage, queued events, decompression, and network response buffers. Separate hard limits from monitored thresholds and bound termination delay and overshoot. Add aggregate admission limits across installations so many individually conforming Apps cannot overwhelm a small machine.

This remains Chariox's installation responsibility. Developers should receive the same supported package contract without configuring the sandbox themselves. See the [platform findings](CHARIOX_APPS_PLATFORM_REVIEW.md).

### R3. Narrow the filesystem transaction guarantee

Priority: high. Evidence: plan private filesystem rules, P1.04, V-SDK-01.

An App can write a file through `node:fs` while `chariox.files` changes the same file. SDK version counters cannot prevent that uncooperative write. Nor does a sequence of ordinary renames provide an atomic multi-file view to arbitrary filesystem readers. The current wording mixes ordinary filesystem semantics with database-style isolation.

Keep standard private `node:fs`. For Phase 1, provide atomic single-file replacement, import/export, and snapshots with explicit consistency levels. Put compare-and-set and multi-record transactions in `chariox.state`, using the kernel's existing transactional persistence where suitable. Defer general multi-file transactions unless they operate on a separately managed store with a defined access contract. SQLite's [atomic commit design](https://www.sqlite.org/atomiccommit.html) illustrates why coordinated transactions require locking and recovery, not only a version counter.

For updates, block new operations and fence every writer, including timers, open descriptors, and background callbacks, before taking the snapshot. A cooperative shutdown callback alone is insufficient. Define whether a snapshot is application-consistent or crash-consistent. Account for retained snapshots and staging copies in disk budgets.

### R4. Define the crash boundaries around events and external effects

Priority: high. Evidence: plan update completion, P1.06, P1.10, P1.12, V-SDK-05/06; [existing event delivery contract](EVENT_TRIGGER_PROTOCOL.md#delivery).

The existing event contract persists a receipt with a queued workflow prompt before acknowledging upstream. Inserting an App handler creates additional crash windows between receipt, App state mutation, event emission, and acknowledgement. A stable occurrence ID alone does not close all of them.

Specify at-least-once delivery to App handlers, durable App outbox/replay behavior, and an atomic dedupe-plus-workflow-enqueue operation owned by the kernel. Define occurrence identity scope, schedule revisions, replay retention, expiry, poison-event handling, and bounded queues. SDK-managed state can expose a transaction that records state and an outgoing occurrence together; Apps using raw files need a documented durable outbox pattern.

Keep delivery receipts and emitted occurrence tombstones outside rollbackable App snapshots. Restoring old data must not re-run an already delivered reminder. Do not promise exactly-once external effects. If a remote service accepted a mutation before a timeout, cancellation cannot undo it. Reuse provider idempotency keys where supported and expose an unknown outcome where reconciliation is required.

### R5. Make authorization consistent across tools, views, and background code

Priority: high. Evidence: plan agent access decision, App view bridge, V-SDK-04 and V-UX-10; [Environment action contract](PROTOCOL.md#action-envelope).

The plan defines agent binding as the grant but only tests it on MCP calls. An unbound agent may still have Room browser/computer access to an App view. Decide whether binding grants only tool availability or whether it restricts App use through every route. My recommendation is to apply the intended App action policy to both routes and document any deliberate shared-Room access. A claim of per-agent data secrecy needs stronger isolation than a shared desktop.

Carry kernel-authenticated installation, Room, actor or background identity, release generation, and operation context. Never infer a human gesture from a boolean supplied by App JavaScript. App-origin identity does not prove which actor caused an asynchronous script callback. Sensitive actions need kernel-issued operation context or a kernel-owned interaction. Background callbacks use their installation capabilities and automation grants, not the last user's identity.

Recheck grants on execution, not only when listing tools. Define revocation for in-flight calls and stale views, nested frames, copied handles, and update generations. Use these rules for terminal-local file pickers and clipboard access as well.

### R6. Complete the shared-view contract for mobile and accessibility

Priority: high. Evidence: plan UI SDK contract, V-UX-02/03, native viewer workstream; [canonical viewport protocol](PROTOCOL.md#viewport-contract).

One shared DOM has one active layout viewport. A 1440-pixel desktop layout scaled to a phone remains a desktop layout. Preserve the shared Tab, but let the authorized input owner request the canonical viewport through the kernel. Other viewers scale or letterbox it. Specify what happens when ownership moves between differently sized terminals.

A rendered stream alone does not give the terminal's screen reader the remote page's semantics. Add an explicit accessibility projection with roles, names, values, focus, live updates, and authorized actions. Chromium's [Accessibility domain](https://chromedevtools.github.io/devtools-protocol/tot/Accessibility/) can supply observations; it does not itself wire those observations into terminal assistive technology. This is an engineering requirement inferred from the separate remote and local accessibility trees.

Include text composition, selection, mobile keyboard behavior, and clipboard round trips. These belong in the shared Environment viewer workstream so Apps reuse one implementation. Test accessibility at the user's terminal, not only in managed Chromium.

Also define view identity across Rooms. Recommend one backend per installation and a shared App Tab per Room/installation. Browser storage remains scoped to its actual Environment and installation; durable shared App data belongs in the backend. State explicitly whether browser storage is disposable cache or is included in backup and update migration.

### R7. Specify the AEGS/AEDS migration and credential ownership

Priority: high. Evidence: plan external event path and P1.06/P1.11; [event protocol](EVENT_TRIGGER_PROTOCOL.md), [AEGS SDK](AEGS_SDK.md).

Current AEGS implementations own provider credentials and upstream subscriptions. AEDS routes by workflow-owned bindings, and the kernel receives canonical workflow input. The new plan introduces installation inboxes and says the HTTP broker injects credentials while Slack retains its normal AEGS connection. Those pieces do not yet define one complete route or credential flow.

Preserve AEGS-held credentials for existing integrations and call its bounded action interface using opaque connection handles. Define any new kernel-held service credential facility separately, including enrollment and revocation; an AEGS handle cannot simply be converted into a local access token. Chariox provider harness credentials remain outside this App facility.

Specify how an App installation maps to AEDS routes, existing workflow bindings, source occurrences, and automation IDs. Version the affected delivery/management protocols and preserve intentional fan-out across distinct automations. Clarify whether the App receives normalized service events or already composed workflow prompts. Migrate Slack with durable ID mapping and dedupe before deleting the old path. New service authorization should follow [OAuth security best practice](https://www.rfc-editor.org/rfc/rfc9700.html).

### R8. Make activation and update one durable generation change

Priority: high. Evidence: plan package lifecycle, update completion, V-PKG-05/06.

An atomic active-release pointer does not atomically change worker code, state schema, approved capabilities, MCP catalog, open views, and browser caches. Define a durable installation generation and an update journal with recoverable states. Fence old workers and bridge sessions, pause timers and callbacks, and reject stale-generation calls. Remove or version old service worker caches.

Capability expansion requires a recorded decision before activation. If declined, keep the working old release. A schema change can also turn an existing tool into a more powerful action, so compatibility and action-policy review must cover semantic changes, not only manifest syntax.

Limit automatic snapshot rollback to the pre-commit update interval. After the new version accepts ordinary writes or external effects, reverting to a pre-update snapshot can erase valid user work and cannot undo service mutations. Define a later downgrade/recovery procedure separately. Migrations and pre-commit health checks must not perform irreversible external effects.

### R9. Publish a bounded Node compatibility contract

Priority: medium. Evidence: plan Node compatibility table and P1.04.

Keep Node. Private filesystem support and npm ecosystem familiarity match the intended App backend. Use a maintained LTS release, pinned by Chariox, and keep the Rust supervisor responsible for authority. Embedding Node is reasonable for a native launcher that applies containment before loading App code. It is not itself a security feature; minimize divergence from upstream.

Pure-JavaScript packages are not automatically compatible. Many use `node:https`, custom dispatchers, workers, package assets, dynamic require, or a local server. Define tested supported APIs, transport adapters, and packaging behavior. A browser UI bundle means adapting Next.js or Express server assumptions; it is not an unchanged full-stack deployment.

Make Fetch streaming, cancellation, upload bodies, redirects, error mapping, and backpressure explicit. HTTP destinations need address validation at connection time, credential-to-origin/path binding, and quotas on the trusted side. Specify SSE and WebSocket support or a precise v1 exclusion. The [Fetch standard](https://fetch.spec.whatwg.org/) defines more behavior than JSON request/response helpers. See additional [platform findings](CHARIOX_APPS_PLATFORM_REVIEW.md).

Use representative API SDK, file-backed, and streaming integrations as regression fixtures. Record actual source edits and adapters. Native dependencies serve as clear compatibility diagnostics while excluded; they should not become a mandatory v1 capability merely because they appear in a benchmark.

### R10. Correct the reviewed-code-only claim and reuse update security

Priority: high for the false security claim; medium for distribution design. Evidence: plan compatibility table line 1106, P2.03/05.

An App with allowed HTTP, writable files, and normal JavaScript execution can fetch text and interpret it as code. Blocking npm installation scripts and making package files immutable do not prevent this. The runtime sandbox must contain all executed code regardless of whether package review saw it. State the precise packaging policy instead of promising that installed Apps cannot download code. Automated and agent-led review provide bounded evidence, not proof that an App is safe.

For the Library, use an existing implementation of The Update Framework if practical for trusted metadata, key rotation, expiry, and rollback/freeze resistance. Keep `.cxapp` as the package format. [TUF](https://theupdateframework.github.io/specification/latest/) deliberately separates trusted distribution from package installation; this reduces custom security protocol work without replacing the Chariox lifecycle.

Define the signature envelope and exact signed bytes, including the manifest and package inventory but excluding recursive signature contents. Cover local developer key trust in Phase 1. Distinguish authorized recovery to an older release from a network attacker replaying old metadata.

### R11. State timer and operational guarantees that can be measured

Priority: medium. Evidence: plan timer decision, P1.12 and resource tests.

Keep App-owned schedules. Replace "exactly once at its due time" with durable due-time intent, a lateness target while running, and overdue recovery after sleep or shutdown. Node documents that timer delays above 2147483647 milliseconds become 1 millisecond and that callbacks do not have exact timing guarantees. Use bounded re-arming and persisted due times. See [Node timers](https://nodejs.org/api/timers.html#settimeoutcallback-delay-args).

Pick measurable budgets for installation startup, idle backend/Tab footprint, input latency, event latency, queue depth, restart rate, and broker throughput on named reference hardware. Define runtime and Chromium security patch ownership and a maximum supported version age. Do not treat a pin as a permanent runtime version.

### R12. Clarify phase dependencies and avoid multiplying every test

Priority: medium. Evidence: phase workstreams and surface validation matrix.

Phase 1 and Phase 2 both claim package file associations/side installation. Assign local file association to one delivery point and public Library trust/discovery to the other. Separate supported kernel operating systems from terminal platforms: a macOS kernel can be Phase 1 while its native desktop terminal is Phase 2. Headless backend installation should not force Chromium startup until a view is needed.

Make the shared Environment viewer and sandbox launch path explicit dependencies. Keep native-addon support optional until developer demand justifies its packaging and security maintenance. General file transactions, ratings, and broad telemetry can be sequenced independently of a usable local App platform.

The validation matrix is already broad. Increase precision and cover missing boundaries rather than testing every kernel invariant through every terminal. Run core state-machine and adversarial tests once per backend implementation; run transport authorization on local and relayed paths; run presentation/input/accessibility per terminal. Add pairwise topology cases and full reference flows for the release architectures.

## Proposed verification and validation additions

These are proposed requirements, not tests executed during this review. Extend existing cases where possible.

| ID | Placement | Test and expected evidence |
| --- | --- | --- |
| V-ADD-01 | Extend V-UX-01 | Inspect production launch configuration and renderer sandbox state in local and managed Environments. Include fallback/recovery launches. App views never depend on `--no-sandbox`. |
| V-ADD-02 | Extend V-RUN-01/02 | Total memory via Buffer, native allocations, thread growth, browser rendering, and slow broker consumers. Saturate many installations. Measure kernel latency, kill latency, memory overshoot, and aggregate reserves on named hardware. |
| V-ADD-03 | Extend V-SDK-01/02 | Raw `node:fs` writes race SDK writes; open descriptors and timers survive update preparation. Verify only the documented consistency guarantee. Test retained snapshots, caches, staging files, and host disk pressure against total quota. |
| V-ADD-04 | Extend V-SDK-05/06 | Kill after inbox persistence, App mutation, outbox record, emission, enqueue, and before each acknowledgement. Replay with the same occurrence. Exactly one durable enqueue per declared dedupe key, with no accepted event silently lost. |
| V-ADD-05 | Extend V-PKG-06 | Emit an occurrence, update, roll back App data, and replay. Kernel receipts survive rollback. Incompatible UI/backend/state generations cannot operate together. |
| V-ADD-06 | Extend V-SDK-07 | Remote POST succeeds but its response is lost. Timeout/cancel reports an unknown effect unless the provider supports idempotency/reconciliation. No automatic unsafe retry. |
| V-ADD-07 | Extend V-SDK-04 and V-UX-10 | Same action through MCP, DOM automation, desktop coordinates, and a human viewer. Unbind or revoke mid-call. Outcomes match the chosen action policy; App messages cannot forge actor or user-gesture authority. |
| V-ADD-08 | Extend V-PKG-05/06 | Update expands network or file access, changes tool meaning, or has stale views/service workers. Declining expansion preserves the old release; no new capability is usable before approval. |
| V-ADD-09 | Extend V-UX-01 | Attack cross-App cookies, IndexedDB, caches, workers, frames, popups, navigation, fetch, WebSocket, WebRTC, and downloads. Include requests to host loopback, local network, and metadata addresses. |
| V-ADD-10 | Extend V-UX-02/03 | Desktop and phone attach concurrently. Transfer viewport ownership, then type with an IME, select text, and use the terminal's screen reader. Semantic updates and actions track the same managed Tab and document revision. |
| V-ADD-11 | Extend V-SDK-07 | Large streaming uploads, SSE, abort after headers, slow response consumption, decompression bombs, mixed IPv4/IPv6 DNS answers, redirects, and credentials scoped to exact destinations. Bound trusted-process memory and connection counts. |
| V-ADD-12 | Extend V-SDK-05 | Migrate an existing Slack connection and trigger without credential export or duplicate prompts. Reconcile old/new routes, replay old deliveries, preserve intentional fan-out, and recover a lost authorization callback. |
| V-ADD-13 | Extend V-SDK-06 | Timer beyond 25 days, sleep/wake, clock jumps, DST-sensitive schedules, edited/deleted Todo, and update spanning due time. Persisted occurrence revision controls delivery and obsolete schedules do not fire. |
| V-ADD-14 | Extend V-UX-10 and inheritance | Same installation open in two Rooms, then update/uninstall. Verify documented shared data and isolated view state. Browser storage follows the chosen backup and deletion policy. |
| V-ADD-15 | Extend package/review validation | Fetch-and-evaluate hostile code, hidden install hooks, package tampering, case/Unicode archive collisions, and parser/IPC fuzzing. Security holds independently of review results. |
| V-ADD-16 | Library updates | Replay old signed metadata, expire metadata while offline, rotate keys, revoke a release, and recover from a bad rollout. Distinguish intentional downgrade from attacker rollback. |
| V-ADD-17 | SDK conformance | Pin representative API, file-backed, and streaming fixtures. Record source changes, unsupported APIs, startup footprint, and adapter behavior across supported Node/OS versions. |
| V-ADD-18 | Extend V-RUN-07/09 | Run a test-only native probe through the same production launcher and OS policy, independently of Node permission checks. Probe files, sockets, process signaling, devices, and inherited descriptors. A deliberately weakened policy must fail the test. Classify OS termination in the supervisor when the process cannot return a JavaScript error. Never expose this probe mode to installed Apps. |

Build deterministic recovery and state-machine tests before broad UI drills. Use fake clocks and fault injection for event and migration boundaries, protocol contract tests for identity and versioning, and production-build containment drills for operating-system behavior. Visual screenshots cannot establish transaction durability or sandbox enforcement; remote DOM audits alone cannot establish terminal accessibility.

## Recommended implementation order

1. Resolve the state, capability, generation, and event/credential contracts, using existing kernel protocol and persistence modules.
2. Implement the shipped macOS/Linux App worker, resource policy, sandbox-enabled App Tab path, and Fetch broker as part of production work, with focused release tests.
3. Complete one vertical Todo flow through package installation, UI, MCP, timer/outbox, update, and crash recovery. Use a generic API fixture alongside it so Todo cannot hide network compatibility issues.
4. Migrate Slack through the existing AEGS/AEDS boundaries. Use Documents to exercise private files, hostile content preview, import/export, and concurrent edits.
5. Finish the developer SDK and command experience, then deliver Library/distribution and native viewers on the same contracts.

This sequence tests Chariox's implementation of an established sandbox architecture. It does not reopen the requirement that supported third-party Apps must run securely.
