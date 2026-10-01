# MP-08/MP-10 synchronized Drill E fixture

The fixture and `runDrillEScenario({ synchronization })` coordinate three real
Room agents through their normal kernel-managed provider harnesses. They change
no kernel protocol, controller timeout, node bound, provider adapter, or input
authority. Provider shell commands contact only the fixture readiness barrier;
Browser tools still use Chariox runtime MCP.

The caller owns an isolated Room and headed slice, the three agents, two fixture
tabs, resource monitoring, provider diagnostics, evidence, and final session and
slice cleanup. Use the fixture's `browserOrigin` for the pages `/page/same` and
`/page/other`. Focus the same tab before the scenario. Warm all three provider
tool catalogs, including the independent worker's click tool. Match source,
client, signed kernel, and image identities explicitly.

Pass these synchronization hooks:

- `beforePhase("reads")`: warm the independent worker’s button reference, then call the fixture method. Set
  `readWorkerKind: "click"` and provide `readWorkerPrompt()`. The readers perform one
  status operation; the third agent waits on the other tab's button. The same page's
  30,000 probe buttons make snapshot work observable after Action admission;
  the production result remains capped at 5,000 nodes. The independent worker receives readiness five seconds after the readers.
- `beforePhase("mutations")`: first discover and warm the held button on each
  tab with real provider `slice_browser_find` and `slice_browser_click` calls.
  Retain their observed opaque `field_id` values through public kernel history.
  Prepare same-tab agents sequentially so warmup does not introduce a queue pair
  into the acceptance history. Then call the fixture method.
- Set `mutationKind: "click"` and provide `mutationPrompt(agentId, tabId)` that
  requests exactly one click on that tab's observed button reference. Do not
  rediscover or activate tabs during the mutation cohort.
- Forward `promptPrefix`, `tick`, and `observed` to the fixture. In the mutation cohort, agent B receives readiness first; agents A and C
  receive it three seconds later.
  Server-sent events push button state to background tabs without timer polling.
  The button stays disabled while the controller's ordinary actionability wait
  runs. Its safety timer begins on the first observed running click. Page
  release follows the kernel takeover acknowledgment, after a running same-tab
  action, another agent's queued action, and the third agent's independent
  action were observed together.
- `settle(phase, { prompts, deadline, attachmentId })`: heartbeat the exact
  attachment and wait for the exact owned prompts to leave the kernel's active
  and queued inventories within the original scenario deadline. This leaves
  the existing five-second cleanup budget for owned cleanup operations.
- `close`: forward to the fixture; always `await fixture.stop()` in the outer
  caller's `finally` block, then settle the owned session and slice.

Retain `fixture.evidence()`, kernel snapshots, paged Action history, native tool
timing, exact commands and hashes, resource samples, and cleanup projections.
The readiness barrier alone does not establish overlap. A safety release,
provider error, missing snapshot, incomplete check, or failed cleanup remains
RED. Only the existing four-check verifier can accept a complete run. Local
evidence does not close MP-08 or MP-10 or prove hosted/fresh-machine parity.

Run focused checks with:

```sh
node --test apps/cli/scripts/lib/drill-e-barrier.test.mjs \
  apps/cli/scripts/lib/drill-e-synchronized-fixture.test.mjs \
  apps/cli/scripts/live-browser-computer-drill-e-scenario.test.mjs \
  apps/cli/scripts/live-browser-computer-drill-e.test.mjs
```

MP-08/MP-10 limitation: readiness aligns shell completion, not native MCP
submission. Read preflight reconciliation is a global controller barrier; an
independent held action can delay later read admission. The fixture must record
RED when the measured cohorts do not overlap. No guaranteed-overlap claim is
supported by the readiness barrier alone.
