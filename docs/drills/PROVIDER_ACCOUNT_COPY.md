# MP-08 / MP-09 / MP-10 / MP-11 — independent provider account copy

`CopyProviderAccountToKernel` is an explicit source-kernel operation. It is separate from credential-free owner Copy and copies exactly one authenticated Vault-backed Claude or native Codex profile. The request contains `provider`, exact `account_profile`, stable `operation_id`, enrolled `target_kernel_id`, pinned `target_key_thumbprint`, and `replace` (false by default). The source caller must be its local or enrolled Cloud owner under existing kernel access authority.

The source checks the existing authenticated Cloud owner-peer directory, live relay key, advertised `provider_account_copy_v1` capability and peer protocol >=215 before resolving the selected Vault credential. The protected encrypted peer payload binds provider/profile, source/destination identities and keys, operation ID and replacement choice. No local response carries the credential. The target validates relay-bound owner/realm/key claims and its own enrollment, then verifies using the official provider. It stores under its own canonical provider-only Vault handle before atomically publishing the managed profile and safe receipt. Claude tokens stay in the Vault. Codex credentials are verified in a private staging profile and published to the target native auth file; future launches use the provider-owned file, never the saved transfer. Target and source remain independent.

The target derives a stable copy profile ID from source kernel/key/provider/profile and destination kernel. Existing profiles need a new operation with explicit replacement. Successful exact-operation retries return the saved receipt. Interrupted publication can retry after recovery; an orphan Vault handle is not a selectable authenticated profile. Verification failure leaves the previous profile and credential intact. Unlock Vaults through existing product controls; this operation does not solicit or accept passkeys or raw credentials.

Public result: `ProviderAccountCopied { receipt }`, containing only operation/provider/source profile/target kernel/target profile identifiers. Failures use constant `provider_account_copy_*` codes and remediation text. Raw provider, parser, network and Vault diagnostics never appear in copy errors. Gateway attempt IDs remain at-most-once; retries of the domain operation use the same `operation_id` with a fresh gateway attempt command ID after a settled refusal or uncertain outcome. Gateway requests allow 150 seconds for bounded discovery, protocol probe and official verification.

| Acceptance ID | Required real scenario |
| --- | --- |
| provider-account-copy/PAC-01 | From the coordinator's real existing grant, copy the real Vault-backed Claude account on the lanes source to a disposable independently enrolled same-owner kernel through the encrypted relay. Use the built CLI and kernel; complete official verification and one short real Claude Haiku 5.5 turn on the target. Prove source profile/default/credential unchanged, target native files contain no credential, target owns its handle after source disconnect, and cleanup removes the target copy and disposable resources. |
| provider-account-copy/PAC-02 | Local 1013. On a real kernel whose Chariox Vault unlocks "ask every time", launch a Copy of that machine from the web (Machines → Rent → Copy of <machine>) with no signed-in provider login: the unlock taken for the copy's preflight before anything is rented is held for the export, so the transfer completes after the machine is created without a second unlock card and the new machine opens its own copy of the Vault. With the Vault locked and nobody to answer, the copy is refused before anything is rented ("... cannot be copied to a new machine: ... Nothing was rented."). Focused checks: `a_copy_of_this_kernel_holds_the_unlock_for_its_vault_export_without_a_login`, `the_export_opens_the_vault_for_a_vault_login_or_a_copy_of_this_kernel_until_prepared`. |
| provider-account-copy/PAC-RENT | Copy a Vault-backed account from an owner-enrolled source kernel to the same owner's freshly rented Chariox machine (machine credential, no kernel credential), run a real turn there, remove the copy and release the rental. Another owner's rented machine is refused with the standard same-owner message. No secret appears in any receipt, log or history. |

Supplementary regression drill: `cargo test -p chariox-kernel --lib account_copy`. It covers the public source request, authenticated encrypted peer delivery, wrong owner/realm/key/profile, unavailable and locked targets, conflict/explicit replacement, lost acknowledgement retry, interrupted registry publication after Vault storage and retry, official verification rejection, redacted errors/receipts/history/logs and unchanged source/native files. Synthetic fixtures are never PAC-01 acceptance.

