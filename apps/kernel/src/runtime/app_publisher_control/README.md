# Publisher enrollment owner

The kernel owns one retained controller alongside App installation control. Its
begin/status/cancel methods receive the owner derived from the shared terminal
route; begin also checks membership of the selected kernel Session. Protocol297
accepts canonical public-key bytes and exact decimal revisions as review input.
No App SDK method, package manifest, public key, or agent decision becomes
publisher consent. The old caller-independent transport response cache is
bypassed; the durable owner/request receipt handles replay.

Begin durably records the immutable request before returning. The kernel pump
loads current writer state, arms a fresh nonce, and projects the existing generic
kernel-operation interaction to that Session. This works with no Agent or focus
Agent. The prompt identifies the exact publisher/key, public key and fingerprint,
and expected trust revision. Only the recorded owner can answer. Approval uses
that exact challenge; denial, cancellation, abandoned waits and expiry never
enroll a key. App installation and capability approval remain separate decisions.

The controller retains at most 32 operations and four total request/background
tasks, sharing the existing eight App-control permits. Round-robin selection
prevents a retrying prefix from starving later operations; a due indexed recovery
scan reserves capacity before foreground jobs. Recovery repeats writer state and
uses a fresh prompt nonce. It does not replay an earlier human response.

Request deadlines begin before queueing. A disconnected caller cannot discard a
writer job or its admission permit. Shutdown stops new work, cancels positive
admission, drops human wait receivers, and joins both task sets before the shared
writer is torn down. An already admitted explicit Cancel retains its original
30-second deadline and completes its negative durable write during that drain.
No owned task clones `KernelRuntimeState`. Commit uncertainty stops the shared
writer; later publisher snapshot reads check its health, and activation still
requires its existing committing writer fence.

The controller regressions cover round-robin fairness, zero-Agent exact
human approval and owner rejection, denial/cancellation, fresh recovery nonce,
reserved scan capacity, and disconnected Begin/Cancel ownership through a real
SQLite lock wait. They use kernel/SQLite fixtures and no provider or App process.
The public adapter regression additionally checks owner isolation, malformed
keys, noncanonical/out-of-range revisions and cancellation without enrollment.
Transient admission pressure returns Busy; durable quota exhaustion is distinct.
These Rust fixtures require hosted execution. The shared-client typecheck and
four request tests pass. A publisher-file terminal command and live terminal
approval drill remain to complete the user flow.
