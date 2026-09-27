# Slice process limits

The Docker slice provisioner caps processes and threads at 1,024 by default.
This limits process exhaustion independently of memory and CPU limits. Docker
counts threads and processes in descendant PID namespaces against this cap.

Set `CHARIOX_SLICE_DOCKER_PIDS_LIMIT` on the kernel process or direct
provisioner invocation to choose a different positive integer. Empty or unset
uses 1,024. Zero, negative, non-integer and out-of-range values are rejected
before Docker operations. The limit is deployment policy, not a new serialized
Room or slice field, so this does not change the client protocol.

Creation sets the cap before the container can run. Reuse, failed-save recovery
and provider-account setup reconcile the current cap before starting services.
If Docker cannot apply the cap, the operation fails. Stop and destroy remain
available even with an invalid configured cap. Updating a running container
does not kill existing tasks; it prevents creation beyond the new limit.

The default is a starting safety limit, not a supported agent-count promise.
Validate intended provider concurrency and graphical workloads before setting
deployment limits. Exhaustion should produce a bounded actionable failure;
it is not an invitation to retry process creation indefinitely or disable caps.
Memory/CPU limits and host admission checks are still required.

Run `pnpm run test:slice-provisioner` for CLI-to-Docker boundary tests. The
standard repository test command includes this suite; run full CI only at the
final reviewed merge gate. The Room Web/TUI drill explicitly requests and
verifies the 1,024 cap alongside its 2 GiB memory/no-extra-swap and one CPU caps.
