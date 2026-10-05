# MP-08 / MP-10 / MP-11 — shared round-2 runner preparation

This is a new runner identity based on main `086d6dc16ac9cc98be62c785947d28004b8ad245`
(local protocol 410). It changes no serialized protocol shape. Frozen round-1
branches, scripts, HARs, results and expected sets remain unchanged. Nothing
here launches automatically or authorizes another scored run. Source fixtures
close no MP acceptance gate and establish no recovered passes or public rank.

Import `index.mjs` for Room lifecycle, the official built kernel client, history
loading, settlement, answer export and passive HAR collection. Benchmark
adapters supply public task instructions, fixture setup, allowed-tool/budget
audit, unchanged official scoring and exact resource/cleanup verification.
Keep expected answers and evaluator feedback outside provider context. Every
benchmark can use this surface without copying the common lifecycle code; no
frozen runner is migrated in this lane.

## MP-08 / MP-10 / MP-11 H1

`PassiveHar.consume()` accepts retained CDP events. `observePassiveHar()` wires
an existing observer's send/subscribe seams to existing target sessions using
only `Network.enable`. The caller owns target discovery/attachment and detach;
it must attach future targets before admitting solver work. No browser, input,
navigation, interception or raw CDP tool is exposed to the provider.

Request ExtraInfo is queued by session/request ID and assigned in redirect-hop
order only after `hasExtraInfo` / `redirectHasExtraInfo` establishes which hops
emit it. Only actual observed headers are merged, case-insensitively; Accept is
never inferred from resource type, MIME type or status. Missing/unmatched
ExtraInfo stays explicit in `_capture`; exporters must fail admission when
their official scorer requires metadata that is missing.

The request allowlist is Accept, Accept-Language, Content-Type and Sec-Fetch
metadata. Response headers allow Content-Type, Content-Length and Location.
Cookies, auth, unknown headers, request bodies and response bodies are omitted;
URLs omit userinfo, fragments and sensitive query parameters. `_requestMetadata`
retains session/request/redirect identity and header provenance. This is a
metadata-only HAR core: adapters needing permitted mutation payloads or response
bodies must retain their separately reviewed sanitized capture contract.
The Node fixtures prove correlation and redaction, not execution of the official
Python navigation classifier or WebArena scoring.

## MP-08 / MP-10 / MP-11 H2 / H7

`openKernelClient()` requires a build containing the new shared
`assembleSessionHistoryEntry` / `assembleSessionHistoryFinalMessage` helpers.
`loadTurnHistory()` retrieves every unique blob under the submitted prompt's
turn, without inserting the summary preview among original entries. The final
summary identifies the message; original entry indices, run/merge identity,
Unicode code-point offsets and coverage reconstruct it once. Complete combined
summaries are used directly. Incomplete, conflicting, missing or empty final
messages fail export. Commentary and reasoning are excluded. A secret-like
answer fails export instead of silently changing the text supplied to a scorer.

`runRound2Episode()` submits once, records prompt/turn identity, lifecycle and
monotonic settlement elapsed time before final export/scoring, assembles and
sanitizes structured provider errors, and keeps failed/cancelled/open settlement
RED with `denominatorIncluded=true`. Missing or incomplete errors have unknown
cause; resource cancellation retains its explicit cause and cleanup receipt.
The grader is never called after failed settlement. Official score zero remains
a scored task failure, not a replacement of the denominator.

## MP-08 / MP-10 / MP-11 H3/H4 — owner policy, 2026-10-03

The `round2-shared-v2` identity implements the policy recorded in the lane's
`COORDINATOR_NOTES.md`: always reject non-essential consent; answer live
questions as of today, declaring the run date; retain the official evaluator
verdict, with no alternative-workflow acceptance or re-grading (H5).

`runRound2Episode()` appends this policy to public task instructions before the
single normal kernel prompt submission. It captures the UTC date once at
admission and retains it in `run.json` alongside exact runtime/runner and
environment provenance in `source`. Live questions use that date; explicit
historical dates in tasks remain authoritative. An adapter can declare
`questionMode: 'fixed'` for a fixed-date task. Historical-gold disagreements and
wrong counts/classifications keep their original official verdict; this library
does not research facts, consult gold, change an evaluator or award credit.

Consent rejection uses observed ordinary Browser UI controls only. When a
reject-all or necessary-only control clearly rejects every optional category,
the adapter classifies its meaning as `reject_nonessential`.
`consentDecision()` permits only that meaning and rejects ambiguous dismissal,
accept-all, authorization and other actions. Adapter `audit()` receives the
same policy for checking retained action evidence. It must enforce the policy
alongside its normal tool/budget audit. There is no automatic label matching,
site-specific click, hidden mutation, CAPTCHA bypass or new permission path.
Unavailable or ambiguous rejection leaves that source blocked. The synthetic
overlay fixture proves this decision rule; it does not prove real site behavior
or that a provider obeys the instructions.

Adapters declare `answerContract` from the **public task and scorer contract**,
before submission, never from gold or evaluator feedback:

| Contract | Solver output | Scorer input |
| --- | --- | --- |
| `{ kind: 'text' }` (default) | Complete final text | Byte-exact string |
| `{ kind: 'name' }` | JSON string with the requested name | Decoded string |
| `{ kind: 'set' }` | JSON array of unique strings | Array in original order |
| `{ kind: 'number' }` | Finite JSON number | Numeric spelling as a string |
| `{ kind: 'records', fields: ['name', 'category'] }` | JSON array of objects with exactly those keys | Original objects/values |

