# One shared environment per Room

## Status

Accepted.

## Context

Agents and users need browser and graphical-computer access that preserves one authenticated profile, visible state, and interaction history across Web and TUI clients. Giving each agent a private browser would split that state and make human takeover ambiguous. Letting a client own the viewer session would also move runtime authority out of the kernel.

## Decision

Each Room owns one default browser and graphical Environment under kernel authority. Users and agents share its tabs, browser profile, desktop, viewport, input arbitration, history, and saved state. Attaching another agent or client never creates a private Environment implicitly. A user may explicitly create another Environment in a future product model.

## Consequences

- The kernel must own Environment identity, lifecycle, mutation ordering, takeover, history, and recovery.
- Browser and Computer modes must be projections of the same Environment rather than separate runtimes.
- Workers execute browser, controller, streamer, desktop, and input processes but do not become authorities.
- Clients must render kernel state and submit scoped Actions; they cannot invent local state transitions.
- Shared authentication and state simplify collaboration, but concurrent mutations require explicit reservations and deterministic ordering.
- Supporting multiple explicit Environments later will require a product and protocol change; it cannot be inferred from attachment count.

## Rejected alternatives

- Per-agent browsers: rejected because they fragment authentication, tabs, storage, and observability.
- Client-owned viewer sessions: rejected because Web or TUI clients would become runtime authorities and reconnect semantics would diverge.
