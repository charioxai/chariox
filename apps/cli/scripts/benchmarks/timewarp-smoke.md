# MP-08 / MP-10 — WP-12 TimeWarp smoke

This is a ten-episode smoke, never a full campaign or MP acceptance. The runner
derives from benchmini `79cb66f923be85965930fd34dd66bb2476f94d53` and retains
benchwa `1441f93610697ffb0962052bbea69e56f0661cdf`'s ordinary Room/tool audit
contract. It uses a signed f1 release, its matching slice provisioning overlay,
and the existing product-linked acct-686 account through documented RPCs.

MP-08 / MP-10: pin TimeWarp `dc0e0885e5018b7b120e24d2f80e7f57b64046ac`
and environment data `bfed490a8b4b5dae94b126be6105b84445dcbc4b`. There are
231 task IDs across six eras, giving 1,386 episodes. Freeze ten public IDs/eras
in the runner before solving. Seed 42, one repeat, 180 seconds, 60 mutating
Browser actions per episode. Wiki/News cover all six eras; Shop, cross-site,
and residual judge tasks are outside this smoke. A persistent provider thread
and headed Room differ from potential public submission tracks.

MP-08 / MP-10: official `GenericTimeWarpTask.setup`, `.validate`, evaluator
router and normalizers run unchanged against the existing Room Chromium.
Playwright is an external setup/evaluator attachment, never the solver. The
solver receives public goals only and uses Chariox `slice_browser_*` tools.
Final assistant text is translated to the official chat-message interface.
Scoring runs exactly once after settlement, never providing scores or golds
to the solver. Synthetic read-only Wiki/News services reset by process restart
between eras and browser cookies reset before tasks. Unused Shop URL is inert.

MP-08 / MP-10: upstream package metadata declares Apache-2.0, while the README
advertises MIT. Official tasks and environment-data cards declare MIT; WebShop
includes the Princeton NLP MIT notice. Retain declarations/notices externally.
These terms permit local evaluation; do not present the inconsistent declarations
as a single confirmed redistribution license. Tasks 32 and 143 require the
official residual judge (default `gpt-5`), twelve episodes across six eras.
No residual judge runs in this smoke. Approved judge access is a full-run prerequisite.

MP-08 / MP-10: required environment variables are `TIMEWARP_LANE_ROOT`,
`TIMEWARP_EVIDENCE_ROOT`, `TIMEWARP_RELEASE_ROOT`, `TIMEWARP_SLICE_IMAGE`,
`TIMEWARP_UPSTREAM_ROOT`, `TIMEWARP_RUNTIME_SOURCE_ROOT` (clean f1 checkout),
and `TIMEWARP_CLIENT_ROOT` (matching built kernel client).
Place a Flask/mwparserfromhell/python-dotenv Python environment at
`<lane>/venv`; copy the pinned Wiki/News indexes into `<lane>/site-data/<site>/`.
Run `node apps/cli/scripts/benchmarks/timewarp-smoke.mjs`.

MP-08 / MP-10: evidence includes exact release/image/source, per-task actions,
tool identities, official normalized rewards, screenshots, resource samples,
commands, append-only smoke rows and a dated published-results snapshot.
No verified public submission board is known; rank remains null for this subset.
Record a full-set cost/time projection after the smoke and stop for coordinator
handoff. No score fishing, benchmark-specific product changes, or full run.
