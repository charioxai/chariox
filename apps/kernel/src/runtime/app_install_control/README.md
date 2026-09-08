# Installation operation control

Protocol 295 adds `BeginAppInstall`, `GetAppInstallOperation`, and
`CancelAppInstallOperation`, returning `AppInstallOperationStatus`. Begin accepts
the current session, a stable request ID, an already uploaded opaque handle and
the canonical expected package digest. The authenticated owner is derived by the
same App control route used for local, relay and browser requests. No request can
supply a publisher key, approval, deadline or kernel path.

The existing writer first commits a `Preparing` operation receipt. The response
does not wait for verification or human approval. Replays bind the exact original
session, handle and digest; a cancelled operation cannot be restaged by a delayed
verification result. Original receipts remain available after upload expiry.

AppControl retains one bounded owner, driven by the existing kernel pump. It
holds at most 32 operations, four background jobs and four retained terminal
request jobs; both task sets share the existing eight App admission permits and
package preparation reservation. A disconnected request remains owned until its
blocking writer call finishes. Its original deadline also observes shutdown, so
a queued request cannot obtain a fresh authorization budget. An admitted Cancel
retains its original deadline and completes its negative durable write while
shutdown joins it, preserving cancellation across restart. Human
waits hold no App permit. Tasks retain only stores and lifecycle services, never
the containing runtime state. Shutdown cancels admission and joins the owned task
sets before stopping workers. An already running blocking verifier retains its
permit and release lease until completion even if its waiter is cancelled.

Verification and immutable release publication use the enrolled publisher key.
The same writer transaction stages the verified candidate and records its review
metadata. This metadata contains the exact package/capability digests, signer
fingerprint/revision and signed declarations. It does not grant access to declared
information sets; those are explicitly labelled `not_granted`.

The controller registers an owner-scoped kernel-operation RuntimeInteraction in
the original session. Approve/decline uses the existing authenticated terminal
response route. A private challenge retains the exact nonce, stage, signer,
capability digest and original five-minute monotonic deadline. The deciding writer
transaction checks all of them, including after queue/SQLite delays. Replaced,
cancelled or expired challenges cannot become fresh consent. Recovery may ask a
new question, but cannot replay an old human response as a new authorization.

An approved stage enters the existing health/commit/startup lifecycle. There is no
second activation owner or readiness proof. Cancel is precommit-only: a committed
operation returns conflict and does not undo installation data. A single cancel
observed during saturated admission stays owned until its durable cancellation
and any worker cleanup complete; Busy is not a claim that cleanup has completed.
Recoverable storage/admission errors back off instead of spinning; irreconcilable
commit uncertainty uses the existing fatal writer fence.

The focused fixtures exercise real upload verification, SQLite, the retained pump
and generic owner decisions; the existing libc lifecycle fixtures cover health
and startup. They do not prove a real Node App launch. Publisher enrollment is a
separate prerequisite. Normal `/app install FILE` file upload, default session and
retry ID handling, and OS file association remain client integration work; opaque
upload handles are an internal transport detail rather than the normal user flow.
