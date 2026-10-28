The workspace drill exercises MP-08 / MP-10 / MP-11 through two disposable kernels, the built `chariox context copy` command, actual source TUI approval, and a hosted WSS relay. It compares linked Git HEAD, index, working files, binary/untracked content, executable state and a supporting directory. It also verifies refusal of an occupied divergent destination and a fresh return snapshot.

Run on the b3 validation host with these environment variables:

```sh
CHARIOX_WORKSPACE_KERNEL=/absolute/path/to/exact/chariox-kernel \
CHARIOX_WORKSPACE_KERNEL_HEAD=<full-kernel-commit> \
CHARIOX_WORKSPACE_TUI=/absolute/path/to/built/chariox \
CHARIOX_WORKSPACE_PROFILE=/absolute/path/to/product-linked/codex-profile \
CHARIOX_WORKSPACE_STATE=/absolute/path/to/unused/disposable-root \
CHARIOX_WORKSPACE_EVIDENCE=/absolute/path/to/evidence \
CHARIOX_WORKSPACE_ORIGIN=https://stack.val.51-255-87-178.sslip.io \
CHARIOX_WORKSPACE_RELAY=wss://stackrelay.val.51-255-87-178.sslip.io \
systemd-run --scope -p MemoryMax=4G node scripts/e2e-stack/movekernel/workspace-transfer.mjs
```

The origin must have normal validation device approval and support the current kernel enrollment contract. The harness links the existing profile through the product API to satisfy terminal admission; it submits no provider turn. Copy selects an empty kernel context and transfers only the selected Project development setup.

The divergence check observes only archive filenames in its own configured transfer store, then occupies the generated destination before import. It never calls private import helpers or constructs admission tickets. The target must refuse publication and preserve the existing inode, Git state and file tree. Runtime processes and disposable state are cleaned up; value-free JSON results and real TUI review captures remain in the evidence directory.
