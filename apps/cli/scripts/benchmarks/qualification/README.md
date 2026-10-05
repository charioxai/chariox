# MP-08 / MP-10 / MP-11 — unscored benchmark qualification

Disjoint b218 preparation on G2 `9334141d420f8a32393f206102c5b8b4a1b0b609`.
No protocol, provider, solver schedule, official evaluator or frozen score changes.
No full/scored/retry campaign, submission or VM boot is authorized by these files.
Functional Browser/Computer/Path-1 merges do not depend on round-1 completion or round-2 rank.

## MP-08 / MP-10 / MP-11 ownership and integration

`r2next` owns `benchmarks/round2`, its shared final-message helper and active
cohorts. This directory is an independent qualification candidate. Integrate
only after coordinator selection of a new runner identity and exact source
review. Do not cherry-pick another lane's runner or overwrite its files implicitly.

- H1/H13: `passive-har.mjs` extends r2next's passive header correlation with
  bounded active/entry/ExtraInfo capacity and a cumulative 64 MiB metadata budget, completed-request retirement, detached
  session failures, missing metadata reporting, immutable snapshots and bounded
  acknowledged final drain. `har-retention.mjs` copies exclusively, syncs and
  verifies the copied hash before scorer admission. Active terminal work or missing
  ExtraInfo fails close. Failed requests require an explicit new-run policy.
  The original round-1 HAR policy stays frozen. Request/response bodies remain
  omitted; this metadata-only policy does **not** qualify full WebArena mutation
  or response-body evaluation. No Accept value is manufactured.
- H2: `final-message.test.mjs` uses the shared helper from r2next's pinned
  `session-history-fragments.ts` at `63ec764151c90d6e1672befd93e19143a2390ac9`.
  G2 does not contain that helper; coordinator integration is a dependency.
  Compile that exact file outside the checkout and supply its module path.
  Unicode offsets count code points, while final comparison preserves UTF-8 bytes.
  Missing identity/coverage, foreign runs and unsettled turns fail; no guessed answer.
- H6: `webmall-freshness.py` reads existing pages before the unchanged synchronous
  evaluator's cached URL gate and latches the first done result, including zero.
  Sampling is lazy so an earlier shop tab cannot inherit another tab's done flag.
  No grade feedback enters a provider. The executable preflight reproduces the
  stale gate, then tests correct/partial/wrong synthetic URL sets on multiple tabs.
  Only its first-party fixture actor navigates/submits; the grader only reads.
- H10: `site-preflight.mjs` admits **public readiness receipts**, requiring all four
  shops' service/JVM/search health, exact indexed counts, post-reset index freshness,
  generation/source binding and timestamps. Recheck with `checkSiteAdmission`
  immediately before a prompt. An adapter must obtain these facts from real,
  approved sites; supplied fixture receipts prove the admission logic only.
  Builder2's real WebMall sites are not probed or declared ready by this lane.
- H12: `AttemptLedger` uses shared `roomProviderToolName` before an explicit
  allowlist. Distinct call and mutation caps reserve every attempted call/unit,
  including over-cap attempts; duplicate receipts never authorize redispatch.
  Conflicting duplicates, forbidden tools, uncharged mutations or secret-like
  finals invalidate the attempt while retaining a sanitized terminal response
  and denominator. Wire live enforcement/audit through the normal kernel/tool
  path in a separately admitted runner. These fixtures alone do not prove live
  budget enforcement or create a new tool authority.

## MP-08 / MP-10 / MP-11 executable preparation

All arguments below are absolute external lane paths. Scripts refuse existing
output files. Set `PYTHONDONTWRITEBYTECODE=1`; state/evidence stays outside Git.
Use a lane venv with **binary-only** pinned packages, never a native-build fallback.
The qualification here used Node 22.22.1, Python 3.14, Playwright 1.55.0,
beautifulsoup4 4.14.2, requests 2.32.5, websockets 15.0.1 and pydantic 2.12.3.
No browser download is needed; pass installed Chrome to the WebMall fixture.

