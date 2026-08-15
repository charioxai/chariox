# Prompt settings and injection

Chariox-owned agent-directed instructions have stable Markdown template IDs and are materialized under
`$CHARIOX_HOME/prompts`. The kernel is the source of truth: it loads bundled defaults, applies a user
override when present, records the current/default SHA-256 values in the prompt manifest, and uses the
same registry during replay and restart.

## Local API

The local daemon protocol (version 259) provides:

- `ListPromptSettings` and `GetPromptSetting` for catalog/read access;
- `PreviewPromptSetting` for bounded variable substitution without mutation;
- `UpdatePromptSetting`, `ResetPromptSetting`, and `ResetAllPromptSettings` for mutations.

Local CLI/IPC callers are trusted. Relay callers must carry an authenticated user identity. Protected
runtime/security templates are catalogued and resettable but cannot be edited. Markdown is validated for
non-empty content, balanced `{{VARIABLE}}` delimiters, and a 64 KiB limit before it is persisted.

## Context minimization

Workflow instructions and event context are assembled by the scheduler's prompt-injection service. Event
reply context stays structured and internal. The `reply_to_event` runtime action is absent from ordinary
provider tool lists and appears only for a session with an active event binding whose reply mode is enabled;
the runtime dispatcher revalidates the binding and invocation before any provider call.

Do not place credentials, raw provider catalogs, installation metadata, or unbounded event payloads in a
prompt. Add a new Markdown template only when the text is an agent-directed instruction; protocol errors,
operator notices, and UI copy remain code-owned.
