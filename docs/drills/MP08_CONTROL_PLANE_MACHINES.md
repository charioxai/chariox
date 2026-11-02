# MP-08 / MP-09 / MP-10 / MP-11 — connected machines

Local protocol 959 / peer 408. Machine acquisition is an optional connected
service capability; compute catalogs, limits, billing and policies stay in Cloud.
The approved Machines window is maintained by the Machines UI lane. This drill
adds no Health placement or alternative UI.

1. Build the base and use its real client to send `ListManagedEnvironmentCatalog`.
   Retain the `client_cloud_authority_required` refusal as the red control.
2. Sign the owner's actual client into an isolated hosted Cloud stack and connect
   it to a separately enrolled home kernel. A client relay-token issuance binds
   that kernel's active enrollment generation to the authenticated owner session.
   Kernel enrollment or a Room invitation by itself confers no rental authority.
3. Start a real official provider on the home machine and approve its normal
   Chariox process grant. It sends `ControlPlane` requests with `chariox request`:

   ```json
   {"ControlPlane":{"action":"capabilities"}}
   ```

   Without the optional `machine_allocation`, expose no hosted actions. Otherwise
   preserve its opaque `revision` and use `action: acquire`, `request_id`, and
   `capability_revision`. Cloud selects placement. Retry with the same request ID;
   never invent a second create after an unknown result.
4. Poll `action: get` with `allocation_ref`. The response distinguishes machine
   state from `access_state`. Available access means the home gateway checked the
   canonical enrolled machine/kernel/key against relay discovery and reached the
   kernel under the same process grant. A pending access reason is a stable code;
   it is not readiness. Use `chariox request --kernel <kernel_id>` for ordinary
   Room exec/files; never SSH or create a VM agent as an access proxy.
5. `action: list` returns stable machine and saved-state refs in creation order.
   Start/Stop/Release require `allocation_ref`, `expected_revision`,
   `capability_revision`, and a fresh `request_id`. Release is permanent Delete.
   Saved-state Start/Delete also require the list's `saved_state_ref`; these refs
   are Cloud record identities, never raw provider snapshot identifiers. Cloud
   reuses the same lifecycle, generation and idempotency fences as the window.
6. Refuse another account, a Room member lacking the grant, an ended grant,
   stale capability/revision, and a mismatched enrolled target/key. Sign the
   connected client out, then prove subsequent requests cannot dispatch. Normal
   session refresh must preserve the binding; a new login requires reconnection.
7. Use the identical OSS binary self-hosted. Discovery is empty and forged rents
   fail before dispatch; ordinary paired-kernel exec/files still work.
8. Before any paid drill record live operator counters, reservation and the
   cleanup deadline. Use the brief's <=2 starts /40 default-minute cap. Delete only
   exact owned allocations through the common lifecycle, capture usage before
   Delete, and confirm provider absence twice at least 60 seconds apart. Preserve
   S1 and p1ret. Record source/binary hashes, screenshots/logs and all cleanup
   outside the repository. TEST trust is explicitly separate from production
   release authenticity; never persist or expose a private signing key.
