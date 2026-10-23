# Lanes through Chariox

MP-08 / MP-10 / MP-11. This is the migration inventory and runbook for replacing
the coordinator's SSH access to b3. It does not close ordinary/managed parity or
the security-anchor review. The scope is the external coordinator, not agents
inside a Room observing other Rooms.

Source baseline for dogfood r1: OSS `extagent/v2` at
`1144cfc5a1345666b64d17949f6633992550aaba`, Cloud `extagent/v2-cloud` at
`92c35e1e69c0c9bf2e76b4a429b411a79d9f6df8`, and the
[external agents plan](EXTERNAL_AGENTS_V2_PLAN.md) at
`159cc920af891dfeb2f4e6aeb63456d36b609d70`. Section 14 is a design, not an
implemented CLI surface at this baseline. The existing `chariox watch` is a
full session stream, not that extension.

## Miguel's single attendance batch

Prepare builds, backups, service definitions and rollback instructions before
booking this batch. Do not start a command that waits overnight for a person.

1. Confirm b3's intended account/realm and service OS user. Complete product
   Setup/device login and any required provider-account transfer through
   Chariox. No owner private keys or copied provider credentials go to b3.
2. With Miguel present, upgrade and restart his Mac kernel. Its Vault relocks;
   he unlocks it in the product. A restart also ends every external grant,
   including an until-exit grant. Confirm normal Mac sessions resume before
   proceeding.
3. Start the real outside coordinator, request
   `chariox access request --until-exit`, and have Miguel approve it with his
   Mac passkey. Verify the displayed holder and term. The coordinator never
   receives the passkey or a relay credential.
4. Confirm the coordinator's `chariox kernels` identifies the intended b3
   kernel, then approve any actual remote `/sudo` task on b3 in its real
   client. An ordinary gateway grant is not a remote sudo approval. Batch any
   required owner-side enrollment/unlock actions here, without granting
   speculative elevation for future work.

EA2-16 remains blocked on this real Mac/b3 flow and Miguel's attendance. A
two-kernel rehearsal on b3 is EA2-16R, not a substitute.

## Coordinator gap table

Sizes below are engineering estimates in engineer-days, excluding owner wait,
review and live acceptance. “Existing” means present in the frozen source;
the rehearsal section separately records what was exercised. “Extension”
means section 14 must ship. Coordinator glue stays on the Mac unless a kernel
must own lifecycle correctness.

