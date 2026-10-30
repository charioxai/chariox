# MP-08 / MP-09 / MP-10 / MP-11 — independent provider account copy

`CopyProviderAccountToKernel` is an explicit source-kernel operation. It is separate from credential-free owner Copy and copies exactly one authenticated Vault-backed Claude profile. The request contains `provider`, exact `account_profile`, stable `operation_id`, enrolled `target_kernel_id`, pinned `target_key_thumbprint`, and `replace` (false by default). The source caller must be its local or enrolled Cloud owner under existing kernel access authority.

The source checks the existing authenticated Cloud owner-peer directory, live relay key, advertised `provider_account_copy_v1` capability and peer protocol >=215 before resolving the selected Vault credential. The protected encrypted peer payload binds provider/profile, source/destination identities and keys, operation ID and replacement choice. No local response carries the credential. The target validates relay-bound owner/realm/key claims and its own enrollment, then verifies using official Claude. It stores under its own canonical provider-only Vault handle before atomically publishing the managed profile and safe receipt. No credential enters native profile files. Target and source remain independent.

The target derives a stable copy profile ID from source kernel/key/provider/profile and destination kernel. Existing profiles need a new operation with explicit replacement. Successful exact-operation retries return the saved receipt. Interrupted publication can retry after recovery; an orphan Vault handle is not a selectable authenticated profile. Verification failure leaves the previous profile and credential intact. Unlock Vaults through existing product controls; this operation does not solicit or accept passkeys or raw credentials.

Public result: `ProviderAccountCopied { receipt }`, containing only operation/provider/source profile/target kernel/target profile identifiers. Failures use constant `provider_account_copy_*` codes and remediation text. Raw provider, parser, network and Vault diagnostics never appear in copy errors. Gateway attempt IDs remain at-most-once; retries of the domain operation use the same `operation_id` with a fresh gateway attempt command ID after a settled refusal or uncertain outcome. Gateway requests allow 150 seconds for bounded discovery, protocol probe and official verification.

| Acceptance ID | Required real scenario |
| --- | --- |
| provider-account-copy/PAC-01 | From the coordinator's real existing grant, copy the real Vault-backed Claude account on the lanes source to a disposable independently enrolled same-owner kernel through the encrypted relay. Use the built CLI and kernel; complete official verification and one short real Claude Haiku 5.5 turn on the target. Prove source profile/default/credential unchanged, target native files contain no credential, target owns its handle after source disconnect, and cleanup removes the target copy and disposable resources. |

Supplementary regression drill: `cargo test -p chariox-kernel --lib account_copy`. It covers the public source request, authenticated encrypted peer delivery, wrong owner/realm/key/profile, unavailable and locked targets, conflict/explicit replacement, lost acknowledgement retry, interrupted registry publication after Vault storage and retry, official verification rejection, redacted errors/receipts/history/logs and unchanged source/native files. Synthetic fixtures are never PAC-01 acceptance.

PAC-01 is blocked until dogfood upgrades the lanes kernel from af52049173 to a runtime containing this operation. That is the only authorized real-account source; do not touch token files or restart shared services to bypass it.

MP-08 / MP-09 / MP-10 / MP-11: the target durably reserves the operation binding before verification and Vault storage. Retrying an interrupted operation with a different profile, credential or binding is a conflict; use a new operation ID when the source credential changes.

MP-08 / MP-09 / MP-10 / MP-11: verification releases the profile registry lock after persisting its reservation. Publication revalidates that reservation and the target profile, preserves unrelated registry writes, and refuses concurrent target changes. Source Vault mutation locks cover credential resolution only; they are released before awaiting the encrypted peer. Paused-verifier and simultaneous bidirectional-copy regressions cover these lock boundaries.

## Copy from the receiving kernel

MP-08 / MP-09 / MP-10 / MP-11: local protocol 710 and peer protocol 274 add
`ListProviderAccountCopyOffers` and `CopyProviderAccountFromKernel`. Offers are
metadata for authenticated, Vault-backed Claude profiles on live, same-owner
enrolled peers with pinned keys and `provider_account_copy_pull_v1`. They are
separate from the receiving kernel's selectable local profile inventory. The relay metadata
uses an explicit `copyable` flag; native authentication type retains its meaning.
Already installed copies are excluded. Removal makes the source available for
a new explicit copy operation. A kernel publishes its provider accounts to the
relay directory again as soon as they change, not only at token renewal, so a
login stored on the source is offered at once.

A receiver whose Vault is locked or was never created asks for the passphrase
after consent through the ordinary Vault unlock interaction of the session the
copy was started from (`session_id` and `agent_id` on the request), then
finishes the same operation. Without a session the copy fails with the Vault
message and can be retried.

The receiving kernel binds an existing detached `RuntimeInteraction` to the
source profile and key, destination key, operation ID and replacement choice.
Only an explicit owner answer through `AnswerUserDomainInteraction` continues;
cancellation, expiry and a dropped waiter transfer nothing. The client waits
without replaying this authorization request. After acceptance, the authenticated
recipient asks the source through `RequestProviderAccountCopy`. The source binds
the recipient to the relay-authenticated kernel identity and key, then uses the
existing `CopyProviderAccountToKernel` admission, encrypted transfer and durable
receipt. A source removal, changed key or enrollment refusal fails definitively.

Web Account choices and TUI `/account` choices label the source and offer
**Copy login to this machine**. TUI `/provider accounts copy` also lists offers;
`/provider accounts copy <source-kernel> <profile>` requests a specific copy.
Both clients use the same kernel-owned approval and receipt. Removing the
receiving profile does not remove the source profile or synchronize either
kernel's later provider state.

Supplementary focused regression: `cargo test -p chariox-kernel --lib account_copy`.
The recipient test uses the public router and authenticated encrypted relay,
checks metadata-only discovery and cancellation, accepts the shared approval,
then removes the target registration and checks the source remains usable.
Real acceptance still requires all five scenarios in
[the target-scope validation plan](../PROVIDER_ACCOUNT_TARGET_SCOPE_VALIDATION_PLAN.md).
