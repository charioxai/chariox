# Prompt notes: "Leave a note" on selected text

Notes live only as prompt attachments. The kernel keeps no notes store, and there
are no note tools; a note is read from the prompt it was sent with.

## What the user sees

Select text anywhere in the central web-terminal area (kernel Browser view, Room,
agent transcripts, App panels, terminal output). A **Leave a note** button appears
just above the selection. It opens a composer next to the selection: the quoted
text on top, a text field below. Enter (Shift+Enter = newline) or the send button
inside the field attaches the note to the prompt as a chip, like an image; Esc or
a click outside cancels. Clicking the chip reopens the composer; the chip's remove
button removes it. The transcript shows the chips; history replay keeps them.

## Wire shape (protocol 566 / relay peer 174)

A note is an inline `PromptAttachment` with mime `application/vnd.chariox.note+json`
and `contents_base64` of `{"quote","comment","source"?}`. `source.kind` is one of
`kernel_browser`, `room`, `agent` or `panel`; the rest are references, never
surrounding text:

| Surface | Source | Anchor | Tool that reads more |
| --- | --- | --- | --- |
| kernel Browser | page title + URL | `document_id`, quote `prefix`/`suffix` | `chariox.kernel_browser` |
| Room | room + tab title/URL | `document_id`, `prefix`/`suffix` | `chariox.room.overview`, `chariox.slice_browser_text`/`find` |
| another agent's pane | `agent_id` | `session_id` + `turn_id` | `chariox.history.turn` / `chariox.history.read` |
| the receiving agent's own pane | none (quote + comment only) | none | none |

`SubmitPrompt` admission rejects more than 16 notes, notes over 48 KiB and notes
without a quote. Selection capture stays `Notes { command: capture_selection |
report_selection }`: the kernel observes the Browser/Room page selection itself
and returns the quote with its anchor; native panels report their own DOM range.

## Hidden context

`assemble_provider_turn` removes note attachments from the provider-bound list
and appends one `<user-notes>` block to the hidden system context, the same
assembly every provider, remote worker and native TUI uses (Claude native TUI via
the `UserPromptSubmit` `additionalContext` bridge). Each `<note>` has `Source`,
`Anchor`, `More` (the one tool above), the selected text and the comment. A note
whose `source.agent_id` is the receiving agent carries only quote and comment.
