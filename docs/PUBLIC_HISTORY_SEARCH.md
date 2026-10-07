# MP-08 / MP-10 / MP-11 — public room history search (A09)

Enable the transitional `CHARIOX_ROOM_AGENT_TOOLS=1` on the kernel. Ordinary
room agents use `chariox.history.search` with literal task terms:

```json
{"query":"compiler regression","agent_ref":"peer-review","limit":20}
```

Results contain bounded `hits`, opaque `event_ref` values, an optional
`next_cursor`, and `coverage`. Pass the unchanged cursor with the same query
and agent/session scope to paginate. Pagination is available after the authorized
room finishes rebuilding; retry the initial query while `coverage.rebuilding=true`.
Rebuild progress and counts are restricted to that room. Results are newest first; ranking never
uses another room's corpus statistics. Read a result with
`chariox.history.read({"event_ref":"<returned reference>"})`. Room turn reads
also use this public projection. Event references never resolve filesystem
paths or arbitrary blobs. Missing, excluded and foreign references return the
same unavailable result.

MP-08 / MP-11: admission derives identity from the live provider or home-admitted
lease. Agents can search any peer in their authorized room. An explicitly
named other session requires a currently live sudo turn and the same owner;
there is no global/owner argument or implicit sudo scope. The home kernel
remains the history authority for leased agents using the existing forwarding
and history projection paths. Cloud and relay receive no search index.

MP-08 / MP-11: the minimal protected projection is introduced here because the
base has observation scrubbing but no durable public FTS source. Public prompts,
answers, normalized tool records and safe errors are projected before insertion
into SQLite FTS5. Private reasoning, hidden/protected metadata, attachment
bodies, private overlays and auth/Vault/login/passkey/hand-off tools are excluded.
Known Vault echoes and encoded variants use the existing Room protection registry.
Nested private tool fields are removed. Protection failure fences reads; legacy
raw/semantic recall tools are unavailable to agents while the transition flag is
on, preventing a raw-history bypass. Legacy sessions with the flag off retain
their existing tools; this does not retire `/meta` or implement PR11 migration.

MP-08 / MP-10 / MP-11: only newly admitted sanitized records are indexed. Raw
legacy imports have unknown provenance and are explicitly excluded. Rebuilds
advance in bounded transactions from sanitized rows and resume after interruption.
A redaction-version change excludes incompatible projections; it never imports
raw history. Retention/source replacement invalidates rows and cursors. Changes
to secret provenance conservatively remove prior public rows for that Room
(all worker-local rows for a home-bound slice), invalidate pagination and truncate
the index WAL before the protected mutation proceeds. Safe earlier rows may
therefore become unavailable; coverage reports that exclusion rather than
silently replaying raw history. SQLite and FTS secure-delete are enabled, temporary
pages remain in memory, and the history database is private to its owner.

MP-08 / MP-10 / MP-11: coverage reports index/redaction versions, indexed and
excluded counts, sequence coverage, rebuild progress, retention gaps and truncated
records. Public text is bounded to 64 Ki characters, snippets to 512 characters,
queries to 1,024 bytes / 32 literal terms, pages to 50 hits and turn reads to 200
records. Any exclusion, truncation, retention gap or unfinished rebuild makes
`complete=false`. This is coverage of the retained public projection, not a claim
of complete provider archival history.

MP-08 / MP-10 / MP-11: allocated local daemon protocol453 carries these tool
schemas/projections; relay peer protocol73 is unchanged. Shared version assertions
and focused tool/result hash snapshots guard the contract. Existing clients can
show provider tool results without a new client-side history authority.

MP-10 / MP-11: source regressions supplement the frozen 1,584 A09 live cells.
Acceptance requires built TUI/web/kernel, official real-account providers, hosted
network conditions, placements, protected login/hand-off and all appendix faults.
A missing account or integrated client/environment is an explicit coordinator
blocker, never a fixture substitute or an acceptance pass.

MP-08 / MP-10 / MP-11: see [the real TUI replay](../scripts/public-history-search-drill.md)
for the focused user-driven protocol drill and exact resource prerequisites.

MP-08 / MP-11: kernel history producers resolve trusted room-owner provenance
before transcript/projection locks. This internal field is skipped by serde;
serialized history cannot supply it. Query services release projection locks to
revalidate canonical authority, then fence the room/index revision before releasing
a result. Invalidated snapshots fail visibly and require a fresh query.
