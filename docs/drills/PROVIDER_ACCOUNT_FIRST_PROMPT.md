# MP-08 / MP-10 / MP-11: unknown account first prompt

`apps-provider-logins/UNKNOWN-FIRST-PROMPT` exercises a newly linked native
account whose saved authentication observation is `unknown`. The first prompt
must run the same official status check as Refresh, persist the observation and
continue through normal account admission. A logged-out account gets the typed
unauthenticated refusal. Known states do not trigger this first-use refresh.
Existing Vault-backed Claude first-use verification retains its own authority.

Supplementary regressions:

```sh
cargo test -p chariox-kernel --lib provider_prompt_runtime::unknown_account
```

Use the lane's cargo slot and disk-backed TMPDIR on shared builders. The fake
official Claude executable covers success, logout, second-prompt reuse and
authenticated/expired/error/not-configured observations without real credentials.

The real drill is `scripts/e2e-stack/unknown-account-first-prompt.mjs`. It uses
the shared context-switch stack harness and its exact-source binary manifests.
Set `CTXVAL_TREE`, `CTXVAL_LANE`, `CTXVAL_STATE_ROOT`, `CTXVAL_EVIDENCE` and
`PLAYWRIGHT_MODULE` as required by that harness. Provide the lane's extracted
slirp4netns/libslirp tools under `CTXVAL_LANE/ns-tools` for shaped hosted runs.

- `UIQA_CODEX_PROFILE`: existing product-linked Codex scope; never raw credentials.
- `UIQA_BUILD_LABEL`: exact build manifest label, default `candidate`.
- `UIQA_RUN_NAME`: unique evidence/state name for this attempt.
- `CTXVAL_ORIGIN`: lane-owned hosted Cloud origin with a matching API contract.
- `UIQA_EXPECT_UNVERIFIED=1`: base run must show the unverified refusal and leave
  the profile unknown. Omit it for the candidate run.

Setup uses public LinkProviderAccountProfile/CreateSession requests; no refresh
is sent. The action under test is the real compiled TUI's ordinary prompt path,
over hosted WSS and the shared shaped network. The candidate must produce two
short real `gpt-6-luna` low-effort replies, become authenticated after the first,
and retain the authentication observation timestamp after the second. The
fixture regression proves the exact status-check count. Preserve snapshots,
rendered terminal captures, binary hashes, model binding, network calibration
and owned-resource cleanup from both runs. This focused drill does not establish
the full account-picker UX matrix or close aggregate MP acceptance.
