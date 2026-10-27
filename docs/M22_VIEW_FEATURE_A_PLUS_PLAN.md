# M22 View Feature A+ Hardening Plan

## Objective

Make the View tab production-grade for user-supervised slice work: stable screen display, reliable selected-agent prompting, clear shared transcript behavior, safe display endpoint handling, and first-class slice controls in the same page where the user observes and drives the agent.

## Current Baseline

- The View route shows the headed slice screen, prompt area, and selected-agent trace.
- The trace pane uses the shared `AgentTranscriptPane` behavior instead of View-specific turn tabs.
- Completed history turns are projected collapsed by default.
- Display endpoint loading, history loading, selection activation, and route rendering still live together in `ViewRouteCoordinator`.

## Improvements

### 1. Async Lifecycle Hardening

- Replace coordinator-wide display state with per-slice display records keyed by `sliceId`.
- Replace in-flight sets with request sequence tokens so stale endpoint/history responses cannot overwrite the current selection.
- Add bounded retry/backoff state for display endpoint and trace history failures.
- Keep explicit retry buttons immediate, but prevent automatic refresh loops after repeated failures.
- Invalidate trace history when session activity advances, selected agent transitions from working to idle, or runtime live entries no longer cover recent completed turns.

### 2. Display Endpoint Trust Boundary

- Add strict validators for `SliceDisplayEndpoint` and `SessionHistoryOutline` payloads instead of casting unknown protocol payloads.
- Require embeddable display endpoints to be HTTPS, relay/display-origin trusted, and capability-compatible with the View iframe.
- Preserve local-only endpoints as open-only actions with a clear reason they cannot be embedded.
- Add regression tests for malformed endpoint payloads, unexpected origins, missing URLs, expired endpoints, and local-only endpoints.

### 3. View Page Slice Controls

- Add compact slice controls directly in the View screen header or adjacent toolbar:
  - Save and restart agents.
  - Save and shut down slice.
  - Open externally.
  - Retry screen.
- Reuse the existing slice save-state path used by Freeform; do not add a separate View-only save implementation.
- Use the same confirmation behavior and status messages as Freeform.
- Keep controls disabled with clear titles when no selected slice exists or the selected slice cannot be saved.

### 4. Transcript Integration Polish

- Key `AgentTranscriptPane` by selected agent/session so local expansion state cannot leak across agents with similar turn IDs.
- Preserve the Freeform transcript rendering contract: all turns shown in sequence, completed turns collapsed, summaries visible, active output visible.
- Add focused tests that switch selected agents and verify expansion state does not carry across traces.
- Add tests for mixed loaded history and live runtime output ordering.

### 5. Responsibility Boundaries

- Extract `ViewDisplayEndpointStore` for endpoint cache, expiry, retry, and request sequencing.
- Extract `ViewTraceHistoryStore` for history cache, invalidation, retry, and projection.
- Extract `ViewSelectionModel` for selected option derivation and active-session defaults.
- Keep `ViewRouteCoordinator` as wiring: compose state, call stores, activate selected option, mount React.
- Keep projection functions pure and covered by unit tests.

### 6. UX And Accessibility

- Make screen loading, unavailable, local-only, and error states visually distinct.
- Ensure retry/open/save controls have accessible labels and keyboard focus behavior.
- Keep the screen iframe and trace pane stable during refresh; avoid layout jumps when endpoint state changes.
- Add a small selected slice/agent identity block near the screen header so the user can verify what they are controlling.

### 7. Validation

- Unit tests:
  - stale endpoint response is ignored after selection changes.
  - endpoint failure does not auto-retry-loop.
  - explicit retry bypasses backoff.
  - malformed/untrusted endpoints are rejected.
  - history invalidates after relevant session activity changes.
  - transcript expansion state is isolated by selected trace.
- Browser/View drills:
  - open View tab, select a headed slice agent, verify screen iframe + prompt + trace render together.
  - submit a prompt from View and verify the same agent receives it.
  - verify trace shows all turns collapsed with summaries.
  - trigger save/restart from View and verify the slice returns with the same user-visible state.
  - capture screenshots of the View page for normal, loading, error, and save/restart states.

## Acceptance Criteria

- View has no View-specific transcript selector or duplicate status badge.
- View screen state cannot be corrupted by stale async responses.
- Endpoint and history failures are observable but do not retry indefinitely.
- Display endpoints are validated before embedding.
- Slice save/restart controls are available from the View page and reuse shared save-state behavior.
- The coordinator is reduced to route orchestration, with endpoint/history/selection logic in named modules.
- Focused tests and browser screenshots prove the main user workflows.