PAC-01 is blocked until dogfood upgrades the lanes kernel from af52049173 to a runtime containing this operation. That is the only authorized real-account source; do not touch token files or restart shared services to bypass it.

MP-08 / MP-09 / MP-10 / MP-11: the target durably reserves the operation binding before verification and Vault storage. Retrying an interrupted operation with a different profile, credential or binding is a conflict; use a new operation ID when the source credential changes.

MP-08 / MP-09 / MP-10 / MP-11: verification releases the profile registry lock after persisting its reservation. Publication revalidates that reservation and the target profile, preserves unrelated registry writes, and refuses concurrent target changes. Source Vault mutation locks cover credential resolution only; they are released before awaiting the encrypted peer. Paused-verifier and simultaneous bidirectional-copy regressions cover these lock boundaries.

## Copy from the receiving kernel

MP-08 / MP-09 / MP-10 / MP-11: local protocol 710 and peer protocol 274 add
`ListProviderAccountCopyOffers` and `CopyProviderAccountFromKernel`. Offers are
metadata for authenticated local profiles (same-kernel links, including while offline) and authenticated Vault-backed Claude or native Codex profiles on live, same-owner
enrolled peers with pinned keys and `provider_account_copy_pull_v1`. They are
separate from the receiving kernel's selectable local profile inventory. The relay metadata
uses an explicit `copyable` flag; native authentication type retains its meaning.
Already installed copies are excluded. Removal makes the source available for
a new explicit copy operation.

When the owner's own client (a terminal caller) sends the request, choosing the
offer is the consent and the receiving kernel raises no decision. For any other
caller (an agent, a leased or remote run, a Metaagent) it binds an existing detached
`RuntimeInteraction` to the source profile and key, destination key, operation ID and
replacement choice. Only an explicit owner answer through `AnswerUserDomainInteraction`
continues; cancellation, expiry and a dropped waiter transfer nothing. The source
kernel's rules for releasing a login are the same in both cases. The client waits
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

## Local 1013 and peer 450: Codex copies, same-kernel links and sign-in notices

Cross-kernel copies remain usable without their source kernel. Same-kernel requests use the existing transaction/receipt to register a link to the same credential; they do not fork a refresh lineage. Installed Codex retries preserve target credentials after rotation or a new device login. Profile auth failures expose a safe `auth_error`.

The existing person Inbox notice carries optional `action: {kind: "provider_sign_in", kernel_id, provider, account_profile, label}`. After copying, it advises a separate target-machine login; after expiry it offers **Sign in again on <machine>**. The client dispatches existing StartProviderLogin on action.kernel_id. The Cloud button is a coordinated change, not implemented in this OSS repository. Read/snooze remain in the existing Inbox store. A recovery notice replaces read copy advice and becomes unread; successful authentication clears the recovery item.

| Cell | Required live scenario |
| --- | --- |
| K1 | Copy to another kernel works immediately, with target sign-in advice visible at DPR1 and DPR2. |
| K2 | Target device sign-in creates its own login; both accounts work across rotation on either side. |
| K3 | A stranded interim copy shows Sign in again, routes to that machine and recovers. |
| K4 | Same-kernel linked profiles survive rotation using one credential/helper. |
| K5 | Remove the source kernel; targets work until their own expiry, then request sign-in. |
| 5 | Same-operation retries preserve rotated target credentials; replace=false conflicts are actionable. |
| 6 | Busy source, reload, web and CLI follow the same copy/link path. |
| 7 | Missing/unauthenticated sources fail clearly without exposing a half-installed profile. |

Run builds/tests through cargo-slot.sh. Run drill kernels, relay and Chrome on buildfarm machines, with isolated CHARIOX_HOME. A synthetic public-router/encrypted-relay regression proves transaction behavior; it does not replace real provider rotation or the web cells above.