| Coordinator function / current tool | Chariox verb and underlying request | Missing work; owner; estimate | Agent-model piece (`4bea22d44`) |
| --- | --- | --- | --- |
| Launch a lane with model, effort, account, hard stop (`brief.sh`, `launch-k.sh`, lane `run.sh`) | Existing: `request --kernel B` with `CreateSession`, `AttachToSession`, `SpawnAgent { provider, account_profile, model, effort, permission_level, worktree_id }`, then `SubmitPrompt`. Accounts are product-linked and verified with the documented product refresh beforehand; check `auth_state: authenticated`. Linking alone did not admit a prompt in the rehearsal. Never select an account by copying an OAuth file. | Thin `sessions`/`agent` verbs mentioned in the plan are absent; generic requests work. Coordinator: request/response adapter, 0.5–1 d. No per-lane wall-clock deadline is exposed on `SpawnAgent`/`SubmitPrompt`: a shell `timeout` around submission only stops that CLI, not its kernel-owned provider run. Kernel: durable deadline/cancel with restart semantics, 2–3 d, or explicitly retain a coordinator watchdog (not equivalent when it is down). | § “regular-agent surface and enforcement” / A01 admits ordinary delegates; A10 supplies leased workers. A04 hour-scale sudo is separate from a task hard stop. `/goal` check-in and wait deadlines are not an unconditional lane wall-clock kill. |
| Deliver initial brief and mid-round review (`brief.sh`, `deliver.sh`) | Existing: `files put` → `WriteKernelFile` for a large brief; `SubmitPrompt` for its instruction, and `SteerActivePrompt` with the expected active prompt id for a running lane. `SubmitPrompt` supplies a continuation after idle. | Coordinator must compose common rules plus current brief, retain review-to-head mapping and avoid steering a different prompt after a race. No need to recreate `run.sh` or edit a launcher's overwritten `prompt.md`. Coordinator: 0.5–1 d. | A01 Room tools and `chariox.send_agent_message` supply correlated delegation/messaging; ordinary no-reply delivery avoids acknowledgement chatter. Reuse rather than inventing a second inbox. |
| Notice finish, silence or an ask (`watch.sh`, `lanes.sh`) | Existing: `watch --kernel B --session S --attachment A` → `Subscribe`; `GetSessionState` can inspect a lane. Extension: `events subscribe`, `wait`, `ack` → `SubscribeEvents`, `WaitEvents`, `AckEvents`, with session/agent targets. | Existing watch needs a held connection per session and returns token traffic; it has no durable merged inbox or finish-returning command. Section 14 replaces completion/interaction polling. EA2 gateway/CLI (extagent r20) + agent-model owners: 2–4 d for the outside adapter once the shared core is reviewed (not a second ledger). File-only asks such as `HOST_REQUEST.md` are not automatically events: lanes must emit task/interaction state or coordinator glue must bridge them. Silence still needs a progress/health deadline; working is not proof of progress. Coordinator: 0.5–1 d. | A02 durable wake service + “no silent dormancy”, and A03 generic process/timer watchers supply task completion, blocked and source-loss occurrences. EA2 §14 must still add outside recipients, cross-kernel subscription parts and pulled delivery. |
| Lane health (`lanehealth.sh`, `areacheck.py`) | Existing: `request` with `GetDaemonHealth`, `GetSessionState`, `ListAgents`; `terminal` → `RoomTerminal` for host resource/process checks. | Kernel health is not the coordinator's ownership/review/budget policy. Keep those checks as Mac-side logic; obtain b3 observations through its ops Room. Events extension gives subscription health, not host disk admission or provider progress guarantees. Coordinator: 1–2 d. | A02 task/obligation/wake receipts, lost-source and deadline re-evaluation; A03 supervised process state. Keep host resource and review-ownership policies in coordinator logic. |
| Read `LANE_STATUS.md`, `PUSH_READY.md`, `ACCEPTANCE_UPDATE.json` (`lanefin.sh`) | Existing: `files stat/get` → `StatKernelFile`/`ReadKernelFile`, bounded binary transfer with digest verification. | Coordinator maps lane → kernel/Room/worktree/artifact paths. Fetch and parse locally; don't put huge files into model context. Events can wake the fetch, but do not replace final artifacts. Coordinator: 0.5 d. | A09 history/turn search supplies public answer references; it does not transfer arbitrary artifact files. Keep machine-file requests. |
| Get a git bundle for publication (`publish.sh`) | Existing: create the bundle in an ops Room terminal; `files get` it to Mac; verify/fetch locally. `RoomTerminal` and `ReadKernelFile` do the remote work. | Replace the SSH stdout transport only; preserve exact base/head checks, skip-CI and review policy. Publishing remains an authorized Mac operation, with no GitHub credentials on b3. Coordinator: 0.5–1 d. This rehearsal never pushes or opens a PR. | A03 process/terminal watchers can signal bundle-command exit; binary transfer remains EA2 machine files, not ledger payload. |
| Rent and release a host (`rent-lane-host.sh`) | Existing transport can run approved control tooling in a Room on its credential-owning machine. Product-managed environments already expose `ListManagedEnvironmentCatalog`, `CreateManagedEnvironment`, `RequestManagedEnvironmentLifecycle`. | The current jump-host script also uses SSH delivery, SSH keys, peer allowances and a TTL reaper. Merely running it in a terminal does not migrate those internals. Default isolation drills move to b3 VMs/containers under current policy. For allowed WAN/performance rentals, coordinator/platform must adapt delivery to Chariox and preserve TTL/cleanup receipts, 1–2 d; equivalence with product-managed environment sizes/regions/TTLs needs validation, 2–4 d. Do not give an external agent forbidden credential-bearing enrollment requests. | A03 timers/process watchers can supervise approved control commands; A10 leases execute work after enrollment. Neither creates a rental adapter, provider authority or TTL reaper by itself. |
| Build slots (`cargo-slot.sh`) | Existing: `terminal open/send/wait/read` → `RoomTerminal`, invoking the installed `agents/cargo-slot.sh cargo …` in the lane worktree. | Keep the b3 slot/memory admission mechanism. Do not replace it with independent target directories or a second kernel scheduler. Record exact source and copy/hash measured artifacts while the slot is held. Coordinator wrapper: 0.5 d; long-lived terminal exit recovery must be drilled. | A03 `chariox.events.process` / `chariox.events.terminal` can wake a Room coordinator on real build output/exit; `c5d761c51` wakes leased agents on a match before exit. Keep cargo-slot admission. Outside coordinator consumes those changes only after EA2 §14. |
| Kill exact owned PIDs | Existing: terminal command through `RoomTerminal`; `CancelActivePrompt` stops a kernel-owned turn; `DestroyAgent` removes the lane agent. | Keep a run-owned PID/start-time ledger and reject PID ≤1, missing/non-integer PIDs and unsafe groups before every signal. `DestroyAgent` does not mean arbitrary detached drill children were cleaned. Coordinator cleanup adapter: 0.5–1 d; no machine-wide kill verb is needed. | A03 generic watchers own handles and PID/start identities, cancellation and cleanup; `891be0479` preserves cancellation after same-worker lease replacement. Reuse for processes launched there; existing arbitrary script PIDs are not automatically owned handles. |
| Merge acceptance results (`watch.sh`, acceptance merge tooling, `areacheck.py`) | Existing transport: `files get` the exact update; Mac-side merge/check code remains the authority for the board. | Validate full `area/ROW` ids, `status`, source head, date, evidence and `new: true` before merge; associate evidence with its source identity. Do not make remote lanes edit the central tracker. Events only trigger collection. Coordinator: 0.5–1 d. | A02 completion wakes and A09 exact child-turn history can trigger artifact collection. Tracker validation and merge policy remain coordinator code. |
| Usage tracking (`usage.sh`) | Existing transport: ops terminals can run approved non-secret status tools, or `files get` sanitized usage TSVs; agent/Room state supplies active-lane counts. | No v2 request replaces the script's cross-account reset windows, measured burn estimates and reviewer reserve policy. Keep the accounting on the Mac; add safe status adapters if needed, 1–2 d, coordinator/provider owners. Section 14 is not a quota API. Never read raw auth/profile files to infer usage. | A02/A03 expose active/wait/task/process state, not provider account quota/reset/burn policy. No direct replacement for usage.sh. |

