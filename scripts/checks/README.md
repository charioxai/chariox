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

## Allow lists

`scripts/checks/<name>.allow.json` holds today's violations as `{path, count, why}`. The lists use the shared ratchet in `scripts/lib/ratchet.mjs`, the same one the OSS boundary guard uses:

- a file may keep at most its listed count; a new violation anywhere else is red;
- an entry that is not in the list at the merge base is red (CI sets `CHARIOX_BOUNDARY_BASE`; CI without a base fails);
- a count that fell, or a file with no violation left, is red until `--shrink` lowers or deletes it.

A check with an empty list holds the rule everywhere. `protocol-shape-needs-version` has no list: its ratchet is the diff against the merge base.

## Adding a check

Add `<name>.mjs` exporting `{name, why, rule, rootFix, scan(ctx)}` (`scan` returns `{file, line, what}`; `ctx` offers `files`, `read`, `size`, `grep`, `diff`), run `node scripts/checks/run.mjs --init --only <name>` to write its list from today's tree, and add `<name>.test.mjs` with a planted violation using `scripts/lib/check-fixture.mjs`. Add its row above.

## Limits

- `protocol-shape-needs-version` reads field, variant and `#[serde(` lines of `apps/kernel/src/local/api/types*` and `transport/relay_peer*`. A moved field nets to nothing. It requires the snapshot update for the local protocol only.
- `tracked-state` checks file kinds and sizes, not contents: a secret in a tracked source file is not found (test canaries make a token-prefix scan too noisy today).
- `retired-names` counts lines per file; a retired name in a file name counts once more.
