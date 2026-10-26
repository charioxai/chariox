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

## Focused development drill

Run the kernel and built TUI from the same head, with the pinned SDK tooling
and a runtime that supports the template's engine. For the browser leg, use a
normal-user host with the product-pinned Chromium and installed namespace
helper. Keep state and recordings outside the checkout.

1. Copy an SDK template to a kernel workspace. Attach the TUI to a new Room,
   unlock that kernel's Vault, and run `/app dev /absolute/path/to/template`.
   Accept its shared install approval. Record the installation and generation.
2. Run `/app open INSTALLATION`. Save a visible change to `bundle/ui/index.html`.
   Confirm the same Tab shows it on the next generation. Record the file-save
   timestamp and the first visible updated frame; report their difference.
3. Break `app.ts` syntax and save. Confirm the TUI reports the compiler error,
   the generation stays unchanged, and the Tab still serves the previous UI.
   Fix the syntax and save once; confirm activation and reload recover.
4. Add a network capability to `app.json` and save. Decline the shared approval.
   Confirm the active generation and UI stay unchanged. Remove the capability
   and save; confirm the next generation activates without another expansion.
5. Save several UI revisions in a burst, then save while a build is running.
   Confirm the final UI contains the newest revision and no concurrent push is
   started by the watcher. Stop it with `/app dev stop`.
6. In that developer Room, run a real agent turn asking it to call
   `app_dev_push` on the same source. Follow a returned request id to its final
   result. Compare its installation and generation with the TUI's result. In
   another Room, verify the tool is absent; an attempted invocation must fail.
7. Stop the TUI and kernel, and verify their provider and browser children exit.

A controller fixture can additionally assert that committed assets are sent
through `browser.app.reload` to the original target and that broken or declined
pushes send no reload. Report this as controller-dispatch evidence. It does not
establish page rendering, web-storage preservation, browser isolation or visible
save-to-reload latency.
