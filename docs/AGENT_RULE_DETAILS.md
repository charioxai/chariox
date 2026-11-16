# Agent rule details

The short rules are in `AGENTS.md`. This file keeps the full wording of the three that need it.

## Development stage

Chariox is in development: backward compatibility with older kernels, clients and stored data is not a goal. When it simplifies the code, discontinue old kernels (re-provision or re-pair them) instead of carrying compatibility shims, legacy read paths, version-gated fallbacks or a long managed-upgrade predecessor list. Keep only what the kernels running today need: the managed-upgrade predecessor list holds the current release and the one before it, and nobody has to prove that an older protocol number never shipped before dropping it. Do not report the missing upgrade or rollback path from an older release as a finding. Delete the old path you replace in the same change. This holds until the owner says Chariox has users to protect.

## OSS boundary (rule 7)

7. OSS boundary (OF-18+, `scripts/oss-boundary-guard.mjs`, part of `pnpm lint`): the change adds no compute-provider or Cloud-only business logic to OSS (provider names, APIs, images, sizes, regions, prices, rental or auto-stop policy, Cloud account UI, hard-coded vendor hosts). Any new control-plane API path or Cloud-shaped request is in `scripts/control-plane-paths.allow.json` or `scripts/control-plane-requests.allow.json` with a justification and a generic seam, and the leak list `scripts/oss-boundary-leaks.allow.json` and the whole-file list `scripts/oss-boundary-files.allow.json` only shrink (the guard compares both lists with their merge-base version, `CHARIOX_BOUNDARY_BASE` overrides the base, and in CI it fails when no base is available): delete an entry in the change that removes its code. Reviewers reject a change that adds a leak-list entry instead of moving the logic to Cloud. `scripts/oss-boundary-guard.test.mjs` plants a Cloud-only request variant, provider words and an unlisted path in a throwaway commit, requires the guard to fail on each, and requires it to pass once the commit is gone.

## Secrets and agents

UX comes first. Agents are benign and act for the user. Chariox gives them tools to finish secret workflows, such as Vault fill by handle and the secret-capture helper, and does not gatekeep them.

- Guard one thing: the product must not hand a Vault value to the model on its own. Keep the Vault-filled field masked in what the model sees (frames, accessibility and DOM text), and never echo Vault values in tool results, history, logs or traces.
- Don't guard secrets that are visible on screens, terminals or pages, anything an agent does with a value itself (copy, paste, move, re-display), or what a person types.
- A guard that blocks or slows an agent or the user's view without protecting what the model sees is a bug.
