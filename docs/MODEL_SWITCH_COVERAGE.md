# Model-switch coverage

`apps/cli/scripts/live-model-switch-context-drill.mjs` exercises the existing kernel prompt and profile-update contracts with official provider CLIs. Use `--rich-probes --round-trip` for the nine-fact A→B recall and ten-fact B→A recall. Facts include an answer-only release name, a superseded decision, the task and next step, a file created by a tool, and the tool result. Recall must use no tools; scoring matches the exact value for each key, as in the previous context-switch rounds.

The provider matrix contains all nine ordered source/target combinations of Codex, Claude and OpenCode. Same-provider cases select a second model before returning to the first. Run each case locally, with `--kernel-ref WORKER`, and with `--slice-ref SLICE`. The placement flags use the home kernel's existing SpawnAgent contract; the drill checks that the leased worker identity survives both profile changes. Standard remote workers need their own accounts and capabilities. Slices use the kernel's secure account-import and provisioner paths.

```sh
CHARIOX_HOME="$HOME/.chariox/dev/coverage-home" node apps/cli/scripts/live-model-switch-context-drill.mjs \
  --kernel-url ws://127.0.0.1:45380 --workspace "$HOME/.chariox/dev/coverage-workspace" \
  --from codex:gpt-5.5:low --to codex:gpt-6-luna:low \
  --rich-probes --round-trip --evidence-root /w/evidence/ctxswitch/round5/local
```

Select authenticated account profiles with `@PROFILE` on each selection. Both the initial profile and each new profile must be available on the execution kernel. OpenCode model IDs such as `opencode/big-pickle` are preserved in requests and sanitized only in evidence filenames. Evidence records recall scores and public worker binding fields; transport credentials are excluded.

Round-five evidence is indexed by `/w/evidence/ctxswitch/ROUND5.md` and `round5/coverage-matrix.json`. Two real Codex round trips passed: local and home-to-worker over the relay. The remaining 25 cases have named BLOCKED prerequisites and reproduction commands. Claude needs a refreshed authenticated profile; OpenCode needs an authenticated kernel profile even though its official free-model smoke succeeds. Slice creation is blocked by unavailable Docker bridge networking on the runner. These prerequisites must be resolved before claiming the complete matrix passes.
