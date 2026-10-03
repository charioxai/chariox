# Publisher enrollment operation persistence

This internal API records an explicit user enrollment request before prompting.
Its immutable input is owner/request identity plus session, publisher ID, key ID,
public key bytes, and expected trust revision. A submitted publisher key, package,
or agent assertion never enrolls itself. The retained kernel controller must
authenticate the owner/session and obtain the generic kernel-operation human
decision; no public request decoder or automatic approval is supplied here.

`begin_publisher_enrollment` is idempotent for exactly the same input.
`arm_publisher_enrollment` replaces the pending interaction nonce and returns an
opaque challenge retaining the original monotonic deadline and exact input.
An old nonce or a fresh writer budget cannot revive an expired decision.
`decide_publisher_enrollment` verifies the challenge in the writer transaction;
approval composes `publisher_trust::enroll_in` and the operation receipt in that
same transaction. Denial and cancellation do not mutate publisher trust.

Approved status is historical evidence of that exact decision. Replaying it
after a later revocation never enrolls the key again. Existing trust CAS and
immutable key binding still apply; changed trust or exhausted trust capacity
becomes a failed operation. The operation status call uses a writer barrier to
settle an earlier lost reply after a successful commit. A SQLite commit failure
returns an uncertain result and stops the shared writer through its existing
fatal fence. Later reads or installation activation must not proceed while that
durability uncertainty remains unresolved; an empty transaction is not repair.

Recovery discovers at most eight pending owner/request pairs through an indexed
query, then repeats the current writer state before showing a fresh prompt.
Pending discovery grants no consent. History retains the existing trust-store
limits of 512 requests per owner and 32,768 overall; pending work is limited to
eight per owner and 32 overall. Terminal request identities are not pruned or
reused. Operation history does not consume the trust store's reserved revocation
capacity or change its existing revocation API.

The borrowed-transaction helper uses a savepoint, so a caller handling an inner
enrollment error cannot commit an orphan decision receipt. The kernel operation
tests exercise real SQLite rollback, owner/input fences, stale/expired prompts,
nonce reuse, cancellation/denial, historical replay and pending limits. The
retained interactive controller is in `runtime/app_publisher_control`; the public
wire route and full client-to-kernel human approval drill remain integration work.
Kernel regression sources are not evidence of execution until their selected
hosted or guarded test gate passes.
