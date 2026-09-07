# Managed slice workspace coordination

## Failure

The public-API office workflow creates an extension file in an ordinary directory,
without initializing Git. The worker sees that directory as `/workspace`, while
the home kernel sees the host path. Comparing those physical path fingerprints
alone rejects `write_artifact` as `remote_workspace_not_coordinated`.

Initializing a dummy repository or accepting every `/workspace` path would hide
the problem. The home kernel already owns the slice's mount and agent binding.

## Mapping contract

Keep existing Git and explicit workspace-link coordination. For plain folders
with different physical paths, require the home kernel's current slice record:

- Local Docker backend and running state.
- Matching owner kernel and machine.
- Matching Room, attached agent, worker kernel and worker machine.
- Exact worker root and fingerprint `/workspace`.
- A canonical host mount matching the Room's canonical workspace folder.
- The existing home-agent lease and active provider-run authorization.

Use the home folder's coordination identity so unrelated slices do not share a
global `/workspace` identity. Apply the same mapping checks at dispatch and
finalization, before replaying cached results or publishing changes. Forwarded
relay requests must also match the actual relay sender to the claimed worker.

No serialized request or response field changes. The relay remains transport;
the home kernel owns authorization, permissions and edit coordination.

Workspace identity alone never authorizes a caller. Both existing matching
identities and the managed-mount mapping require the current home-agent binding,
worker identity, leased agent and provider run. Replay and finalization recheck
that binding before returning a cached reply or publishing changes.

## Validation status

The router regression reproduced the non-Git rejection, then passed with the
mapping. A further router regression reproduced acceptance of a stale lease
when the worker claimed the home folder's exact identity, then passed after
moving authorization ahead of identity matching. The focused kernel workspace
suite passes 280 tests, including negative mount/binding cases, relay-sender
checks, permission retry, replay and finalization after provider-run replacement.
Three stale protocol-version assertions and a Docker test fixture missing the
already-required file-descriptor limit were corrected without changing the
protocol version or production provisioning policy.

The official-provider public-API office workflow still needs a rebuilt runtime
and full live retest. Local test results do not establish that workflow passes.

This does not establish support for arbitrary remote folders, SSH Docker mount
translation, mismatched Git branches, production images, managed-machine
acceptance, or browser sign-in import. Those claims require their own evidence.
