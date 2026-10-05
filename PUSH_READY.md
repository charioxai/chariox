# MP-11 publication run isolation

MP-11 assigned branch `mp11/publication-run-isolation` from exact base `74e50b787a5919ee5c3d580c5b088989fd4a1adf`.

MP-11 fail-first commit `a08e6c687`: all six new cross-caller/publication-account isolation regressions fail on unchanged base; standalone test entry contains four existing support tests (10 total, 4 pass, 6 fail). Synthetic signed callers B and A are both admitted to the same publication; B receives A's status/output/thinking/partial traces through status, viewer-result, run-SSE and invocation-SSE. No real credentials/provider/kernel used.

MP-11 fix: one named access predicate verifies publication/endpoint/workflow and the authenticated subject/account/deployment/environment before run projection. List correlation filters before returning a run; status filters summaries and rechecks detailed fetches; SSE rechecks every fetch. Status explicitly authorizes even on publications without ingress hooks. Anonymous local/public runs retain shared anonymous and legacy no-envelope behavior; authenticated records never become public when a verifier is absent. Authenticated callers without matching ownership fail closed. No new owner-sharing flag or serialized kernel protocol shape.

MP-11 final server CI entry `pnpm --filter @chariox/server test`: 99/99, zero failures/skips; includes actual Fastify HTTP routes, both SSE paths, ownership changing on later fetches, subject/account/publication separation, anonymous visibility, status admission with/without ingress. Existing server fixtures all pass. Focused initial GREEN 10/10. Evidence: `/root/.codex/evidence/browser-resume-20260930/mp11fix-publication/REPORT.md`.

MP-11 inbox checked after milestone: new 14:30 #879 protocol-435 drill-consumer review found at 14:36. Switch next to `mp11/credential-access-fixes` to handle it, then final handoff. Original three #868 P2s are fixed separately at 0b122df01, 63059cc67, 6cfb8fdde. Earlier broader Rust/shell coverage follow-up is not claimed complete in this explicit round.

## MP-11 #886 round 1 — canonical publication scope
- Review P2 -> kernel ResolveWorkflow plus endpoint resolution with kernel precedence before comparing differing recorded IDs. Caller/publication checks retained, including canonical and configured envelope endpoint forms.
- Fail-first alias/prefix invocation -> status/result/run-SSE/invocation-SSE: 2 failures (13/15 pass). Full server entry after fix: 101/101, exit 0. No serialized protocol change.
- MP-11 evidence: /root/.codex/evidence/browser-resume-20260930/mp11fix-round3/publication-{red,green}.log. Local regression proof only; no live acceptance claim.
