# Disposable worker activity reporting

The disposable worker supervisor passes its enrollment receipt to the ordinary
kernel activity reporter. The reporter validates the worker Machine, kernel,
relay public key, Cloud profile, capacity, and home caller before reporting.
It does not transfer or refresh provider credentials.

The child starts before enrollment confirmation. An exchanged receipt therefore
enables reporting; Cloud rejects reports until enrollment is confirmed, and the
existing reporter retry loop handles that interval. Restart cursor recovery,
lost acknowledgements, and activity transitions use the same implementation as
independent managed kernels.

Worker reports use `/v1/disposable-workers/activity` with `allocationId` instead
of `environmentId`. The signed fields are accountId, allocationId, kernelId,
machineId, runningAgentCount, and sequence, sorted in that order. The Machine
credential is the HMAC-SHA256 key. Independent managed kernels retain their
existing endpoint and signature contract.

Paired Cloud implementation: chariox-cloud PR #92, head
`f7bc768c4117a1e42c112de3f2408eca145bbf65`.

## Validation still required before deployment acceptance

- Execute the focused reporter and worker-bootstrap Rust tests.
- Exercise bootstrap confirmation delay, restart cursor recovery, and
  busy-to-idle reporting against the paired Cloud service.
- Verify queued prompts and unresolved permissions prevent idle release.
- Verify keep-running, fresh work cancelling a deadline, and disconnected clients.
- Verify VM and billable-resource deletion after release without changing ordinary
  managed-machine stop/start persistence.

Local type-checking is not proof of those live lifecycle requirements. No local
daemon or relay serialized shape is changed by this HTTP reporter connection.
