# MP-08 / MP-10 / MP-11 — evals phase 1 preparation

MP-08 / MP-10: round 3 uses tokens as primary cost metrics and an explicit dated
proxy mapping with unknown band/write bounds. The unchanged official SWE full
baseline scored 44/50 (88%). The coordinator approved official Harbor on the
same builder, exposing the existing product-linked Codex profile through the
standard environment bind mount and registering it through the normal TUI.
Claude/OpenCode still need owner-side leased grants. No source check closes an
MP item. See `docs/EVALS_PHASE1.md` for live result scope.

`inputs.lock.json` freezes the exact 89 Terminal-Bench 2.0 and 50 HAL Verified
Mini tasks plus official harness revisions. Use `PYTHONDONTWRITEBYTECODE=1` and
put state, logs, job output and predictions outside every repository. Run one
task at a time on the reserved builder and maintain its memory/disk floor.

## MP-08 / MP-10 / MP-11 — runtime and profile admission

The Linux runtime bundle contains `bin/chariox-kernel`, `bin/bun`, the compiled
real app entry `apps/cli/dist/index.js`, kernel-client dist, runtime dependencies
and the official provider executable(s) under `bin/`. Its `eval-runtime.json`
contains `source_commit`, `kernel_sha256` and a `files` map from every shipped
relative file path to SHA-256. Only that manifest may be unlisted. Symlinks must
remain inside the bundle. Do not include profiles, credentials, keys, state or
logs. Build and verify the bundle from the selected clean Chariox source; a
protocol number alone does not establish its source identity.

Local placement (`placement=local`, the default): supply a profile path already materialized by documented Chariox commands in
the task environment. The runner registers it through the TUI's normal
`/provider accounts link` command. It never reads/copies provider-account files
or uses a provider SDK. Runtime-generated private identities are created only
in its disposable external Chariox home. Credential-bearing state, if found,
is retained and makes cleanup incomplete rather than being deleted.

Leased placement (`placement=leased`): the coordinator prepares a fresh task
agent on the owner's Mac home kernel and its builder worker grant, bound to the
actual benchmark workspace. Supply `home_kernel_url`, canonical
`home_session_ref`, `home_agent_id` and `worker_kernel_id`; optionally supply a
`home_auth_file` locator for normal product client access. No raw access token
or provider profile is accepted. The same TUI prompt/settlement path attaches
to that exact task agent and checks the home session, worker, provider and model.
It starts no home/worker kernel and changes no provider registration. The
coordinator owns worker/grant teardown. Never use unrelated owner agents.
Claude/OpenCode use this mode; no builder login is requested (coordinator inbox
2026-10-06 12:35 UTC). Both modes set the task agent's permission policy through
the ordinary TUI command, inside the disposable solver environment.

## MP-08 / MP-10 — official harness entry points

Install Harbor from the exact Git revision in the lock and put this directory
on `PYTHONPATH`. The custom agent reference is `harbor_agent:CharioxAgent`, used
with Harbor's `--agent` option. Pass `--ak` values for `runtime_root`, optionally
`runtime_bundle` (a host bundle to upload), `profile_path`, `source_commit`,
`kernel_sha256`, `local_protocol`, `placement` and `provider=codex`; `--model` is an exact
native provider model ID. Use `--path` for the pinned Terminal-Bench checkout,
`--n-concurrent 1` and an external `--jobs-dir`. Pass `relay_binary` for the real encrypted relay TUI path. The first 10 task IDs in the
lock define a reproducible smoke; the full run must retain all 89 IDs.

For HAL Mini, `swe_campaign.py --help` drives the serial smoke/full solver
campaign with external workspace/output roots and explicit runtime/profile/model
inputs. It checks each clean instance base, retains failed tasks in the denominator,
and stops on proven plan exhaustion or incomplete cleanup. Individual task replay
uses `swe_adapter.py --help`. The adapter verifies the parquet
hash and complete 50-task selection, drives a real Chariox TUI turn, and writes
one standard prediction. Gold/test patches never enter the prompt. Evaluate
predictions with `score_swe.py --help`, which validates the pinned official
harness source and calls its unchanged Docker evaluator on the frozen local
JSON dataset. The run label must begin `chariox-evals-`; the scorer adds a fresh
nonce and refuses a container-name collision before the official harness can
remove an existing container. `swe_adapter.py --placement-config` accepts the
same lease references (no credentials). Smoke uses the first 10 pinned IDs; full uses all 50. Never run
an evaluator from a changed or different upstream source.

