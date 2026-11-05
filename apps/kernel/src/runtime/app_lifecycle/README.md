# Kernel-owned App lifecycle

MP-08 / MP-09 / MP-10 / MP-11. Installed Apps use contract 2 WASM components.
`start.rs` opens `chariox-app-wasm-worker` beside the kernel, retains verified
release bytes and the trust binding, prepares the installation's private data,
and starts the worker on the bounded blocking service. Unsupported stored
releases must be reinstalled; they have no production launch path.

The worker enters Linux seccomp/Landlock or macOS Seatbelt containment before
its ready record and before instantiating any guest. Fuel, epoch deadlines,
linear-memory bounds and the native monitor bound execution. The component
has no ambient network, process, environment or host filesystem authority.
The kernel brokers authorized capabilities through the existing SDK channel.

Updates retain the committed generation and checkpoint private data before
migration. Failed or interrupted updates restore the checkpoint; cancellation,
actual process reap and broker drain precede releasing the binding and leases.
`restore.rs` and `worker_process/wasm/storage.rs` use the same storage ownership.
Deleting an App's data refuses while a worker holds its data lock.

Room App Tabs serve only verified UI assets on an installation-scoped origin.
The view bridge binds calls to the installation, generation and document.
Agent input additionally retains caller identity through asynchronous handlers.

Focused checks: kernel `app_` and `protocol_shapes` library tests, App runtime
library tests, WASM worker engine/containment and installed-worker drills.
Real acceptance additionally requires the built kernel, relay and clients,
install/open, Excalidraw and Univer edit/save/reopen, update recovery and
containment refusal. Unit/fixture results alone do not establish acceptance.
