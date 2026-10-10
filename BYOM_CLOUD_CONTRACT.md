# Owner-managed Copy contract — aligned with OSS #903

The kernel request is already implemented on OSS `byom/owner-context-transfer`:
`StartManagedContextTransfer { interactive: true, ownerManaged: { target, contextSelection } }`.
The source kernel owns the plan, credential-free export, native RuntimeInteraction review,
relay transfer and progress. The browser uses that existing shape directly. Local daemon
protocol **445** and relay peer **88** remain unchanged; no serialized shape was changed.
The web requires updated source and target kernels and connects through the existing
BrowserKernelClient, never a Cloud runtime proxy.

Cloud issuance is `POST /v1/owner-managed-context-tickets`, authorized by a browser session
with CSRF or by `x-chariox-kernel-credential`. Its body is `{ sourceTargetId, target,
contextSelection, ttl_seconds: 300 }`. `sourceTargetId` is the Cloud directory row ID;
`target` and the returned `source` are exact `{ relayRealmId, machineId, kernelId,
relayPublicKey, keyThumbprint }` pins. The result contains `kind: owner_managed_machine`,
`ticket_id`, the opaque `ticket`, `expires_at`, `sourceTargetId`, `source`, `target` and
`contextSelection`. Storage is hash-only. Tickets are single-use, short-lived and
revocable; both peers must be freshly online and enrolled to the same user/account/realm.
`GET /v1/owner-managed-context-tickets/peers` returns only this user's eligible peers.

The real web flow delegates issuance to the source kernel's credential, so the capability
never enters browser state or the daemon/relay protocol. The source issues and consumes
it inside its shared exporter. Consumption is `POST /v1/managed-kernels/context/ticket`
with `{ kernelCredential, ownerManaged: { ticket, source, target, contextSelection,
contextId, planDigest } }`; the context ID and `sha256:<hex>` digest are kernel-generated.
No environment ID substitute is accepted.

The same endpoint handles credential-bound operation checks:
`{ kernelCredential, ownerManagedExport: { contextId, planDigest, source, target } }`
for source retries, and `ownerManagedImport` with those same fields for target admission.
They require a consumed ticket and the original active grant generations and exact live
peer pins. Only the source credential can re-authorize export; only the target credential
can admit import. Re-authorization lasts at most 30 minutes after consumption, matching
the shared transfer lifetime. It never consumes the capability again or changes its
selection. Account/user/machine/kernel revocation still wins through the shared lock order.

The enabled web action picks a source kernel and Project/kernel context, renders the
source's human-only export review, polls authoritative progress and shows the result.
After Copy, normal provider login runs on the target and exposes an HTTPS authorization
link and, when required, its public device code. The UI neither displays login terminal
commands nor copies credentials. Phone-width screenshots and focused browser/component
checks use explicit synthetic runtime fixtures; they do not prove the real-stack drill.
