# Rule checks

`pnpm lint:rules` (part of `pnpm lint`) turns rules that a text scan can decide into checks. A failure is one line: `file:line: what is wrong. Root fix: ... Rule: ...`.

```sh
node scripts/checks/run.mjs [--only name,name] [--root dir]
node scripts/checks/run.mjs --shrink    # lower counts and delete entries whose violations are gone
node --test scripts/checks/*.test.mjs   # each check plants a violation in a throwaway repository
```

| Check | Rule | Root fix |
|---|---|---|
| `no-provider-sdk` | Runtime rule 2: providers run only through their official tools | drive the provider through its CLI, local server or documented hook and delete the SDK dependency |
| `relay-isolation` | Runtime rule 3: the relay only carries encrypted traffic | keep payload parsing, decryption and session logic in the kernel or shared protocol |
| `hosted-wss` | Runtime rule 6: hosted connections use `wss://` behind Caddy | put the endpoint behind Caddy and configure the `wss://` URL (`ws://` is for loopback only) |
| `protocol-shape-needs-version` | Protocol changes: a changed serialized shape needs the version bump and the snapshot | raise the constant, run `pnpm protocol:generate`, update the snapshot under `protocol_shapes/` |
| `retired-names` | Names the owner retired are never written | use the current name; rename in files you touch |
| `slice-image-closure` | A module a slice entrypoint imports must be in the slice image | add the `COPY` line to `apps/kernel/slice-linux-docker/docker/Dockerfile` |
| `tracked-state` | State, logs and recordings never live in a repository | `git rm` it and keep it under `~/.chariox/dev/<task>/` |
| `commit-message-rules` | Commits: no Co-Authored-By lines, no agent or model attribution | reword the commit to a plain summary and force-push the branch with `--force-with-lease` |
| `no-dead-code-allow` | Writing clean code: delete dead code in the same change | delete the unused item or use it; no `#[allow(dead_code)]` or `unused` allowance |
| `no-legacy-read-paths` | Development stage: no shims or legacy read paths | delete the old name, serde alias or fallback and re-provision old kernels |
| `no-empty-catch` | Writing clean code: fail clearly, never silently | handle or log the error, or comment why ignoring it is safe |

## Allow lists

`scripts/checks/<name>.allow.json` holds today's violations as `{path, count, why}`. The lists use the shared ratchet in `scripts/lib/ratchet.mjs`, the same one the OSS boundary guard uses:

- a file may keep at most its listed count; a new violation anywhere else is red;
- an entry that is not in the list at the merge base is red (CI sets `CHARIOX_BOUNDARY_BASE`; CI without a base fails);
- a count that fell, or a file with no violation left, is red until `--shrink` lowers or deletes it.

A check with an empty list holds the rule everywhere. `protocol-shape-needs-version` and `commit-message-rules` have no list: their ratchet is the diff against the merge base.

## Adding a check

Add `<name>.mjs` exporting `{name, why, rule, rootFix, scan(ctx)}` (`scan` returns `{file, line, what}`; `ctx` offers `files`, `read`, `size`, `grep`, `diff`), run `node scripts/checks/run.mjs --init --only <name>` to write its list from today's tree, and add `<name>.test.mjs` with a planted violation using `scripts/lib/check-fixture.mjs`. Add its row above.

## Limits

- `protocol-shape-needs-version` reads public fields, enum variants, the fields inside variants and `#[serde(` lines of `apps/kernel/src/local/api/types*` and `transport/relay_peer*`. A moved field nets to nothing, also between modules of the same protocol. The constant must be higher at the head than at the merge base, not just touched. It requires the snapshot update for the local protocol only.
- `slice-image-closure` counts only the last (runtime) stage of the Dockerfile, with `COPY --from` transfers followed through the stage they read from. A relative import is looked up from where its importer is installed in the image, so a module installed elsewhere does not count.
- `tracked-state` checks file kinds and sizes, not contents: a secret in a tracked source file is not found (test canaries make a token-prefix scan too noisy today).
- `retired-names` counts lines per file; a retired name in a file name counts once more.
- `commit-message-rules` reads the commits between the merge base and HEAD (merge commits skipped) for attribution lines only: `Co-Authored-By:`, a "generated with <tool>" line, a robot "generated" footer and an `anthropic.com` or `openai.com` noreply address. A model or provider name in a subject is not matched, since commits about the Claude provider are normal. The squash message GitHub writes is not a commit on the branch and is not seen.
- `no-dead-code-allow` reads `dead_code` and `unused*` allowances (also inside `cfg_attr`) in production Rust under `apps/`, `packages/` and `adapters/`; test code, comments and strings are hidden first. A clippy allowance is not matched.
- `no-legacy-read-paths` finds serde aliases and Rust or top-level TypeScript declarations whose name starts with `legacy` or `Legacy` (`legacy_` in Rust). A legacy path under another name is not seen. Each list entry says what the alias or declaration still reads.
- `no-empty-catch` finds `catch {}` with nothing in the braces in non-test `.ts` and `.tsx` under `apps/` and `packages/`; a catch holding only a comment passes, and `.mjs` scripts and `.catch(() => {})` are not scanned.
