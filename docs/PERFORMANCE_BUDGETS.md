# Performance budgets

A performance budget is a ceiling on a number we measure: bytes the browser loads before first paint, milliseconds until a reload shows content, tasks that block the main thread, tokens an agent reads before its first turn. Each one is checked by a gate that measures the number on the real path and fails when it is over the ceiling.

## The rule: budgets only tighten

- A change may lower a ceiling or add a budget at any time.
- A change that raises a ceiling, removes a budget, or stops a gate from measuring one, needs a line in a commit message:

  ```
  budget raised: <id>: <why the product needs the extra cost>
  ```

  `<id>` is `<file>.<name>`, for example `web.initial_bundle_brotli_bytes` or `agent_context_budget.claude_tools_listed`. One line covers one budget.
- Review approves the reason or rejects the change. A change that moves a budget the wrong way without an approved line is a functional finding (AGENTS.md, "Commits and pull requests").
- Keep the line in the squash message, so `git log` holds every raise and its reason.
- After a change makes a number smaller, lower its ceiling in the same change. A ceiling far above the measured number guards nothing.

`pnpm lint:rules` enforces the first two points with the check `perf-budget-tighten-only` (`scripts/checks/perf-budget-tighten-only.mjs`): it compares every budget with the merge base (`CHARIOX_BOUNDARY_BASE`, else `merge-base HEAD origin/main`) and fails on a raised or removed ceiling unless a commit since the base carries its `budget raised:` line. In a shallow clone (CI checks a pull request out at depth 1) it first fetches the commits since the base from `origin`, and fails naming the base when it cannot, so a raise is never judged on a cut history. It also fails on a budget file that is not in the format below. A branch built on a landing branch sets `CHARIOX_BOUNDARY_BASE` to that branch (for example `origin/land/l11-oss`): its merge base with `main` is older than the landing's squash, so the landing's own budget changes would count as the branch's.

## Budget files

Budgets live in `perf-budgets/<side>.json`, one file per side of the product. Each entry maps a budget name to:

```json
{ "max": 632000, "unit": "bytes", "gate": "where the number is measured and checked", "measured": "the measurement the ceiling comes from" }
```

- `max`: the ceiling. A gate passes when the measured number is at most `max`.
- `unit`: what `max` counts (`bytes`, `ms`, `tasks`).
- `gate`: the check that measures the number and fails over the ceiling, and where it runs.
- `measured`: the number the ceiling was set from, when, and on what. A ceiling is the measured number plus a small headroom for noise, never a guess.

Cloud gates read the OSS file through `CHARIOX_OSS_ROOT` (else the sibling `../chariox` checkout), so one file holds each budget and the tighten-only check sees every change to it.

## Where the checks run

Each gate runs first where a developer runs it (`pnpm lint`, `pnpm test`, or the drill), and in verifyloop; CI only runs the same commands again.

| Check | Runs in | Fails when |
|---|---|---|
| `perf-budget-tighten-only` | OSS `pnpm lint` (`pnpm lint:rules`) | a ceiling is raised or removed without its `budget raised:` line, or a budget file is not in the format |
| Initial bundle (`web.initial_bundle_*`) | chariox-cloud `apps/web`: `pnpm test` after the build (`scripts/initial-bundle-budget.test.mjs`) | the built page's first-paint files are over the ceiling |
| Reload and Room switch (`web.reload_*`, `web.room_switch_*`) | chariox-cloud `apps/web` UI data plane reload drill (`scripts/reload-stability/run.mjs`) and its landing gate (`gate:reload-stability`, DPR 1 and 2); verifyloop job `reload-stability` | a drill cell or the `switch` check is over the ceiling |
| Agent context (`agent_context_budget.*`) | OSS kernel tests (`prompt_assembly`, `transport/mcp_server/tests/tool_loading.rs`) | the measured size differs from the file |

## Web

`perf-budgets/web.json`. The numbers, how they were measured and the RED/GREEN runs of each gate are in the round notes of webperf-r7; the `measured` field of each budget says what its ceiling comes from.

- `initial_bundle_bytes`, `initial_bundle_brotli_bytes`: everything the browser fetches before the first paint of `/terminal?ui=os`: `index.html`, the scripts and styles it references, and the static import closure of the terminal-os entry the client script loads. Brotli is the size on the wire. A lazy (`import()`) chunk is not counted, which is the way to keep a feature out of the budget.
- `reload_first_content_p95_ms`: in a drill cell with remembered data, the p95 time from the reload until the window shows its content (docs/CLIENT_DATA_PLANE.md 11.4 in chariox-cloud).
- `reload_long_tasks`: main-thread tasks over 50 ms (the `longtask` entries the page reports) during one reload of a drill cell; the gate takes the reload with the most, so the ceiling does not depend on how many reloads a cell runs.
- `room_switch_busy_p95_ms`: the drill's `switch` check opens the two drill Rooms and switches between them from the Sidebar; for each switch it counts the main thread's busy time (Chrome's `TaskDuration`) in the 2 s after the click, which holds the view-cache save that follows a switch. The budget is the p95 over the switches.
- `room_switch_long_tasks`: tasks over 50 ms in those windows, summed over the switches.

## Agents

`apps/kernel/agent_context_budget.json` is the model these files follow, and predates them: a flat map from a name to the measured size of what an agent reads before its first turn (tools listed, their JSON bytes, the server instructions, the runtime notes). The kernel tests compare it for equality, so a change that grows the context also changes the file, and `perf-budget-tighten-only` then asks for its `budget raised:` line. A change that shrinks the context lowers the number in the same change.

## Kernel

`perf-budgets/kernel.json` and this section are owned by the kernelstarve work: kernel budgets (CPU, attach and history latency), their gates and their measurements are written there.
