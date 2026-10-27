# Local publisher trust

This Phase 1 registry persists explicit local developer publisher enrollment and
revocation in the kernel's existing SQLite database. It does not interpret
package-declared keys as trusted, implement Library publisher management, enroll
a human authenticator, or decide whether a caller may grant trust. The kernel
must supply the authenticated owner and a recorded decision bound to the exact
publisher ID, key ID, Ed25519 public key and requested change.

Identity follows the package manifest's exact ASCII identifier contract. Trust
is scoped to `(owner, publisher ID, key ID)`. Key bytes are immutable after the
first enrollment, including after revocation; a different key needs a new key ID.
Invalid/weak Ed25519 points are rejected. No private signing key is accepted or
stored. Publisher display names confer no authority.

Each explicit enrollment or revocation consumes the expected revision and
increments it without wrapping. Re-enrollment after revocation requires a fresh
explicit decision and the current revision. Decision IDs are unique per owner;
reusing one for another action, identity, key, expected revision or authority
reference conflicts. A byte-equivalent retry returns the **historical receipt**
and original timestamp. It never changes a later decision. A receipt's `enrolled`
field describes that past decision, not the current trust state.

Package verification takes a `TrustedPublisherSnapshot`. Before committing an
installation, the writer must call `snapshot.require_current(&transaction, owner)`
inside the **same existing write transaction** as the installation mutation.
Checking on a reader first does not fence a later commit. Revocation rejects the
snapshot; revocation followed by re-enrollment also rejects its old revision.
Verified installation staging and `commit_verified` perform this check in their
committing transactions; activation also requires the exact revision retained
when the package was staged. Stopping already-running installations and
projecting authenticated enrollment/confirmation to terminals remain separate
kernel integration work.

Limits are 128 owners, 64 key bindings per owner, 4096 total bindings, 512 decision
receipts per owner and 32768 total receipts. Revoked bindings and receipts remain
stored to prevent silent identity reuse or forgotten conflicting decisions.
Enrollment admission reserves one future revocation receipt for every active
key, so exhausting an enrollment quota still allows revoking those grants.
Exhaustion returns `app_publisher_trust_limit`; it never evicts trust history or
automatically regrants a revoked key. SQLite durability configuration and writer
admission belong to the existing kernel persistence owner.

Focused tests cover reopen/owner isolation, immutable keys, decision conflicts,
revocation/re-enrollment/replay, actual SQL-triggered rollback, competing SQLite
connections, transactional snapshot fencing, quota-preserved revocation,
retained key tombstones, weak keys and revision overflow. They do not stand in
for authenticated human-consent or installer/runtime integration drills.