```sh
CHARIOX_FRAGMENT_HELPERS_MODULE=/absolute/fragment-build/session-history-fragments.js \
  node --test apps/cli/scripts/benchmarks/qualification/*.test.mjs
node apps/cli/scripts/benchmarks/qualification/har-preflight.mjs \
  /absolute/lane/test-state /absolute/evidence/new-fixture.har
python apps/cli/scripts/benchmarks/qualification/navigation-preflight.py \
  --source /absolute/lane/upstream/tracing.py \
  --har /absolute/evidence/new-fixture.har --output /absolute/evidence/navigation.json
python apps/cli/scripts/benchmarks/qualification/webmall-freshness-preflight.py \
  --upstream /absolute/lane/upstream --state-root /absolute/lane/test-state \
  --output /absolute/evidence/webmall.json
node apps/cli/scripts/benchmarks/qualification/site-preflight.mjs \
  /absolute/lane/public-readiness.json /absolute/evidence/site-admission.json
python apps/cli/scripts/benchmarks/qualification/guest-preflight.py \
  --upstream /absolute/lane/upstream --output /absolute/evidence/guest-readiness.json
```

`guest-preflight.py` deliberately exits 1/RED while owner-approved host, guest,
track, normal Room binding, assets and human/judge access remain missing. It
reads public manifests/resources only. No QEMU/KVM operation or VM-slice fallback.
The helper prints neither credentials nor guest/account contents.
The Chrome fixture's root-only `--no-sandbox` is isolated credential-free
qualification setup, not a production Chromium launch or sandbox acceptance.

## MP-08 / MP-10 / MP-11 exact official pins and denominators