The table maps the copied coordinator tools, not an attempt to execute their
SSH-bearing commands during dogfood. `watch.sh` also checks host requests,
review status, local memory, artifact freshness and unmerged acceptance. Those
policy checks must survive migration even after session state becomes event
driven. `areacheck.py`'s GitHub review lookup remains on the Mac; external
events do not manufacture GitHub review events.

## Two shapes

MP-08 / MP-09 / MP-10 / MP-11. The agent-model comparison is pinned to
`amint/c2` at `4bea22d440d792ba9e866020bcfb8408a6deb7d6`, with
[AGENT_MODEL_OVERHAUL_PLAN.md](AGENT_MODEL_OVERHAUL_PLAN.md) sections named
in the table. Its source includes the ordinary event tools in
`transport/runtime_tools/agent_events.rs`, the process owner in
`runtime/state/agent_process_watch.rs`, and delegation/goal runtimes. “Built”
here means source is present, not that this dogfood round validated its
separate A01–A10 acceptance matrix. No union build is used in EA2-16R.

**A — outside coordinator.** Keep the coordinator in its current official
provider harness on the Mac. Its local until-exit grant authorizes gateway
requests to b3; b3 owns the lane agents, terminals and files. The core can
launch a lane now using generic requests. Reuse agent-model's ledger/producers
for section 14 instead of writing a second event system. The missing outside
recipient, durable pull inbox, merged subscriptions and CLI wait remain EA2
work. Keep Mac policy/accounting and the publication boundary. This is the
shortest first migration because it preserves the current coordinator and
already exposes machine-wide operations through the gateway.

