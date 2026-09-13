# Combined worker and reliability candidate

This candidate extends worker integration `16c38d6a91` with the following
existing reliability patches, applied without conflicts:

- `d75a6625a5`: recover missing live remote provider-run bindings.
- `cd564d1366`, `e71ad139da`, `ee471a844c`: deliver agent messages through
  provider steering, outside the user queue, and record success only after
  provider acceptance.
- `4ac21fe935`: retain leased prompt echoes and projected queue promotion.
- `38b1441bd3`: check managed Docker readiness through the slice broker.
- The completed-turn fixture correction from `7fde563d5b` supplies required
  settlement and termination fields. Its obsolete bootstrap changes are excluded.

All changed Rust files pass formatting checks. The focused client activity
dependency graph type-checks, and all 18 Node activity tests pass. Test output
was generated outside the repository and removed afterward. No GitHub CI ran.

## Required before deployment

The combined Rust source has not yet been compiled or executed. Run the remote
prompt recovery, local and leased agent-message delivery, slice broker readiness,
provider launch policy, managed isolation, and worker bootstrap regression tests.
Then build and attest this exact combined commit with the pinned release builder.
The earlier `16c38d6a91` build is not this combined candidate.

Validate the resulting binary's protocol, package signatures and source identity,
then use the transactional upgrade with rollback and live health checks. A green
build does not prove live Web/TUI synchronization or Cloud VM lifecycle behavior.
Those acceptance gates remain governed by the full end-to-end plan.
