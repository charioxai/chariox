# Prompt settings and injection

Chariox-owned agent-directed instructions have stable Markdown template IDs and are materialized under
`$CHARIOX_HOME/prompts`. The kernel is the source of truth: it loads bundled defaults, applies a user
override when present, records the current/default SHA-256 values in the prompt manifest, and uses the
same registry during replay and restart.

## Local API

The local daemon protocol (version 261) provides:

- `ListPromptSettings` and `GetPromptSetting` for catalog/read access;
- `PreviewPromptSetting` for bounded variable substitution without mutation;
- `UpdatePromptSetting`, `ResetPromptSetting`, and `ResetAllPromptSettings` for mutations.

Local CLI/IPC callers are trusted. Remote callers must be authenticated and listed in the kernel's
`CHARIOX_PROMPT_SETTINGS_ADMIN_USER_IDS` comma-separated administrator allow-list; an identity alone is
not permission to mutate global instructions. Protected runtime/security templates are catalogued and
resettable but cannot be edited. Markdown is validated for non-empty content, balanced `{{VARIABLE}}`
delimiters, preservation of every variable required by the bundled contract, and a 64 KiB limit before
it is persisted.

The TUI exposes the same kernel-owned reset path through the shared shell namespace:

```text
/settings prompts list
/settings prompts reset <template-id> --confirm
/settings prompts reset-all --confirm
```

The command first reads the catalog and sends the current revision/hash as an optimistic concurrency
check. It cannot silently overwrite an edit made by another client, and reset-all includes every catalog
entry (including protected entries, which are resettable but not editable).

## Context minimization

Workflow instructions and event context are assembled by the scheduler's prompt-injection service. Event
reply context stays structured and internal. The `reply_to_event` runtime action is absent from ordinary
provider tool lists and appears only for a session with an active event binding whose reply mode is enabled;
the runtime dispatcher revalidates the binding and invocation before any provider call.

Do not place credentials, raw provider catalogs, installation metadata, or unbounded event payloads in a
prompt. Add a new Markdown template only when the text is an agent-directed instruction; protocol errors,
operator notices, and UI copy remain code-owned.

## Adding a prompt

Add one default Markdown file under `apps/kernel/src/provider/`, give it a stable namespaced ID in
`bundled_templates()`, and register its scope, audience, provider applicability, and editability in
`prompt_setting_metadata()`. Use `{{VARIABLE}}` only for bounded values supplied by the prompt-injection
service. Add a focused assembly test that proves the template is loaded through the registry, and include
the entry in the catalog acceptance test. Never duplicate the same agent-directed prose in a provider
adapter or client.

## Override scope and deployment

Overrides are kernel-local user settings stored below `$CHARIOX_HOME/prompts`; they are not Cloud-owned
and are never copied into provider credentials or the relay. A workflow run uses the override from the
kernel that executes it. Exporting or deploying a workflow does not implicitly transfer a user's prompt
overrides: the destination kernel loads its bundled defaults until its administrator explicitly applies
the desired settings there. This keeps deployment reproducible and prevents a local instruction edit
from silently changing a hosted runtime. The prompt manifest records template IDs and hashes so replay,
restart, and diagnostics can identify the exact source used for a run.

Protected entries carry `protected=true`, remain visible for inspection, and can be reset individually or
globally, but cannot be edited because they contain security/protocol invariants. Their enforcement still
lives in code; editing explanatory Markdown never changes authorization or dispatch policy.

## Provider limitations

The kernel remains the only prompt assembly authority. Codex and OpenCode receive hidden Chariox context
through their provider-native bridges; Claude native TUI uses the `UserPromptSubmit` hook bridge. If a
provider bridge is unavailable, Chariox fails closed or runs without hidden context according to the
provider contract; it must not inject hidden instructions as visible terminal text.

## Troubleshooting

- **Catalog unavailable:** connect a protocol client to a kernel with local-daemon protocol 261 or newer
  and call `ListPromptSettings`. The TUI and raw local-daemon clients always retain the bundled catalog
  as their offline source. The Chariox Cloud Settings page is shipped by the matching `chariox-cloud`
  release and uses the same protocol; deploy the two releases together when using its Sync catalog action.
- **Save conflict:** another administrator changed the prompt. Re-list the catalog, compare the revision/hash
  and save again; the kernel never overwrites a newer revision silently. Cloud surfaces this as a conflict
  and the TUI reports the failed optimistic-concurrency request.
- **Validation failure:** check that Markdown is non-empty, no larger than 64 KiB, has balanced variables,
  and preserves every variable required by the bundled default.
- **Unexpected prompt content:** inspect the run's prompt manifest and compare the current/default hashes;
  reset the affected entry or reset all entries with the local-daemon API or TUI. The Cloud Settings page
  exposes the same reset operations when connected to the kernel.
