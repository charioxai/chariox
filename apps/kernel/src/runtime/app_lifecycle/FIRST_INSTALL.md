# First-install lifecycle integration

`AppControlService::prepare_first_install` accepts the authenticated owner
separately from an opaque request ID, complete upload handle and canonical
`sha256:<64 lowercase hex>` package digest. The digest correlates retries; it
does not establish publisher trust. The existing upload service, enrolled-key
verifier and immutable release store produce the candidate. A repeated request
returns its original operation after a writer durability barrier, including after
the original upload expires or is aborted. Different bytes conflict.

The existing SQLite writer commits the supervised first stage and operation
receipt together. The initial state is `AwaitingApproval`. The existing staged
`CapabilityDecision` is the approval authority; no caller boolean, package-carried
key or synthetic agent ID grants access. A supervised stage cannot activate
through the older foundation commit API.

After approval, the retained lifecycle service serializes each installation and
owns the same four-worker, one-preparation and eight-operation limits used for
active-generation restart. The writer resolves the exact stage, approval,
enrollment revision and attempt before preparation. The owner reopens and
verifies the anchored signed archive, retains its sealed release lease, and
constructs the existing platform worker and its sole SDK peer.

SDK 0.5 / protocol 293 adds a distinct `health_check` lifecycle round trip before
activation. It has a three-second limit, also bounded by the actual pending
`worker.ready` request's original monotonic deadline. The worker remains
`Starting`: every broker SDK effect is denied. An App can perform contained local
initialization or validation; an absent handler returns the trusted SDK's normal
null response. This does not prove network availability or business outcomes.

The same committing writer transaction rechecks signer, capability decision,
attempt and health catalog, then records the active generation, committed
operation and worker restart intent. Only its private committed proof lets the
retained peer acknowledge readiness. Normal `startup` then runs with the approved
SDK, and successful startup precedes publishing the callable App handle. Startup
may arrive on the peer before the readiness response is written; both follow
activation, and the peer handles them independently.

Precommit health/preparation failure aborts the stage only after the actual
process and broker owner are reaped. A withdrawn signer cancels the pending
operation, preventing recovery from silently regranting it. A user stop before
the initial claim must persist both operation cancellation and worker stop
intent; saturation retains the pending stop owner until those writes succeed.
Postcommit startup failure cannot roll back installation data. It records an
ordinary failed active worker; a kernel crash after commit instead retains the
durable restart intent for existing headless recovery.

An uncertain activation commit is reconciled against the exact operation,
generation, approval and worker intent in another FULL-synchronous writer
transaction. Irreconcilable state stops the writer. No caller reports rollback
merely because an acknowledgment was lost. Recovery alternates bounded pages of
approved pending first installs and active workers using the existing kernel
pump, without opening an App view.

Operation tombstones are retained: arbitrary request IDs have no safe replay
expiry. Admission backpressures at 4,096 receipts per owner / 16,384 total, and
eight pending operations per owner / 32 total. Failed/cancelled receipts retain
`cleanup_pending`; this component releases process, temporary mount and code
leases, but does not claim deletion of retained private data or shared immutable
releases. Clearing that marker requires the platform storage owner's exclusive
cleanup operation. It must never remove workflows or agents.

The production preparation factory currently supports Linux. macOS preparation
fails explicitly until its enrolled, signed platform factory is integrated.
Fixed libc tests exercise actual FD registration, denied precommit SDK access,
postcommit startup file writes, publication and reap; they do not establish real
Node execution, macOS signing, App views or complete Phase 1 acceptance.

The protocol 295 control adapter is documented in
[`app_install_control/README.md`](../app_install_control/README.md). It adds a
durable `Preparing` receipt before upload verification, then reuses the same
stage and lifecycle authority described here.