**B — coordinator and lanes are Room agents.** Run a Chariox coordinator in a
Room and delegate ordinary child agents using A01; lease their execution onto
b3 using A10. Home owns task/ordering/completion; worker receipts identify the
run and lease. A02 ledger/wakes, A03 process/timer/terminal watchers, and `/goal`
completion checks can supervise work without an external process polling.
These Room-scoped tools do not grant cross-Room observation. A04/leased sudo
must be authorized on the execution kernel, never inherited by delegates.
This needs the reviewed agent-model + worker-lease path and real A10
acceptance, plus tested file/bundle collection and coordinator policy adapters.
It changes where the coordinator runs and confines its task graph to its
Room; it is not a reason to expose the outside event API to session agents.

The amint r13 source handoff confirms these pieces at `4bea22d44` and identifies
`0f492b692` for worker-home tools over the connected relay, and
`9f77a9b24` / `6cce52c10` for preserving worker failures in settlement/replay.
All five named reuse commits are ancestors of `4bea22d44`. Its reported
acceptance limits still matter: A02-S01–S03 and A01-S01/S02 remain FAIL;
A10-S01 remains FAIL and A10-S02–S05 are NOT RUN. The live partition uncertainty
seam is still being tested by that lane. Do not treat source reuse or this
EA2-only rehearsal as acceptance of those rows.

The shapes combine: start with A, then move a coherent batch of lane work to a
B Room. The outside coordinator follows that Room through §14 and collects
artifacts through EA2 files. Prioritize the shared ledger/settlement seams,
A03 terminal completion, A10 causal receipts and the outside adapter; do not
build competing schedulers or mistake wait/goal deadlines for a hard stop.

## b3 rehearsal (EA2-16R)

Results are filled from the live run, not inferred from source tests. Evidence
root: `/root/.codex/evidence/browser-resume-20260930/dogfood/r1/`.

The corrected rehearsal passed all eight steps on 2026-10-23. The outside
coordinator was official Codex `gpt-6-luna`, effort `low`, on A; the work agent
on B used the same model/effort, present in the received session wire metadata.
A and B had separate product homes and runtime identities under
`/root/.chariox/dev/dogfood-r1/`. Both enrolled through the product device flow
and connected to the owned stack `pr1207-dogfoodr1` over public TLS/WSS.
The real built CLI entry and exact-head kernel/relay binaries were used.
This is a b3-only workflow rehearsal, not the real Mac working day, WAN timing,
multi-hour stability, managed-placement parity or browser-display acceptance.

| EA2-16R step | Result | Elapsed seconds | Receipt / observed result |
| --- | --- | ---: | --- |
| 1. Ops and work Rooms | PASS | 30 | Created `18e1427774db4602` and `18e1427840ea2ee5`, attached work Room, repaired/verified baseline through the ops Room terminal. |
| 2. Spawn and brief | PASS | 10 | Authenticated product profile; agent `agent-38de2cf6b4192ca2ae454a04523cad0c`; initial prompt `prompt-1792787247794` accepted. |
| 3. Active steer | PASS | 4 | `SteerActivePrompt` accepted against that exact active prompt; committed test includes negative-operand assertions. |
| 4. Notice finish | PASS | 34 | Live `chariox watch` saw assistant completion event 355 and completed/idle snapshot 357; one final state check. No repeated state-poll loop in this attempt. |
| 5. Download status | PASS after correction | 71 | `files get` transferred the note. Its first hash was the baseline hash; one continuation corrected it and another transfer verified final HEAD and `STEER-INCORPORATED`. |
| 6. Bundle and local fetch | PASS | 42 | Bundle created through ops terminal, downloaded with digest verification and fetched into A's local repository. Fetched HEAD equals the corrected downloaded status. |
| 7. Build and wait | PASS | 23 | Real TypeScript compiler emitted `dist/sum.cjs`; artifact assertion succeeded. `terminal wait --for finished` returned exit code 0. |
| 8. Stop/cleanup lane | PASS | 73 | `DestroyAgent` removed the implementation agent and prior attempt's orphan; final rosters prove absence. Both opened terminals closed. |

