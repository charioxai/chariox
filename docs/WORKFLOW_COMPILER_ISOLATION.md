# Workflow compiler isolation

The home kernel compiles workflow source in a separate process and exchanges only serialized input and output. Workflow source gets the declarative builder and serialized approved schema data. It gets no host process, files or commands. The kernel selects Node from its operator configuration; caller `node_path` values are ignored.

Linux uses `/usr/bin/bwrap`. macOS uses Apple's `/usr/bin/sandbox-exec` with a generated Seatbelt profile, the mechanism used by the [official Codex CLI](https://github.com/openai/codex/blob/main/codex-rs/sandboxing/src/seatbelt.rs). Windows and other unsupported hosts remain fail-closed. A missing or unusable isolation backend never starts an unrestricted compiler. On macOS, restore `/usr/bin/sandbox-exec` availability or compile on a supported Linux host with Bubblewrap.

The macOS profile denies operations by default. It permits execution of the selected Node binary, but denies process creation, network access and host IPC. Reads cover only the Node binary, its linked runtime libraries, their intermediate symlink names, platform random devices and `/dev/null`. The loader also needs ancestor metadata and access to the root directory itself; this does not grant reads of its children. Writes cover only an atomically created, mode-0700 empty scratch directory. The environment is cleared, inherited non-stdio handles close on exec, OpenSSL operator configuration is replaced by `/dev/null`, and the scratch directory is removed when compilation finishes.

Node receives a V8 heap limit. macOS additionally samples the isolated process's resident memory and physical footprint every five milliseconds and kills and reaps it when either exceeds `script_memory_bytes`. Failure to measure a live child fails closed. Sampling can allow a transient overshoot between measurements. CPU and scratch-file size limits are inherited OS resource limits, and a kernel deadline bounds all schema replays and serialized I/O. Input and output pipes are drained concurrently, and output capture is bounded. Linux retains its address-space limit for native allocations.

Schema imports stay in the kernel. Linux opens beneath a held directory with `openat2`; macOS walks beneath a held directory with `openat`, `O_NOFOLLOW` and held directory descriptors for every component. Both reject symlinks and non-regular files, enforce aggregate schema bytes and send only requested schema data to the compiler. The scratch directory has no workspace or schema-root access.

The kernel permits 32 schema-resolution rounds followed by a final compilation. Another unresolved request fails closed, and all rounds share one total deadline.

Run the focused suites on a real macOS host with Node installed:

```sh
RUST_MIN_STACK=16777216 RUST_TEST_THREADS=1 cargo test -p chariox-kernel workflow_code
RUST_MIN_STACK=16777216 RUST_TEST_THREADS=1 cargo test -p chariox-kernel compiler_isolation
RUST_MIN_STACK=16777216 RUST_TEST_THREADS=1 cargo test -p chariox-kernel macos_compiler
```

`apps/cli/scripts/live-macos-workflow-compiler-drill.mjs` drives the built kernel and a rendered TUI through its automation socket. It validates and runs an ordinary workflow, validates 32 schema imports, and checks refusal of a room agent's compile request through the official Codex CLI. Pass absolute `--kernel`, `--home`, `--workspace-parent` and `--evidence` paths. The home must be below `~/.chariox/dev/`; source fixtures must be outside that directory and the repository. The drill reports verdicts without guard source or raw provider responses. It removes its fixture workspace and stops its kernel and TUI, while retaining the kernel home because it can contain identity assets.
