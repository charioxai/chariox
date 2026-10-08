# Model-switch coverage

`apps/cli/scripts/live-model-switch-context-drill.mjs` exercises the existing kernel prompt and profile-update contracts with official provider CLIs. Use `--rich-probes --round-trip` for nine-fact A→B recall and ten-fact B→A recall. Facts include an answer-only release name, a superseded decision, the task and next step, a file created by a tool, and the tool result. Recall uses no tools; scoring checks whether each expected value appears as a whole token in its key’s answer (case-insensitive; extra answer tokens are accepted).

The provider matrix contains all nine ordered source/target combinations of Codex, Claude and OpenCode. Same-provider cases select a second model before returning to the first. Run each case locally, with `--kernel-ref WORKER`, and with `--slice-ref SLICE`. Both placement flags use the shared SpawnAgent builder and the home kernel's existing leased-agent protocol. The drill checks that the worker binding survives both profile changes.

```sh
CHARIOX_HOME="$HOME/.chariox/dev/coverage-home" node apps/cli/scripts/live-model-switch-context-drill.mjs \
  --kernel-url ws://127.0.0.1:45380 --workspace "$HOME/chariox-worktrees/coverage" \
  --from codex:gpt-5.5:low@PROFILE --to codex:gpt-6-luna:low@PROFILE \
  --rich-probes --round-trip
```

Select authenticated profiles with `@PROFILE`. Each selected profile must exist on the **home kernel**: home exports it over the encrypted leased-account path and installs it on the trusted execution worker or slice, both at binding and when the account changes. For managed slices, use the supported provisioner and secure ImportSliceProviderAuth flow. Standard workers need their capabilities installed by the operator; managed slices may transfer home skill packages through their existing provisioning path.

The file probe uses a workspace-relative scratch filename, creates and reads it back through the execution agent, and requires a completed tool result matching the requested shell command and expected tool output. Explicit nonzero exits, failed tools, unrelated commands and assistant-only marker echoes cannot verify the operation. A finally block immediately removes and verifies the file through its creating profile and placement, including on worker and slice filesystems, even if creation/readback fails. Cleanup runs before the Python probe and profile switch; recall asks only for the filename. Failed cleanup makes the drill fail, displays the error, and records the filename for recovery.

Evidence defaults to `~/.codex/evidence/model-switch-context`; `--evidence-root DIR` overrides it. Evidence records whole-token recall scores, tool probes, cleanup, and public worker binding fields, excluding transport credentials. OpenCode model IDs such as `opencode/big-pickle` remain unchanged in requests and are sanitized only in evidence filenames.

Keep a coverage matrix alongside the evidence, recording each case's command, placement, scores and evidence path. If an official provider login, supported model or execution environment is unavailable, record a named BLOCKED prerequisite and its exact next step. Historical passes must identify their evidence round. Do not claim the complete matrix passes until every case has a successful live drill.