An optional `taskId` requires an exact `{ "id": taskId, "answer": value }`
envelope and passes only its answer value to the adapter's unchanged scorer.
The number envelope returns the numeric value's JSON spelling as a string;
unsafe integers and nonfinite values fail. Unsupported name/number/text
answers use an empty JSON string; sets/records use an empty array. No parser
strips prose or fences, deduplicates sets, changes factual values, repairs
classification or consults an expected answer. Invalid packaging stays RED at
`final_response_export`, included in the denominator, without calling scoring.

`answer.txt` retains the complete sanitized-admitted final message byte-exact;
`answer-package.json` retains the declared contract and packaged scorer value.
Both raw and decoded values must pass secret checks. The audit and grader
callbacks receive `policy`; grader output is retained without reinterpretation.
Synthetic name/set/number/records and full-episode fixtures cover these seams.
They establish no recovered benchmark pass, rank or MP acceptance.

Room creation uses one normal product session and headed slice; host-port
collisions delete/recreate only the owned slice, at most three attempts, and
are journaled. Admission requires runtime/image verification and measured
resource guards. Cleanup uses product RPCs, checks owned session/slice absence,
and requires adapter proof of owned containers/volumes/processes/listeners gone.
It never prunes Docker or deletes another lane's resources. The API caller owns
the kernel connection and closes it after its run cohort. Evidence directories
must be external lane-owned paths, never repository paths.

## MP-08 / MP-10 / MP-11 H6 — external-CDP mechanism proof

The frozen upstream BrowserGym fork is `1aaef63d737308144f13b32e37ab5dbc3688d070`.
Its unchanged [StringEvaluator](https://github.com/wbsg-uni-mannheim/BrowserGym/blob/1aaef63d737308144f13b32e37ab5dbc3688d070/browsergym/webmall/src/browsergym/webmall/evaluator.py)
gates `#submittedResult` retrieval on cached `page.url` before any browser API.
The Node fixture reproduces that ordering with queued external navigation on
multiple synthetic pages, then verifies a read-only `title()` round trip before
validation. `sampleExistingPages()` is preparation for adapters using that API;
it is not wired into the frozen Python grader.

`webmall_sampling_test.py` now proves this seam with real synchronous Playwright,
two existing Chromium tabs, an independent CDP connection, and that unchanged
official StringEvaluator. The historical cached-URL ordering fails all three
synthetic submissions; passive event pumping returns the exact correct, partial,
and wrong-set scores and done states. Submitted text remains unchanged, and the
sampler stops at the first terminal validation. `webmall-grader.py` uses this
passive observation before consulting cached page URLs. No grader source, URL
normalizer, expected set or historical score changes.

Run with a Python environment containing Playwright, beautifulsoup4 and requests,
and its installed Chromium. `WEBMALL_OFFICIAL_SOURCE` points to the pinned fork's
`browsergym/webmall/src`. Set `PYTHONDONTWRITEBYTECODE=1` to keep source-only
checkouts. The standalone test requires no provider or live WebMall deployment.
This proves the synthetic official-evaluator seam, not full WebMallTask routing,
live fixture readiness, recovered benchmark wins or MP acceptance.

MP-08 / MP-10 H10: `webmall_readiness.probe_search` separately requires four
distinct product indexes, current published catalog counts, healthy search,
complete count responses and unchanged index UUIDs across each read. Callers
must collect current shop counts and bind index names from their owned fixture,
then probe immediately before prompt admission. It writes no index and reads no
task answers. Its synthetic tests do not establish a deployed fixture's schema,
WordPress search wiring, JVM compatibility or frozen asset provenance.

## MP-08 / MP-10 / MP-11 focused checks

Use installed TypeScript, with compilation/test scratch outside the repository.
No Rust, provider, Docker, live benchmark or full Node/Web suite is needed.

```sh
node /absolute/node_modules/typescript/bin/tsc \
  packages/kernel-client/src/session-history-fragments.ts \
  --target es2022 --module es2022 --strict --outDir /absolute/lane/fragment-build
CHARIOX_FRAGMENT_HELPERS_MODULE=/absolute/lane/fragment-build/session-history-fragments.js \
CHARIOX_ROUND2_TEST_STATE=/absolute/lane/test-state \
  node --test apps/cli/scripts/benchmarks/round2/*.test.mjs
```

Create `/absolute/lane/test-state` first. Episode fixtures use only fake kernel
RPCs and remove their exact temporary directories in `finally`. Also run the
existing kernel-client fragment tests from an external TypeScript output root.
Tests include fail-first H1/H2/H6/H7 reproduction, Unicode/combined previews,
duplicate blobs, missing/conflicting coverage, redirect/session permutations,
structured errors, cancellation, bounded port retries, cleanup failures, and
shared successful/failed episode wiring, plus H3/H4 consent, live/fixed date,
answer packaging and unchanged official-zero fixtures. Retain commands, exit codes, exact
runner commit, resource samples and cleanup receipts outside the repository.

MP-08 / MP-10 / MP-11: full fixture admission and benchmark remeasurement remain
separate from the H6 mechanism proof and H10 probe preparation. H3/H4 policy is
implemented here; H5 leaves the official verdict and frozen round-1 zero unchanged.