`turn.py` captures the actual PTY, per-step terminal screenshots, kernel logs
and shared kernel usage report. `accounting_required=true` fails a completed
turn if tokens or a price are unavailable; diagnostic tasks retain known tokens
with null cost. A real base-red/candidate-green run is required for acceptance. A completed task with missing accounting remains unpriced. Preserve
failed attempts, timeouts and quota observations. Do not turn an infrastructure
failure, partial campaign or skipped task into a successful full run.

## MP-08 / MP-10 / MP-11 — preparation checks

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s apps/cli/scripts/benchmarks/evals-phase1 -p 'test_*.py'
python3 apps/cli/scripts/benchmarks/evals-phase1/report.py \
  --campaign /absolute/external/campaign --score-root /absolute/external/scoring \
  --output /absolute/external/report
```

The report validates campaign/scorer provenance and writes actual task CSV,
summary and a cost/accuracy plot. Missing prices produce no cost/accuracy point.
No fixture, imported interface or source check closes an MP item. See
[`docs/EVALS_PHASE1.md`](../../../../../docs/EVALS_PHASE1.md) for exact blockers.

## MP-08 / MP-10 / MP-11 — round 3 serial Terminal-Bench campaign

`terminal_campaign.py --help` selects exactly the frozen first ten tasks or all
89, with fresh official Harbor trials, no score-based retries and no smoke
reuse. It requires the clean pinned task checkout, hash-bound runtime and the
approved existing product profile. Harbor's standard mounts expose the public
runtime read-only and the product-managed profile at its existing path; no
credential is copied. Use a lane-scoped Compose plugin via `DOCKER_CONFIG`.
The kernel and relay must load on the oldest official task OS. The Python hash
reader supports Bullseye's Python 3.9. Runner dependency venvs live in disposable
container scratch, outside the retained logs.

`terminal_report.py --help` validates official task/runtime identities and writes
the CSV, primary token totals, bounded proxy estimates, elapsed time and
cost-versus-accuracy plot. `--compare-summary` can add the existing SWE result.
Incomplete campaigns have no full accuracy score; missing accounting stays
unknown. `proxy-prices-2026-10-07.json` is the editable model mapping. The exact
public API model exists, so `gpt-6.1-sol` maps to itself. Every dollar estimate
is labeled proxy. Unknown context bands span the public bands; cache-write
counts span zero through non-cached input. Cache writes replace the uncached
rate; the three input categories are disjoint, per the official prompt-caching
guide linked by the mapping. Version-1 additive receipts remain archived; final
reports reprice their original counters with version 2. Reasoning is already included in output and is never added twice.

MP-11: `signal_guard.py` wraps real harness child-process signal operations
with explicit system/invalid PID rejection. `harbor_cleanup.py` settles only
exact Compose projects recorded in the fresh owned job directory. A manual
settlement or unknown retained volume makes the campaign fail; unused task
image tags are removed only when absent before this run, pinned to the same
image ID afterward, and used by no container. Shared caches/images are untouched.

MP-08 / MP-10 / MP-11: `terminal_campaign.py --resume` accepts an unchanged
campaign identity only when the last attempt was a clean `quota_exhausted`
admission. It preserves that attempt and all scored official tasks, then checks
the fresh product quota. SWE campaigns archive clean quota attempts automatically
on re-invocation. Each invocation retains its own runner identity. Benchmark
measurements select the submitted agent/prompt; descendants require explicit
agent/prompt bindings. The real TUI session aggregate is validated separately.

MP-08 / MP-10 / MP-11: Harbor initializes writable mount directories to 0777. The
adapter restores the supplied linked directory to 0700 before ordinary product
linking, changing no files below it. Slim images need Git and a terminal font
in addition to the Python screenshot dependencies. Public ELF packaging is
explicitly hash-bound and mounted at the same absolute path; frozen source and
official task/verifier files remain intact. A settled, measured provider failure
continues to official verification and is retained in the denominator.

## MP-08 / MP-10 / MP-11 — admission-only diagnostics

`profile_probe_only=true` stops local admission after normal profile refresh
and quota checks, before task session/agent/prompt creation. Harbor rejects
this diagnostic as a benchmark result and skips verification. It is false by
default. Account-status failures capture only fixed allowlisted class names,
stage and codes in `profile-status-error-classes.jsonl`; opaque messages and
credential payloads are never recorded there. A 401 requires normal product
account repair. Preserve its failed receipt and every already scored task;
the setup-only resume option does not admit later agent-wrapper failures.