| Source | Pin | Scope |
| --- | --- | --- |
| [WebMall BrowserGym evaluator](https://github.com/wbsg-uni-mannheim/BrowserGym/blob/1aaef63d737308144f13b32e37ab5dbc3688d070/browsergym/webmall/src/browsergym/webmall/evaluator.py) | `1aaef63d737308144f13b32e37ab5dbc3688d070` | unchanged full module + checkpoints; synthetic unscored H6 only |
| [WebArena-Verified tracing](https://github.com/ServiceNow/webarena-verified/blob/6473f72db5dcefc97b5725b59e734504edc28a21/src/webarena_verified/types/tracing.py) | `6473f72db5dcefc97b5725b59e734504edc28a21` | exact unchanged NetworkEvent class AST, excluding unused trace loaders; no full grading |
| [OSWorld v1 manifests](https://github.com/xlang-ai/OSWorld/tree/b138d348256078fa634fc3b73567a7337c793e6b/evaluation_examples) | `b138d348256078fa634fc3b73567a7337c793e6b` | 369 full; 361 official no-Drive; eight exclusions, never 358–360 by failed-task omission |
| [OSWorld2 v2.1 release](https://github.com/xlang-ai/OSWorld-V2/blob/acdd3493808e716825975b0f0208194bb2faf3c3/benchmark_releases/osworld-v2.1.json) | `acdd3493808e716825975b0f0208194bb2faf3c3` | 108; separate v1/v2, binary/partial, full/offline tracks |

Fetch those exact public files into external `upstream`. Every preflight checks
hardcoded SHA-256 before using official code/manifests. WebMall evaluator hash
`999783fbfcffdc776742251d255b0c2e3cbc7e73cf7660f918c06fa6a6726d52`, checkpoints
`8d36a60d6733d4b52fc1a91177c673d1441047001a3976fb5a9f3261cf1eeef9`; tracing
`5b31038716a9701f0fcc03e7b4a48acb79a54a59f5c84dae370fd9f8358d631d`.
Guest preflight records task/assets/website revisions, task hash manifest and
canonical v2 archive/runtime image pins directly from the verified release.
V1 guest artifact digest is still an owner/coordinator selection, not inferred
from an unpinned provider image name. V2 Task029's documented setup timeout stays
recorded; no task patch, omission or alternative score is introduced here.

## MP-08 / MP-10 / MP-11 canonical Computer guest boundary

An OSWorld Computer environment is the exact canonical desktop, not a Chariox
Debian browser slice. Setup/reset/evaluator remain official and outside model
context. Their guest API and assets are evaluator inputs, never hidden solver
operations. The ordinary kernel owns Room, actor, display/input arbitration,
provider turn, permissions, history and reconnect; normal Computer tools must
reach that same guest display. A direct benchmark pyautogui/provider SDK loop
cannot substitute for the product binding. A multidomain browser outside slices
also cannot substitute for the canonical guest's desktop and app state.

Retain a reviewed binding receipt tying Room/Environment and host Kernel IDs to
canonical guest/provider/image digest, task-release hash, reset generation,
viewport/display, input actor, screenshot artifact and time. Qualification must
prove same-display observation/input, a harmless non-browser GUI edit, human
observe/takeover, provider pause/cancellation, reconnect without duplicate input,
reset fence rejection and exact owned cleanup. No such product binding is
qualified on builder2. Any required serialized contract change needs a
coordinator protocol allocation; this lane changes no protocol.

## MP-08 / MP-10 / MP-11 human/judge access checklist

Before even an authorized canonical guest smoke:

- Owner selects approved KVM host or official canonical cloud provider, v1/v2
  release, task manifest, permitted observation/action/model track and budgets.
  TCG readiness is RED. Check actual accessible KVM and measured host headroom.
- Owner accepts guest/app/data grants and gated v2 assets through documented
  product access; no owner keys, credentials or manual account copies on builder2.
- Coordinator supplies exact reviewed kernel/binary/release/provider versions,
  accepted guest binding and external state/evidence roots. Validate hashes.
- Human can observe the same desktop through ordinary product display, take over
  and answer one shared RuntimeInteraction; evaluator credentials are isolated.
- Official grader/setup access is ready outside solver authority. V1 deterministic
  checks and any judge-dependent task requirements must be inventoried per track.
  Public organizer acceptance/submission and paid human review are separate grants.
- Owner explicitly authorizes the exact unscored smoke cohort; cohort authorization
  does not imply full/scored/retry campaigns or uploads. Retain every admitted
  attempt and its actual first failing seam; never replay uncertain mutations.

## MP-08 / MP-10 / MP-11 smoke cost projection

Illustrative planning only; no new solver schedule or campaign policy is selected.
For three unscored episodes at an assumed 10-minute solver bound plus five-minute
reset/settlement allowance, one canonical guest consumes at most **0.75 guest
hours**, excluding initial download/setup. Use measured reset/latency to replace
these assumptions before admission. V2's pinned archive is **14,891,811,084 bytes**;
forecast 64 GiB temporary disk and 8 GiB guest memory until measured, plus builder
floors of 10 GiB disk and 16 GiB MemAvailable. Do not start a guest here.

If a selected track has 50 steps/episode and averages 2,000 input + 500 output
text tokens/step, the example totals **300,000 input / 75,000 output tokens** plus
up to 150 screenshot observations. Cost is `input_rate*0.3 + output_rate*0.075`
for rates per million tokens, plus image/judge charges, `guest_hour_rate*0.75`,
storage/network and human fees. Rates and subscription usage costs are **unknown**
until the owner selects provider/model/accounts and billing telemetry. Deterministic
local fixture tests use no provider or judge. No zero-dollar inference claim.

## MP-08 / MP-10 / MP-11 proof limits

Source/fail-first fixtures and bounded local qualification establish only their
named seams. Real WebMall service generation/index readiness, full HAR payload
policy, canonical guest Computer binding/KVM/reset/human acceptance, exact-head
independent review and signed ordinary/managed MP-10 comparison remain open.
Frozen round-1 scores are untouched. No MP item, recovered pass or public rank
is established by this directory.
