# MP-08 / MP-10 / MP-11 — evals phase 1 preparation

This is a diagnostic pilot area. Accounting history/CLI integration and real
Codex acceptance await the coordinator protocol allocation and authorized
`acct-686` mapping. No full benchmark score is claimed.

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
`--n-concurrent 1` and an external `--jobs-dir`. The first 10 task IDs in the
lock define a reproducible smoke; the full run must retain all 89 IDs.

For HAL Mini, prepare an isolated solver workspace at each pinned instance's
base commit, then run `swe_adapter.py --help`. The adapter verifies the parquet
hash and complete 50-task selection, drives a real Chariox TUI turn, and writes
one standard prediction. Gold/test patches never enter the prompt. Evaluate
predictions with `score_swe.py --help`, which validates the pinned official
harness source and calls its unchanged Docker evaluator on the frozen local
JSON dataset. The run label must begin `chariox-evals-`; the scorer adds a fresh
nonce and refuses a container-name collision before the official harness can
remove an existing container. `swe_adapter.py --placement-config` accepts the
same lease references (no credentials). Smoke uses the first 10 pinned IDs; full uses all 50. Never run
an evaluator from a changed or different upstream source.

`turn.py` captures the actual PTY and kernel logs. It does not yet collect the
required screenshots or kernel usage report; it cannot certify accounting
acceptance. A completed task with missing accounting remains unpriced. Preserve
failed attempts, timeouts and quota observations. Do not turn an infrastructure
failure, partial campaign or skipped task into a successful full run.

## MP-08 / MP-10 / MP-11 — preparation checks

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s apps/cli/scripts/benchmarks/evals-phase1 -p 'test_*.py'
python3 apps/cli/scripts/benchmarks/evals-phase1/report.py --output /absolute/external/evidence
```

The report currently writes status-only CSV and an explicitly empty-data plot.
No fixture, imported interface or source check closes an MP item. See
[`docs/EVALS_PHASE1.md`](../../../../../docs/EVALS_PHASE1.md) for exact blockers.
