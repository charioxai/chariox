# Publisher enrollment owner

The kernel owns one retained controller alongside App installation control. Its
internal begin/status/cancel methods receive an already authenticated owner;
begin also checks membership of the selected kernel Session. No App SDK method,
package manifest, public key, or agent decision becomes publisher consent. A
public terminal request route is not supplied in this slice.

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

The seven controller regressions cover round-robin fairness, zero-Agent exact
human approval and owner rejection, denial/cancellation, fresh recovery nonce,
reserved scan capacity, and disconnected Begin/Cancel ownership through a real
SQLite lock wait. They use kernel/SQLite fixtures and no provider or App process.
The source is not evidence of a passed test or a completed public enrollment
workflow until the corresponding validation gate runs. Production request
routing, shared protocol/client helpers, and an end-to-end terminal approval drill
remain outside this internal slice.