These are driver wall-clock windows, at one-second resolution, including its
reasoning and recovery. Steps 5 and 6 overlapped while correcting the status
artifact; do not sum the rows or interpret them as network/request latency.
The first completed work turn was 18,751 ms according to the received session
snapshot. The fetched test commit is
`55da401162bebfb4854df02a572485374e7e8f73`; it changes only `sum.test.cjs`.
The transferred note reports `npm test` exit 0, two tests passed, and the
committed test covers positive, zero and negative operands.

The driver issued 54 shell-tool commands in the corrected attempt, with zero
SSH commands and no direct B filesystem access found in the transcript audit.
Every B operation crossed `chariox --kernel`; local JSON/receipt handling and
the final git fetch ran on A. No implementation source changes or union build
were necessary. Exact command receipts, initial failures and fixes to driver
usage are preserved, rather than counted as fresh product features.

### What failed, and what remains missing

- **Initial setup failed.** The first attempt linked the shared product Codex
  profile but omitted `RefreshProviderAccountProfile`. The target refused new
  work because its profile status was unverified; it was not a provider 401 or
  an unavailable account. After the documented product verification returned
  `authenticated`, the same source accepted a real turn. The first attempt's
  twelve state reads, failed dependent steps and leftover agent remain in its
  evidence; the corrected driver removed that orphan. This is a setup
  correction, not a claimed before/after source fix.
- **Actionable refusals are hidden.** The external path returned
  `local_transport_error` and “details are available only in the host terminal”
  for the unauthenticated profile. A wrong-Room agent removal likewise hid its
  reason. A coordinator trying to drop SSH needs safe, typed remediation on
  this surface. Preserve credential-value redaction; do not simply expose
  arbitrary provider error text. Owner: EA2/kernel error classification and
  security-anchor review; estimate 1–2 d. Diagnosis here used the own-kernel
  structured host log during setup recovery, outside the driver's authority;
  that diagnostic escape remains a migration gap.
- **The durable external wait is still absent.** The corrected driver kept one
  full-session `watch` process, read its streamed output through its harness,
  and explicitly stopped its verified PID. This worked for one lane but is
  not §14's per-agent interest, durable merged inbox or wait that returns on a
  relevant change. Owner: extagent r20 plus shared agent-model producers,
  prioritized in the gap table. The first attempt's polling does not become a
  passing events-extension drill.
- **Raw schemas invite coordinator mistakes.** The driver initially lost saved
  Room IDs while rebuilding its local recorder, generated alias conflicts,
  supplied null IDs/omitted fields, omitted an attachment command id, targeted
  the wrong Room for cleanup, and used the wrong terminal wait condition.
  Those are observed driver errors, not evidence that valid requests failed.
  Thin typed helpers and a tested coordinator adapter should prevent them.
  Running a Room terminal command is the intended remote shell surface; the
  driver never substituted a direct b3 shell for it.
- **Artifacts need verification.** A model-written status can contain an old
  hash even after a successful commit. Compare it to terminal/git state and
  the locally fetched bundle; retain the correction. Kernel completion state
  alone does not prove a lane's source/result report is correct.
- **Hard stop and real owner migration remain.** This rehearsal did not add a
  durable per-lane wall-clock kill or move the real Mac coordinator. EA2-16
  stays `BLOCKED(coordinator: real Mac working day and Miguel attendance)`.

