# Kernel App development (MP-08 / MP-09 / MP-10 / MP-11)

`/app dev ./app` watches source files in the TUI process. It debounces saves,
keeps one push in flight, and pushes the latest saved source after a running
push completes. `/app dev stop` closes the watcher. Generated Rust `target/`,
dependency directories and dotfiles do not trigger pushes.

The source must be available at the requested absolute path **on the kernel**.
A local TUI watches that same directory. For a remote TUI with client-only source,
put the folder in a kernel workspace using the existing `files put` command,
and develop from that kernel workspace. D2 adds no source-transfer service or
kernel watcher. A missing kernel path returns `source_unavailable` with this
remedy; it never builds from the kernel's current directory as a fallback.

The kernel builds, packs with the shared `chariox app dev-pack` implementation,
uploads through the normal package store and enters the normal installer. The
installer owns signing with the kernel dev identity, migration, capability
approval, health checking and generation commit. Registered installations and
other Rooms keep their own copies. A failed build or declined expansion leaves
the active generation usable. Open DEV views use the existing generation reload.

For WASM builds, install L1's pinned SDK tooling as described in
[the SDK build guide](../packages/app-sdk-tools/README.md). The kernel needs
`CHARIOX_APP_SDK_TOOLS` pointing to that tools directory and the same
`CHARIOX_APP_TOOLCHAIN` and `CHARIOX_APP_COMPONENT_CHECK` used by the builder.
JS/TS uses `app.js`/`app.ts`; Rust uses `Cargo.toml`. No manifest-supplied shell
build command runs. Shared build hosts can set `CHARIOX_APP_CARGO_RUNNER` to their
Cargo slot wrapper. Build scratch lives beside the kernel state database and is
removed after each compiler run. Compiler time and diagnostic output are bounded.
The installed kernel must support WASM activation; D2 does not add a second
worker or bypass the normal runtime gate.

`BeginAppDevPush {session_id, request_id, directory}` acknowledges a retained
build. `GetAppDevPush {session_id, request_id}` reports building, a typed build
failure, or the existing `AppInstallOperationStatus`. Repeat a request id to
follow that exact push; use a new id for a new revision. Once installation has
started, progress and approvals are the normal durable installer operation.
Kernel restart can interrupt a build before installation starts; an unknown
request then needs a new push. Local protocol 647 is required. No peer shape
changes in D2.

Agents in a Room where the developer has started a dev push get `app_dev_push`.
Its `directory` is a kernel path; optional `request_id` follows a previous push.
Long builds return progress after a bounded wait, and capability expansions
return the shared approval state. An agent in another Room neither lists nor
invokes the tool. Agent calls and the TUI use the same kernel operation.

These implementation checks do not establish Stage 1 acceptance. S1-02/S1-07
still require real client/provider, DEV Tab reload, timing and platform evidence
on the selected integrated runtime head. MP-09 shutdown policy is unchanged.
