# Chariox Apps Phase 1 implementation ledger

Baseline inspected 2026-09-07 in isolated `codex/` worktrees. This ledger covers the full Phase 1 objective; it is not a reduced delivery scope or release certification. The [implementation plan](https://github.com/charioxai/chariox-cloud/blob/codex/chariox-apps-phase1/docs/CHARIOX_APPS_IMPLEMENTATION_PLAN.html) remains authoritative. No implementation tests or live drills were run for this read-only inventory. Concurrent foundation work must add its own evidence below after integration.

- OSS baseline: `ec85a6f97d8797908a69d46015a1038ddf5916a8`.
- Cloud baseline: `c9341afb73ee7c1e47ae3f7d9f0e75b3d58a30b7`.
- Plan SHA-256: `806fab9295a54b8c0a22165508c0b4d05dab9a4a866841a1116007f2f41f85c0`.
- Baseline protocol: `284` in Rust and TypeScript. The plan and review documents were untracked additions at the start of this work; they are specification input, not evidence of runtime implementation.
- Release scope: macOS/Linux kernels; Web, local TUI browser viewer and remote TUI browser viewer. Native terminal integrations, Windows, public Library/distribution and tracked cross-App work remain Phase 2.
- Status notation: **Partial** means a reusable prerequisite exists, but the Phase 1 feature and acceptance evidence are incomplete. **Missing** means the requested App feature has no integrated implementation at this baseline. **Unverified** means the complete required matrix scenario has no inspected Phase 1 result. No row passes merely because a related existing test exists.

## Existing integration seams

| Key | Existing location and actual support | App integration still required |
|---|---|---|
| Protocol | `apps/kernel/src/local/api/types.rs` and `types/`; `packages/kernel-client/src/kernel-types*.ts`, `ipc-*-requests.ts`; kernel `local/api/tests/protocol_shapes/`; `lib_tests/client_protocol_conformance.rs` | Add App requests, responses and events through normal local/relay transport. Bump Rust/TS version together; add serialized shape hashes and focused drills. Update Cloud minimum only where new behavior is needed. Existing conformance checks include source-marker existence and cannot certify new behavior by themselves. |
| Durable state | `apps/kernel/src/durable_state.rs`, `durable_state/workflow_runtime.rs`, `durable_snapshot.rs`, `app/durable_runtime_state.rs` | Existing SQLite event/snapshot writer and atomic workflow runtime transitions can own installations, update journals, inbox/outbox and receipts. App mutable data rollback must not roll back event tombstones or approvals. Do not put new policy into the large coordinator files. |
| Runtime MCP | `runtime/router/runtime_tool_bridge.rs`, `runtime/state/tool_dispatch.rs`, `transport/runtime_tools/`, `runtime/state/tool_dispatch/extension_request_tool.rs`, `extension_registration_tool.rs` | Add bound App tool declarations and IPC dispatch in the single catalog. Existing extension request supports MCP/skill/script kinds, not Apps. Reuse reload readiness and caller authorization; no extra per-App MCP server. |
| Permission/focus | `session/agent_config.rs`, `runtime/state/tool_dispatch/home_extension_authorizer.rs`, `runtime/router/tests/m16_runtime_extension_registration.rs` | Effective YOLO/Ask resolver and extension self-grant exist. Foreground binding intent, App grants/revocation, captured focus/task/turn and stale generation rules do not. Opening a view must not create or activate a workflow. |
| Events | `runtime/state/event_delivery_runtime_state.rs`, `transport/event_delivery_client.rs`, `event_connection.rs`, `session/workflow_publication.rs`, `packages/event-protocol`, `packages/aegs-sdk` | Current AEDS delivery resolves a workflow binding and persists its receipt plus queue transition. Insert durable installation inbox/handler/outbox before the same workflow queue. Preserve intentional fan-out, occurrence scope and external credential ownership. |
| Workflows/assets | `session/service/workflow_defs.rs`, `session/service/workflow_code.rs`, `session/workflow_scheduling.rs`, `runtime/state/workflow_output_tool.rs`, `session/workflow_outputs.rs`, `scheduler/runtime/workflow_validation.rs` | Existing user-owned workflow execution and validated intermediate/final output provide mechanisms. App asset facade, explicit information-set consent, source-scoped outputs and second-agent App-task correlation are missing. Domain validation stays inside the sandboxed App. |
| Human interaction | `session/runtime_interactions.rs`, `runtime/state/runtime_interaction_state.rs`, `runtime_interaction_owned_state.rs`; CLI interaction controllers; Cloud `kernel/runtime-interaction-requests.ts` | Existing interaction choices and provider permissions use in-memory pending resolutions/oneshot senders. Add durable pending critical operations, exact-effect receipts and trusted human identity/step-up. No production passkey path was found in the inspected kernel/CLI/client sources. Ordinary interaction support does not prove this gate. |
| Environment model | `session/room_environment/` (model, stable Tabs, action ledger, viewport, takeover); re-export in `session/mod.rs` | This is currently an isolated domain model: references outside that directory only re-export it. No integrated Room owner, App-tab protocol, installation origin/partition or terminal accessibility path is evidenced. |
| Existing browser/viewer | `apps/kernel/slice-linux-docker/docker/browser-cdp.mjs`, `slice-screen.sh`, `runtime/state/tool_dispatch/slice/`, `runtime/slice_command_executor/display_endpoint.rs`; Cloud `terminal/view-route-coordinator.ts`, `view-route-projection.ts`, `view-display-endpoint-store.ts` | Current headed-slice browser and display endpoint/iframe are reusable transport/UI seams, not the requested managed App Tab. Primary line 227 and fallback line 593 still use `--no-sandbox`; both use `--password-store=basic`. Add renderer isolation, session persistence proof, App origin/network policy, authenticated viewer input and AX projection. |
| Package trust | `packages/aegs-sdk/src/manifest.rs`, `apps/cli/src/workflow-runtime-signature.ts` | Existing Ed25519/RFC 8785 conventions are reusable. `.cxapp` inventory/extraction/installer, publisher trust enrollment and release lifecycle are missing. |
| Commands/Freeform | `apps/cli/src/commands.ts`, `capability-command-handlers.ts`; kernel `runtime/terminal_command_catalog/`, `runtime/interactive_command_dispatcher.rs`; Cloud `terminal/freeform-*`, `terminal/extensions-*`, `kernel/extension-requests.ts` | Existing generic extensions/workflow commands and Freeform are integration points. App commands, installed-App selector, foreground activation and guided Trigger/Deploy do not exist as complete App flows. |
| Production AEGS/AEDS | `docs/AEGS_SDK.md:214`, `docs/EVENT_TRIGGER_PROTOCOL.md`, generic SDK management/action interfaces | Production implementations are intentionally in private `chariox-aeds` and `chariox-aegs-<provider>` repositories. These two worktrees contain the generic SDK/protocol and dummy generator. Inspect those private service sources and deployed route/connection versions for Slack cutover; a local dummy cannot prove live Slack parity. |

All kernel paths in the table are relative to `apps/kernel/src/` unless an explicit prefix is shown; Cloud paths are relative to `apps/web/src/` in the Cloud repository.

## Workstream requirements

| ID | Deliverable | Baseline | Implementation dependency / missing acceptance |
|---|---|---|---|
| P1.01 | Protocol and persistent model | Partial | Durable/protocol seams above. Define every installation, release/generation, binding, consent/output/task, panel-layout, automation/inbox, storage, migration, health and App-set record; expose identical local/remote state. |
| P1.02 | Package and local installer | Missing | Reuse signature conventions; implement exact immutable inventory, bounded safe extraction, developer trust, file associations, staging, activation and uninstall. |
| P1.03 | macOS/Linux supervisor and worker | Missing | Signed worker with pinned embedded Node; Seatbelt and Linux namespace/seccomp/cgroup launcher before App code; hard/monitored quota/resource mechanisms, aggregate admission, bounded IPC/lifecycle and production native-probe evidence. |
| P1.04 | Node SDK, private files and HTTP broker | Missing | Versioned supported Node subset, SDK state transactions/outbox, atomic single-file writes, external grants, quota/snapshot implementation, Fetch streams/abort/SSRF/credential scope. Conformance must measure real SDK adaptations. |
| P1.05 | Single runtime MCP | Partial | Existing catalog/grant plumbing; add namespaced App functions and authenticated operation dispatch, schema/deadline/cancel, equivalent operation policy across UI/MCP/background. |
| P1.06 | Durable events and notification migration | Partial | Existing workflow delivery receipt/queue machinery; add installation inbox/outbox and local events, automation resolution, replay/poison/expiry/backpressure and App acknowledgement. |
| P1.07 | Managed App Tabs and viewers | Partial | Existing slice viewer plus unintegrated Room model; implement same-Tab authority, scoped origins/partitions/bridge, Web/local/remote authenticated viewer, responsive tooling and accessibility. |
| P1.08 | CLI/TUI/local development | Missing | `chariox app create/dev/validate/pack/install/list/open/status/logs/restart/uninstall`, matching `/app` commands, default-browser viewer, watch reload/preserved data. No manual JSON or sandbox setup. |
| P1.09 | Freeform/workflow integration | Partial | Existing Freeform and normal workflow creation; App/Extension selectors, guided Trigger/Deploy with visible one-node workflows and origin metadata; binding creates none. |
| P1.10 | Updates/migration/recovery | Missing | Durable staged generation switch across code/data/capabilities/catalog/views; fence all writers; restricted migration; precommit rollback and postcommit recovery preserving accepted writes; queued events pause/resume. |
| P1.11 | Slack App and old-path cleanup | Partial | Existing external event/AEGS service contract. Build generic packaged Slack App; verify live clean install and upgrade, migrate route/connection/dedupe identities, then remove old execution/configuration paths. |
| P1.12 | Todo App | Missing | Human/agent concurrent CRUD, durable TypeScript scheduling and overdue recovery, one reusable automation, durable occurrence dedupe through update/rollback. No kernel alarm dependency. |
| P1.13 | Documents App | Missing | Private files/folders, Markdown/isolated HTML editor/preview, safe import/export/external grants, versions and concurrent human/agent edits. |
| P1.14 | Chromium sandbox/session persistence | Partial | Current browser launcher and save/restore seams; enable renderer sandbox on primary/fallback/recovery, preserve profile/key material, verify deterministic and live Google sessions, shared viewer AX and viewport/input. |
| P1.15 | Kernel human validation | Partial | Extend RuntimeInteraction; add durable operation/receipt store, trusted confirmation and production step-up for Web/local/remote TUI; enforce single-use exact authorized effect at broker boundary. |
| P1.16 | Foreground bindings/existing authority | Partial | Extend current extension self-grant/effective permission resolver; bind explicit/foreground/self-grant consistently, capture focus at submission, provider readiness, no stale regrant or focus retargeting. |
| P1.17 | Private conversation panel | Missing | Trusted terminal composites kernel history outside App DOM, stream, AX and screenshot APIs. App receives geometry only; one prompt area, no transcript subscription. |
| P1.18 | Consented outputs/agent continuation | Partial | Existing workflow outputs/handoffs only; named information sets with explicit consent, schema/version/source scope, correlated intermediate/final outputs, bounded repair/delivery, App validators and second-agent continuation. |
| P1.19 | User assets/complete App work | Partial | Existing generic workflow/agent owners; add App-requested normal user assets under existing policy, visible automation/dependencies, full same-App acceptance through every released terminal. |

## Phase 1 verification baseline

Every row below is **Unverified**. Existing related tests are useful starting points, but none were run or audited as full App release evidence in this inventory. Preserve exact scenarios and pass criteria from the plan when implementing drills. The matrix includes all 91 non-header rows: 14 terminal, 6 kernel, 32 integration, 36 core and 3 reference-App rows.

### Released-terminal matrix

| Requirement | Required coverage | Evidence status |
|---|---|---|
| Install, inspect, update, restart and uninstall | Web terminal: Required; Local TUI: Required; Remote TUI: Required | Unverified |
| Local developer .cxapp package installation | Web terminal: Required; Local TUI: Required; Remote TUI: Required | Unverified |
| Shared App Tab, reconnect and human takeover | Web terminal: Required; Local TUI: Required; Remote TUI: Required | Unverified |
| Tool discovery and medium-independent operation policy | Web terminal: Required; Local TUI: Required; Remote TUI: Required | Unverified |
| Critical-action validation and trusted step-up page | Web terminal: Required; Local TUI: Required via trusted browser; Remote TUI: Required via trusted browser | Unverified |
| Events, automation health and workflow configuration | Web terminal: Required; Local TUI: Required; Remote TUI: Required | Unverified |
| File picker, import/export, clipboard and links | Web terminal: Required; Local TUI: Required via viewer; Remote TUI: Required via viewer | Unverified |
| Canonical viewport, text input and accessibility | Web terminal: Required; Local TUI: Required in browser viewer; Remote TUI: Required in browser viewer | Unverified |
| Logs, denied operations and crash recovery | Web terminal: Required; Local TUI: Required; Remote TUI: Required | Unverified |
| Foreground/self-grant binding with existing permission modes | Web terminal: Required; Local TUI: Required; Remote TUI: Required | Unverified |
| Private conversation panel and single prompt area | Web terminal: Required; Local TUI: Required; Remote TUI: Required | Unverified |
| Information-set consent, validated outputs and agent continuation | Web terminal: Required; Local TUI: Required; Remote TUI: Required | Unverified |
| Generic user-asset creation and App work experience | Web terminal: Required; Local TUI: Required; Remote TUI: Required | Unverified |
| Slack replacement, Todo and Documents acceptance | Web terminal: Required; Local TUI: Required; Remote TUI: Required | Unverified |

### Kernel and Environment matrix

| Requirement | Required coverage | Evidence status |
|---|---|---|
| Installer provisions sandbox and resource facilities | macOS kernel: Signed/notarized release artifact; Linux kernel: Production installer and delegated resources | Unverified |
| App worker containment and native probe | macOS kernel: Required; Linux kernel: Required | Unverified |
| Private filesystem quota, snapshot and update recovery | macOS kernel: Required; Linux kernel: Required | Unverified |
| Supported Node transports and critical-effect enforcement | macOS kernel: Required; Linux kernel: Required | Unverified |
| Managed Chromium sandbox and profile persistence | macOS kernel: Actual supported Environment topology; Linux kernel: Local and managed container topology | Unverified |
| Offline, sleep/reboot, multiple Rooms/kernels and aggregate pressure | macOS kernel: Required; Linux kernel: Required | Unverified |

### Integration/adversarial matrix

| ID | Required case | Required outcome/evidence | Status |
|---|---|---|---|
| V1-INT-01 | Chromium sandbox and login persistence | Production launch evidence, deterministic state assertions, redacted live service drill | Unverified |
| V1-INT-02 | Independent OS boundary | macOS/Linux artifact and policy digests; stable supervisor termination classification | Unverified |
| V1-INT-03 | Total and aggregate resources | Measured CPU, total memory, disk, queue and latency budgets on named hardware | Unverified |
| V1-INT-04 | Raw files and state transactions | Fault-injection checkpoints and old/new generation data validation | Unverified |
| V1-INT-05 | Storage exhaustion | No cross-installation exhaustion or loss of accepted state | Unverified |
| V1-INT-06 | Event crash windows | One durable enqueue per scoped occurrence; visible terminal outcomes and bounded queues | Unverified |
| V1-INT-07 | Rollback dedupe and generations | Dedupe receipts survive rollback; incompatible generations cannot operate | Unverified |
| V1-INT-08 | Ambiguous service effects | Unknown outcome is visible; no unsafe automatic replay | Unverified |
| V1-INT-09 | Equivalent human and agent operation | Attributed action evidence; no tool/view privilege split | Unverified |
| V1-INT-10 | Human validation lifecycle | One durable RuntimeInteraction and operation state; execution waits for human decision | Unverified |
| V1-INT-11 | Approval parameter and effect binding | Only the exact approved effect can execute; stale/changed/reused authority is denied | Unverified |
| V1-INT-12 | Step-up authentication | Authenticator integration tests plus real human verification drill; test mocks never count as production authentication | Unverified |
| V1-INT-13 | Capability-expanding update | Decline keeps old release; recovery preserves accepted work; no early capability use | Unverified |
| V1-INT-14 | App-origin escape | Browser-controlled origin and network policy holds; host loopback and metadata are not ambient authority | Unverified |
| V1-INT-15 | Concurrent viewer semantics | Same Tab/document/viewport revisions; accessible names, focus and live updates reach terminal | Unverified |
| V1-INT-16 | Transport conformance and SSRF | Measured source adaptations; bounded trusted-side memory and actual connected-address checks | Unverified |
| V1-INT-17 | Slack contract cutover | Live third-party-style App parity before old code removal; final artifact has no privileged Slack fallback | Unverified |
| V1-INT-18 | Schedule correctness | Persisted occurrence revision controls enqueue; overdue work recovers within declared budget | Unverified |
| V1-INT-19 | Rooms and kernels | Update/uninstall/backup isolation matches documented ownership; no credential or handle inheritance | Unverified |
| V1-INT-20 | Package and review independence | Code remains contained regardless of review; exact signed bytes and extraction rules are verified | Unverified |
| V1-INT-21 | Fresh installation and upgrade | No hidden manual sandbox setup, no Cloud requirement for local Apps, actionable version failures | Unverified |
| V1-INT-22 | Runtime lifecycle and throughput | Declared startup/idle/event/input budgets; relay and kernel authority remain responsive | Unverified |
| V1-INT-23 | One binding and existing permissions | One effective binding path; no redundant prompts, auto-regrant loop or invented authority | Unverified |
| V1-INT-24 | Focus and single prompt area | No retargeted accepted work; one terminal prompt area; no App-owned provider conversation | Unverified |
| V1-INT-25 | Private panel isolation | Only trusted terminal renders/reads panel content; App gets layout data without transcript-dependent callbacks | Unverified |
| V1-INT-26 | Panel accessibility and recovery | Human conversation remains usable from kernel history and is absent from App accessibility tree and stream | Unverified |
| V1-INT-27 | Information-set consent | No data request/delivery before explicit consent; no repeated consent for an unchanged accepted set; no agent self-approval | Unverified |
| V1-INT-28 | Correlated intermediate/final outputs | Authorized fields only; bounded repair, ordered idempotent delivery and explicit incomplete state; no transcript scraping | Unverified |
| V1-INT-29 | App-owned semantics | Kernel enforces generic envelopes and policy; domain checks execute only in sandboxed App code | Unverified |
| V1-INT-30 | Resume with a second agent | Same opaque App task relation with distinct attributed turns; no raw conversation exported or automatic focus retargeting | Unverified |
| V1-INT-31 | Create user workflows and agents | Normal user-owned assets and one execution path; dependencies become visible/broken without silently deleting assets | Unverified |
| V1-INT-32 | Complete App work acceptance | All required Phase 1 terminal/kernel combinations pass the same task; cross-App coordination remains Phase 2 | Unverified |

### Package lifecycle matrix

| ID | Required case | Required outcome/evidence | Status |
|---|---|---|---|
| V-PKG-01 | Valid built-in and local developer packages | Contract tests plus local and managed-kernel drill. | Unverified |
| V-PKG-02 | Archive attacks | Adversarial package corpus on macOS and Linux. | Unverified |
| V-PKG-03 | Manifest and protocol mismatch | Parser snapshots and client rendering tests. | Unverified |
| V-PKG-04 | Interrupted installation | Checkpoint fault-injection suite. | Unverified |
| V-PKG-05 | Update success | End-to-end update across local TUI, remote TUI and web. | Unverified |
| V-PKG-06 | Update failure | Fault injection at every update checkpoint. | Unverified |
| V-PKG-07 | Concurrent operations | Four-client concurrency test. | Unverified |
| V-PKG-08 | Uninstall and reinstall | Lifecycle drill with all three reference Apps. | Unverified |

### Worker containment/lifecycle matrix

| ID | Required case | Required outcome/evidence | Status |
|---|---|---|---|
| V-RUN-01 | Infinite loop and CPU saturation | Kernel health, local TUI, remote TUI, web terminal. | Unverified |
| V-RUN-02 | Memory growth | macOS and Linux App worker tests in Phase 1; Windows repeats them in Phase 2. | Unverified |
| V-RUN-03 | Crash loop | CLI, slash command and web status. | Unverified |
| V-RUN-04 | Malformed or oversized IPC | IPC contract and fuzz tests. | Unverified |
| V-RUN-05 | Ignored cancellation or acknowledgement | Tool call, external event, local event and lifecycle callback. | Unverified |
| V-RUN-06 | Kernel restart and machine reboot | Local and managed machine drills. | Unverified |
| V-RUN-07 | Sandbox escape attempts | One malicious package on macOS and Linux Phase 1 release builds. Windows reuses the corpus in Phase 2. | Unverified |
| V-RUN-08 | Log flooding | CLI, TUI and web logs. | Unverified |
| V-RUN-09 | Sandbox active before App code | Every attempt observes the final default-deny OS policy. No unconfined startup window exists. | Unverified |
| V-RUN-10 | Cross-installation isolation | Neither installation reads, writes, signals, impersonates, or exhausts the other's worker, supervisor, or HTTP budget. | Unverified |

### SDK matrix

| ID | Required case | Required outcome/evidence | Status |
|---|---|---|---|
| V-SDK-01 | Files | Documents App passes on case-sensitive and case-insensitive filesystems without a Chariox-specific wrapper for ordinary private I/O. | Unverified |
| V-SDK-02 | Path safety | Zero escape from installation root across macOS and Linux in Phase 1, then Windows in Phase 2. | Unverified |
| V-SDK-03 | Tool catalog | Catalog matches the active generation and configured bindings; execution checks the same operation policy as App view calls. | Unverified |
| V-SDK-04 | Tool authorization | Authenticated human, agent and background identities cannot be forged. Agents may use UI or tools. Critical-action validation is identical across routes, and binding changes affect discovery rather than creating an App view prohibition. | Unverified |
| V-SDK-05 | External event delivery | One workflow enqueue per accepted occurrence, no loss after accepted receipt. | Unverified |
| V-SDK-06 | Local event delivery | Todo occurrence reaches exactly one configured endpoint or a visible terminal state. | Unverified |
| V-SDK-07 | HTTP | Pinned representative API clients pass their declared transport contract. Streaming, abort, multipart, SSE, decompression, redirects, DNS and credential scope tests bound trusted-side resources. WebSocket is explicitly supported or documented as excluded in Phase 1; it is never silently bypassed. | Unverified |
| V-SDK-08 | Lifecycle | Deadlines and ordering match on local and managed kernels. | Unverified |

### UX matrix

| ID | Required case | Required outcome/evidence | Status |
|---|---|---|---|
| V-UX-01 | App Tab and viewer isolation | Managed Chromium security tests, viewer protocol tests, and DevTools capture from the Environment. | Unverified |
| V-UX-02 | Responsive App view | Screenshot set for Slack, Todo and Documents with no clipped primary action. | Unverified |
| V-UX-03 | Accessibility | Automated audit plus manual VoiceOver pass for every App view. | Unverified |
| V-UX-04 | Local TUI commands | Command parity snapshot and interactive drill. | Unverified |
| V-UX-05 | Remote TUI | Fresh remote connection, not a reused client session. | Unverified |
| V-UX-06 | Agent App selector | Freeform and workflow agent screenshots plus runtime catalog assertion. | Unverified |
| V-UX-07 | Freeform trigger | Web right-click and TUI slash-command drill. | Unverified |
| V-UX-08 | Freeform deploy | Hosted and connected-ingress drill. | Unverified |
| V-UX-09 | Broken automation | App view, workflow and TUI all show the same state and recovery action. | Unverified |
| V-UX-10 | One App Tab, many terminals | Every client reports the same Environment and tab_id. Closing or refreshing a viewer neither duplicates nor closes the managed App Tab. | Unverified |

### Reference-App acceptance

| App | Required human / agent / event acceptance | Release blocker | Status |
|---|---|---|---|
| Slack |  Human: Authorize connection, choose notifications, inspect state and disconnect. Agent: Read allowed context and send or update messages through bound runtime MCP tools. Event: AEGS event reaches App, then one workflow endpoint; duplicate delivery stays single. | Any remaining Slack-specific kernel execution path. | Unverified |
| Todo |  Human: Create, edit, complete, schedule, filter and reopen after restart. Agent: List, read, create, update and complete through App tools with concurrent human edits. Event: Standard timer emits through one automation; overdue restart recovery deduplicates durable workflow enqueue even after App-data rollback. | Any kernel alarm dependency or one workflow per Todo. | Unverified |
| Documents |  Human: Create folders, edit Markdown and HTML, preview, import, export and recover versions. Agent: List, read, create and update planning files through App tools and standard private Node file I/O. Event: Optional document event uses the same automation model, but no event is required for basic use. | Any read or write outside installation storage, or unsafe HTML access to the viewer or Chariox UI. | Unverified |

## Release gates

| Gate | Baseline status | Evidence required before completion |
|---|---|---|
| Architecture | Unverified | Integrated signed worker/OS sandbox, bounded IPC, sandboxed managed Chromium and one runtime MCP; all Phase 1 review obligations implemented. |
| Workflow | Unverified | App events use existing workflow trigger queues; bindings never activate agents; each automation has one target. |
| Migration | Unverified | Generic installed Slack App passes parity/cutover and old privileged execution/configuration is removed. |
| Reference | Unverified | Slack, Todo and Documents pass human/agent/event/update/recovery acceptance. |
| Isolation | Unverified | Production macOS/Linux CPU, total memory, IPC, path/network, crash and log drills, including native probes and aggregate reserves. |
| Browser persistence | Unverified | Renderer sandbox remains active through primary/fallback/restore; fixture and real Google login persistence evidence with diagnosis. |
| Human validation | Unverified | Every route uses trusted human approval/step-up and exact single-use receipt enforced at the effect boundary. |
| App work | Unverified | V1-INT-23 through V1-INT-32 pass with actual agent/provider and terminal behavior. |
| Client | Unverified | CLI/TUI parity, same App Tab through Web/local/remote viewers, authenticated input, responsive layout and terminal screen reader evidence. |
| Lifecycle | Unverified | Install/update/migration/rollback/uninstall/reboot/kernel restart checkpoints and accepted data preservation. |
| Storage | Unverified | Actual private Node I/O, external grants, concurrent edits, snapshots and enforced quotas; no cross-installation access. |
| Freeform | Unverified | Binding creates no workflow; Trigger/Deploy each create visible metadata-linked one-node workflow in one guided action. |
| Evidence | Unverified | Complete matrix on exact release revisions/artifacts/digests, SDK/protocol/runtime versions, numeric reference-machine budgets and dated evidence; publish independently shippable Phase 1 only then. |

## Dependency order and remaining release obligations

1. Establish durable installation/generation, binding/operation identity, critical validation, state/outbox and event routing contracts in small kernel modules. Connect shared protocol and bump/hash guardrails. Keep root coordinator files as wiring.
2. Complete package trust/extraction and native worker sandbox/resource provisioning in parallel. Name supported macOS/Linux floors, installed privileges, quota mechanisms, numeric limits and hard versus monitored enforcement. Pin maintained Node/Chromium, assign patch owners and maximum supported age. A policy fixture or stock Node permission flags do not replace a production signed launcher.
3. Connect private storage, bounded broker transports, lifecycle supervision and runtime MCP to an installed Todo App. Prove headless backend calls without opening Chromium, then durable events into normal workflow queues, update/crash recovery, and long-timer/sleep/wake behavior. API/streaming conformance can run with deterministic local fixtures; hostile resource drills need conservative ceilings.
4. Integrate the Room Environment model into runtime ownership and shared protocol. Fix sandboxed Chromium persistence, origins/partitions/egress and same-Tab viewer/input/accessibility. Add Web/local/remote terminal App flows, private panel composition and one prompt area.
5. Complete trusted human validation/step-up, foreground/self-grant bindings, explicit information-set consent, intermediate/final validated outputs and generic asset operations. Prove an App task continues with a second agent while conversation remains private. Share provider harness paths already owned by the kernel.
6. Complete Documents and Slack live parity/upgrade. Inspect private AEDS/Slack repositories and actual credentials/routes before deciding migration changes; do not fabricate a cutover claim from a dummy fixture. Delete obsolete paths only after replacement acceptance.
7. Finish every developer command, file association, Freeform path and release artifact. Run Phase 1 matrices serially where practical, check resource headroom between heavy stages, capture dated evidence outside repositories, publish PRs and address reviewer comments. Unit/contract tests are necessary but cannot stand in for signed native containment, actual screen readers, trusted authenticator or live-service evidence.
8. Audit every workstream, matrix row and gate against the candidate. Record incomplete or unavailable evidence explicitly. Release/build/distribution evidence must use the same candidate revisions that received review; no subset of passing foundation tests completes Phase 1.

## Evidence entries

| Date | Candidate / artifact | Commands or drill | Scope proved | Remaining gap |
|---|---|---|---|---|
| 2026-09-07 | Baseline revisions above | Read-only `rg`, source/document reads and HTML matrix inventory | Locations, current prerequisites, complete Phase 1 requirement enumeration | No App implementation or release verification is claimed. |

## Implementation progress: 2026-09-07 foundations

Code commit `10a15cc0d` adds `packages/app-package`, `packages/app-runtime` and
`packages/app-sdk`. This is partial progress on P1.01/P1.02/P1.03/P1.04/P1.10,
not a passed release gate. No App process is launched and no existing CLI, MCP,
browser or terminal contract is changed by these components.

- Package verifier: canonical USTAR, RFC 8785/Ed25519 envelope, enrolled publisher
  trust input, bounded hostile archive/path checks and offline declaration schemas.
- Worker transport: bounded bidirectional frames, generation/caller checks, strict
  JSON agreement, concurrent read/write with shared terminal failure, deadlines and
  cancellation. SDK broker namespaces are typed forwarding interfaces; their
  kernel handlers and global fetch/streaming adaptation remain unimplemented.
- Installation registry: borrowed kernel SQLite connection, immutable staged
  metadata, approvals, non-reused generations, atomic activation metadata,
  precommit abort and uninstall fencing. Filesystem/sandbox/migration execution is
  not implemented by this metadata store.
- Evidence: 14 package tests, 15 installation tests, 10 Rust transport tests, two
  Rust/Node interoperability tests and 26 Node SDK tests pass. Targeted Clippy
  with warnings denied, SDK strict declaration checking, workspace formatting and
  whitespace checks pass. These checks do not establish release-matrix coverage.
- Execution: one Cargo job, incremental compilation/debug symbols disabled, shared
  scratch build directory outside Git. At the last resource observation the host
  reported 48% memory free and 27 GiB disk available; build output was 314 MiB.
- Local evidence directory: `/Users/miguel/.codex/evidence/chariox-apps-phase1/`.
  This is development evidence, not signed macOS/Linux release artifact evidence.
- Independent component review fixed schema-keyword/literal confusion, bounded
  pattern-property regexes, added an explicit offline resolver, and corrected
  full-duplex and Rust/Node JSON discrepancies before integration.
- Cloud specification PR: https://github.com/charioxai/chariox-cloud/pull/67.
  Reviewer feedback at original head `c9341afb7` was addressed in `661989247`: P1.07
  explicitly covers Web/local/remote TUI viewers; native viewers remain P2.06.

## Implementation progress: 2026-09-08 packaging and durable staging

Commits `bae04a5e8`, `f06acb832`, `e778695f2` and `58473cc87` add the next
prerequisites. Full Phase 1 matrix rows and release gates remain unverified.

- The installation registry now uses ordered barriers on the existing kernel
  durable writer, with owner-filtered reads and atomic initial staging. All 32
  durable-state tests passed locally, including five new App writer tests. The
  guarded build peaked at 3.2 GiB RSS with one Cargo job.
- Release staging preserves the signed archive, uses descriptor-relative private
  filesystem operations, performs exclusive atomic publication, and validates
  exact content before reuse. Reservation accounting includes allocation units;
  the caller still must hold an aggregate reservation, and provisioned storage
  quotas remain outstanding.
- Upload storage has bounded owner-scoped request IDs, chunk retries, durable
  offsets, original expiries and cancellation receipts. Anchored verifier leases
  prevent abort/expiry from releasing storage still in use. Kernel upload request
  routing, package trust enrollment and actual activation remain outstanding.
- The developer package executable supports key generation, manifest generation,
  deterministic packing, verification and explicitly untrusted inspection.
  Public enrollment material never enrolls itself. Protected key files and
  exclusive output publication reject symlink, hardlink and signing-file inclusion
  attacks. Integration with the shipped `chariox app` developer workflow remains
  outstanding.
- The runtime build recipe pins Node 24.20.0, its source digest, four target plans
  and compiler/resource requirements. Ten lightweight build-tool tests pass.
  Outputs are explicitly unsigned. No native Node build, sandbox launcher,
  signing/notarization or App execution is claimed by this recipe.
- Independent PR review fixed the idle-channel timeout (`ec4c0346f`) and three
  staging findings (`58473cc87`): macOS publication before root sealing, durable
  parent sync before acknowledging reuse, and recovery-marker retention through
  payload cleanup. macOS 14 initially failed staging at `bae04a5e8`; that result
  was not waived. [The rerun on `58473cc87`](https://github.com/charioxai/chariox/actions/runs/34164893075)
  passes on Linux and macOS 14: 90 Rust component tests, 26 SDK tests and ten
  build-tool tests. These component checks do not certify the production sandbox.

Additional local logs in the existing evidence directory are
`developer-package-tests.log`, `release-upload-tests.log`,
`runtime-build-tool-tests.log`, `kernel-durable-tests.log` and
`kernel-build-resources.log`. Earlier failed attempts remain identifiable in that
directory. Tests use bounded temporary data outside repositories; no provider
credential payloads or App processes are part of these drills.

Next: finish shared installation/upload commands and trust enrollment, then wire
the confined worker/bootstrap and brokers. Account linking must explicitly handle
previously unlinked local installation ownership; do not alias default local
ownership to arbitrary relay users. Preserve every matrix row until its complete
acceptance scenario has direct evidence. Continue all nineteen workstreams.

## Implementation progress: shared App inspection and review follow-up

The branch was rebased onto main `9f5ec7d6e`; App inspection now uses shared
protocol 288 after main's protocol 287 changes. The inspection implementation is
`23cd29b21`, and upload durability review recovery is `1862938e7`.

- Kernel-owned App list/status/journal reads flow through the normal router with
  bounded admission and authenticated ownership. Shared request builders, the
  terminal command catalog, standalone CLI and TUI slash commands use this same
  contract. Client projections omit approval handles, host paths and internal
  ownership references, and preserve generations as decimal strings.
- These three reads bypass the transport result cache: every replay rechecks its
  caller and reads current durable state. The cache's existing sessionless
  fingerprint does not identify the owner. A real relay-dispatch regression
  exercises identical command IDs across owners and after installation changes.
- PR review identified a failed upload metadata rename/sync boundary. Recovery
  now synchronizes the held directory before accepting reloaded offsets; a failed
  recovery keeps the instance unavailable for acknowledgments and cleanup. Two
  new tests inject the failure after the actual rename and cover both possible
  crash outcomes. All 15 upload tests passed locally.
- Post-rebase validation passed 82 CLI/shared-client tests, full shared-client
  TypeScript compilation and CLI typechecking across 766 files. No dependency
  symlink or repository-local build output was required.
- Before rebasing, the kernel test executable passed 100 tests selected by
  `app_`, all 66 protocol snapshots and eight command-catalog tests. Those are
  earlier-candidate results, not verification of the rebased candidate. Its
  guarded kernel test build was stopped at 3,638,208 KiB process-group RSS while
  the host still reported 45% free memory; no tests ran in that attempt. The
  expanded hosted CI runs the current kernel routing, relay replay, durable
  writer, protocol and catalog checks. Current-head results remain pending.

Evidence: `post-rebase-cli-validation.json`, `post-rebase-cli-shared-tests.log`,
`upload-durability-tests.log`, and `kernel-app-control-resources.log` in the
existing evidence directory. This increment does not install or start App code.
Full release-matrix rows retain their unverified status.

## Implementation progress: shared package upload transport

Protocol 289 integrates bounded package uploads into the existing App control
service and authenticated local/relay router. All four operations derive their
owner from the kernel caller; the upload store derives its private root from the
kernel's durable database path. No terminal-supplied path or expiry is accepted.
Blocking work uses the same eight-permit admission bound as App inspection, and
cancellation retains that permit until the operation actually finishes.

The shared client supplies begin/chunk/status/abort builders. Raw package bytes
are excluded from command payloads and Debug formatting, and encoded chunks are
bounded before copying/decoding. Upload retries bypass the transport cache so
ownership, current durable offsets and original abort/expiry receipts are checked
on every retry. Tests exercise the actual relay dispatcher with the same command
IDs for two owners, forbidden foreign chunks, local observation of relay writes,
current status after a chunk, and an abort followed by a delayed begin replay.

Local verification passed all 17 upload-store tests, 17 focused shared-client
request/command tests and shared-client TypeScript compilation. A guarded
`cargo check -p chariox-kernel --tests` passed in 109 seconds at a peak 2,524,752
KiB process-group RSS. This validates compilation of the new kernel fixtures;
their execution and protocol hashes are delegated to the expanded hosted CI.
The earlier protocol-288 candidate `85de67ec5` passed Linux's component, kernel
routing/relay replay, durable writer, protocol and catalog job; its macOS job was
superseded by the next candidate, so no macOS result is inferred from Linux.

Evidence: `kernel-upload-store-tests.log`, `kernel-upload-client-tests.log`,
`kernel-upload-client-typecheck.log`, `kernel-upload-typecheck.log` and
`kernel-upload-typecheck-resources.log` under the existing evidence directory.
Publisher trust and verified installation/activation remain separate integration
work. Periodic upload collection now uses the kernel maintenance pump and the
same bounded App admission as requests. It recovers existing expired uploads
without creating storage for kernels that have never used Apps. Existing-root
discovery validates the same held parent as creation and completes child/parent
publication before acknowledging recovered data. Faulted instances reopen only
through the exclusive lease and durable recovery boundary.

The updated runtime suite passed all 72 tests locally, including 18 upload tests
and the existing-root publication/ownership regressions. The final guarded kernel
test typecheck also passed (peak 3,369,984 KiB process-group RSS); execution of the
new kernel maintenance fixture is assigned to hosted CI. Evidence:
`worker-upload-runtime-tests.log`, `kernel-migration-maintenance-final-typecheck.log`
and its resource log in the existing evidence directory.

## Implementation progress: native boundary and worker ownership

Protocol-289 candidate `e2652c393` passed the full configured App component jobs
on [Linux and macOS](https://github.com/charioxai/chariox/actions/runs/34167932018),
including kernel upload routing, real relay replay, durable writes and protocol
snapshots. The independent incremental review found no actionable issues.

The native launcher confines itself before loading the runtime library, checks
bounded launch identity and roots, filters descriptors/environment, reports Ready
and waits for the supervisor's Continue. The Rust worker owner retains the
prepared objects and resource-domain lease through process termination and reap;
it bounds startup, IPC ownership and logs, including cancellation and panic
cleanup. Its production preparation factory still requires enrolled runtime
artifacts, platform provisioning and full quota/resource enforcement.

The first [hosted Linux native fixture](https://github.com/charioxai/chariox/actions/runs/34169010393)
passed on `0f89936d9`: all 28 production libc probe checks and both negative
controls passed inside pinned bubblewrap 0.12.0, private namespaces/mounts and
a 1 GiB/no-swap, one-CPU, 64-task cgroup. It verified actual process identity,
capabilities, seccomp and mount flags before Continue. This is evidence for the
native boundary with fixture provisioning, not for Node compatibility, the
production installer or the complete Phase 1 resource gate.

Local macOS native tests passed all seven scenarios, including constructor
ordering, private I/O, denied host access, pthreads and bidirectional FD3 traffic.
Two Rust supervisor tests passed, exercising the actual C launch-record parser
and process lifecycle without building Node. Evidence is recorded in
`native-linux-34169010393/`, `native-launcher-extended-macos.log` and
`worker-supervisor-tests.log` under the existing evidence directory. The pinned
Node compile remains a separate hosted job. Signed macOS/JIT containment and
the full production resource/compatibility scenarios remain unverified.

## Implementation progress: Chromium migration recovery

The production primary, fallback and recovery launch paths now enable Chromium's
sandbox. The existing profile path and password-store selection are preserved.
The image and live overlays include the real sandbox probe and the provisioner
uses the committed seccomp policy. Its policy label versions configuration;
actual browser processes and `chrome://sandbox` supply runtime evidence.

Legacy slice startup captures a fresh durable checkpoint before replacement.
Snapshot stopping and destructive migration/helper operations use checked
immutable container IDs. Saved-state and active-reference events commit together
through the existing writer before volatile publication. A completed migration
can retry old-container cleanup without restoring an old checkpoint over later
user work; post-commit audit errors cannot trigger rollback. Cancellation retains
the existing slice lifecycle exclusion until the blocking operation finishes.

Local deterministic launch/restore/probe checks passed 46 cases with one
GNU-tar-only case skipped on macOS. The kernel test typecheck passed before the
last helper ownership refinement. The refined helper's four actual process tests
then passed through a tiny harness importing the production module: timeout,
output overflow, descendant pipe cleanup and lost wait authority. The production
CI now runs these plus migration and atomic-publication regressions on both
platforms. Evidence: `chromium-sandbox-tests.log`, `bounded-helper-tests.log` and
the kernel typecheck logs already listed above. Real browser save/restore,
full kernel migration and live Google session acceptance remain separate gates.

The trusted JS bootstrap now imports one ESM default or CommonJS registration
function with the frozen App SDK, owns readiness and the inherited channel, and
drains the shutdown response before exit. Its nine ordinary-Node wire/lifecycle
fixtures passed (`bootstrap-wire-tests.log`); they do not establish execution
inside the pinned embedded runtime or native sandbox.

## Implementation progress: verified preparation and current evidence

Publisher enrollment and revocation now use the existing kernel writer and
owner-scoped, revision-bound decisions. Verified packages retain the exact
signing key. Installation staging binds that provenance, and activation checks
the retained enrollment revision in the same transaction as the active pointer.
The older metadata commit API rejects trust-bound stages, closing an alternate
activation route found during review. Re-enrollment cannot revive an older
stage. This does not yet provide terminal enrollment, runtime preparation or
revocation of a running worker.

The eight publisher tests, twelve verified-staging tests and seventeen existing
installation tests passed locally. The kernel library compiled during the new
guarded check; the larger test-target check was stopped at the local memory
ceiling (3,614,096 KiB peak sampled process-group RSS). Its five new kernel writer
tests are assigned to hosted CI, with no local execution claim. Evidence:
`verified-activation-alternate-route-tests.log`, publisher test logs and
`kernel-verified-installation-typecheck.log` plus their resource logs.

The [macOS 14 native job](https://github.com/charioxai/chariox/actions/runs/34170581264)
passed all sixteen bootstrap/native tests on `093ebb30b`, preserving all 23
native probe records. An earlier real macOS 14 failure showed direct executable
mapping of package files remained allowed. The explicit executable-mapping
denial corrected that failure without changing the syscall assertion. The
unsigned read-map-to-mprotect observation still does not prove signed hardened
runtime code integrity. Failed and successful evidence remains in
`native-macos-34169996404/` and `native-macos-34170581264/`.

[Linux and macOS component CI](https://github.com/charioxai/chariox/actions/runs/34169996393)
passed the fourteen kernel Chromium migration cases, atomic checkpoint failure
injection and four actual process-ownership tests on each platform. The real
browser drill exposed verifier lifetime and renderer-discovery assumptions,
corrected with CDP process inventory and explicit observation evidence. The probe retains
kernel PID-nesting, identity and seccomp checks and records when network
isolation relies on Chromium's diagnostic. A real browser persistence pass and
live Google acceptance remain outstanding until their separate drills pass.

Unsigned runtime bundle assembly now checks exact committed bootstrap/SDK
sources, bounded complete inventories and native provenance without rebuilding
Node. Ten deterministic/tampering tests passed (`runtime-bundle-tests.log`).
Historical native artifacts require an explicit evidence-only mode; neither
assembly nor unsigned manifests establish release authenticity, embedded Node
execution or production containment. No full Phase 1 release gate is advanced
by these component results alone.

## Hosted Chromium persistence result: 2026-09-07

[Run 34171378772](https://github.com/charioxai/chariox/actions/runs/34171378772)
passed on commit `12ffd1f5156625ecd82d2c88d1ef31451273abb2`. It ran the
production launcher, sandbox probe and home-restore action with the exact
committed seccomp policy, Chromium `147.0.7727.137-1~deb12u1`, Node `22.23.2`
and Linux `6.17.0-1022-azure` in resource-limited, externally disconnected
containers. The initial, restored and empty browsers exposed 3, 4 and 3
renderers. Every renderer passed direct PID/network namespace-inode separation,
kernel PID nesting, non-root UID, zero effective capabilities, no-new-privileges
and additional-seccomp-filter checks. All restricted namespace-link counts were
zero: this result did not use the documented diagnostic-only network path.

Persistent HttpOnly authentication and visible cookies, localStorage, IndexedDB
and the fixture tab survived a clean browser stop, whole-home archive, removal
of the original container, production restore into a fresh volume and relaunch.
Server-side revocation denied authentication while preserving local data; an
empty profile lost authentication and all three local storage fixtures. Owned
resource cleanup completed successfully. Evidence is retained outside Git in
`chromium-34171378772/{inputs.json,results.json,versions.txt}` under the task's
evidence directory.

This advances the deterministic browser persistence part of P1.14. The result
explicitly records `fullKernelMigrationValidated=false` and
`googleAuthenticationValidated=false`. Full kernel migration with real
containers, live Google acceptance, App-origin isolation and shared-viewer
input/accessibility remain separate release requirements. The complete browser
persistence or Phase 1 release gate is not marked passed by this fixture.