Evidence index: `REHEARSAL_AUDIT.json`, `ATTEMPT1_CAUSE.json`,
`driver/evidence/` (failed setup), `driver-attempt2/steps.json`,
`driver-attempt2/gaps.json`, its CLI receipts and `watch.jsonl`, downloaded
status and git bundle, owner popup recordings, and `work-observer/` screenshots
and video at DPR2. The owner captures corroborate the rehearsal; this round
makes no screenshot-fidelity or full browser UI acceptance claim. Final cleanup
and resource receipts are in `CLEANUP_RESULT.json` and `resources.jsonl`.

Acceptance update: new row **external-agents-v2/EA2-16R**, PASS with the per-step
results above and both attempts retained; existing **EA2-16** remains blocked.
No MP-08 / MP-09 / MP-10 / MP-11 item is closed by this rehearsal.

The setup/owner harness may enroll disposable kernels and answer their real
passkey popups as earlier EA2 drills do. The outside driver cannot do so. Once
granted, every operation on B must cross A's `chariox` gateway; direct access
to B's files, SSH and raw IPC are failures of that boundary. Local A parsing,
timing and fetching the downloaded bundle are ordinary coordinator work.

## Ordered migration

1. Land the reviewed Room-terminal service, external agents v2 core/CLI and
   matching Cloud changes first. Confirm protocol/capability compatibility at
   both ends and live expiry/revoke/reconnect behavior. Do not allocate a
   protocol number in this runbook.
2. Land the agent event core and section 14 extension before retiring the
   watcher. Finish durable lane deadlines or retain an explicit, tested
   watchdog as an acknowledged operational dependency. Write/test the
   coordinator adapters from the table, including exact-head review delivery,
   acceptance merging, usage reserve, resource admission and cleanup.
3. Bootstrap b3 once using the approved product install/Setup path, as a
   per-user service under the intended OS user. Give it its own product home,
   normal Vault and runtime identities. Enroll it to Miguel's account/realm
   through device login; select product-linked provider profiles. Verify
   service restart/recovery and hosted WSS reachability. Keep production
   state separate from disposable dev kernels; never use a source checkout
   as the production release contract. Later Path-1/BYOM enrollment can
   replace this interim owner-managed enrollment without changing gateway
   requests.
4. Prepare the Mac upgrade and execute the attendance batch above. After
   unlocking and granting, list kernels and store b3's exact kernel id.
   Several kernels may share a machine; ambiguous aliases must fail, never
   silently select a development kernel. Record the expected machine/user,
   protocol and capabilities alongside the id.
5. Create one ops Room on b3, then one work Room per lane in its own worktree.
   Port one short, low-risk docs/test lane first: brief, active steer,
   completion, final files, bundle, build-slot wait, stop and owned-resource
   cleanup. Use real providers and verified models. Keep existing migration
   and security rules in the brief.
6. Move bounded implementation/review-fix lanes next, then longer build/drill
   lanes after events, restart, deadlines and terminal recovery are proved.
   Keep credential setup, release signing and owner-attended Mac work in
   their existing human/product flows. Compare missing/duplicate final
   artifacts, orphaned runs and quota accounting before removing old tools.
7. Retire SSH transport only after the real EA2-16 and event dogfood EA2-34
   pass and all migrated coordinator functions have receipts. Retain an
   operator-only recovery path for a down/unreachable kernel. Break-glass:
   diagnose/restart that exact service using approved access, record why the
   gateway was unavailable, and return to Chariox once recovered. Never
   silently fall back to SSH for a healthy kernel, never transfer the
   recovery key to the agent, and never replay an unknown-outcome mutation
   before inspecting state.

If b3 is healthy but a needed operation is missing, record a gap and keep
that function on the documented old path until its adapter is accepted.
Migration is incremental; a blocked owner attendance batch does not stop
preparation or unrelated lane work.
