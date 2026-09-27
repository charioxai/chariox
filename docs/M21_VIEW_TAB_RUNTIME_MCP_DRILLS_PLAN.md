# M21 View Tab Runtime MCP Drills Plan

## Goal

Validate from the Chariox Cloud web terminal View tab that a slice-backed agent can discover and use Chariox runtime MCP tools to operate the slice browser and screen on behalf of the user.

The drills must be run from the web terminal View tab, with the slice screen,
prompt area, and agent pane or traces visible. Store screenshots under
`/Users/miguel/.codex/evidence/browser-computer-use/m21-view-runtime-mcp/`, never
inside a repository.

## Runtime MCP Surfaces Under Test

- `slice_screen_status`
- `slice_screenshot`
- `slice_ocr`
- `slice_find_text`
- `slice_mouse`
- `slice_keyboard`
- `slice_open_url`
- `slice_browser_status`
- `slice_browser_find`
- `slice_browser_fill`
- `slice_browser_click`
- `slice_browser_submit`
- `slice_browser_dialog`
- `slice_browser_events`
- `slice_browser_downloads`
- `slice_browser_upload`
- `slice_browser_permission`
- `slice_browser_text`
- `slice_browser_wait_for_text`
- `slice_browser_wait_for_selector`
- `slice_browser_wait_for_idle`
- `paste_secret_to_slice`

## Drill 1: Browser And Screen Discovery

Purpose: validate that the agent can inspect the slice display, open a site, read browser state, and use OCR/screenshot tools.

Prompt:

```text
You are running in a Chariox slice. Use the Chariox runtime MCP slice tools to inspect the slice screen, open Gmail in the slice browser, confirm whether the inbox is accessible, and summarize the visible inbox state.

Use browser DOM tools first. Also take a slice screenshot and use OCR or find-text once to verify visible screen text.
```

Expected tool usage:

- `slice_screen_status`
- `slice_open_url`
- `slice_browser_status`
- `slice_browser_text`
- `slice_screenshot`
- `slice_ocr` or `slice_find_text`

Required screenshots:

- `m21-drill1-view-tab-start.png`
- `m21-drill1-agent-tool-traces.png`
- `m21-drill1-gmail-visible.png`

Pass criteria:

- The View tab screenshot shows the slice screen, prompt area, and agent pane.
- The agent trace shows runtime MCP tool calls.
- Gmail or the relevant browser state is visible in the slice screen.
- The agent reports the inbox/browser state based on tool output, not manual operator inspection.

## Drill 2: Browser Task Execution Through Runtime MCP

Purpose: validate that the agent can perform real browser work through the exposed runtime MCP tools.

Prompt:

```text
Use the runtime MCP tools available to you to drive the browser inside the Chariox slice. Do not rely on provider-native browser tools or manual operator actions.

Open Gmail, compose an email to ma.gutierrez.estevez@gmail.com with subject "Chariox slice runtime MCP drill" and body "This email was sent by a Chariox agent driving Gmail inside a slice through runtime MCP tools from the web terminal View tab.", send it, and then verify from the browser state that the action completed.

Use browser DOM tools first. If DOM tools are insufficient, fall back to slice OCR/find-text plus mouse/keyboard tools and explain why.
```

Expected tool usage:

- `slice_browser_status`
- `slice_browser_find`
- `slice_browser_click`
- `slice_browser_fill`
- `slice_browser_text`
- Optional fallback: `slice_find_text`, `slice_mouse`, `slice_keyboard`, `slice_ocr`

Required screenshots:

- `m21-drill2-compose-trace.png`
- `m21-drill2-email-sent.png`
- `m21-drill2-agent-pane-final.png`

Pass criteria:

- The agent uses Chariox runtime MCP browser/screen tools to drive Gmail.
- The agent pane trace clearly shows the tool calls and results.
- The View tab screenshot shows the sent/confirmation state or equivalent Gmail evidence.
- No manual browser interaction is used for the task, except web terminal prompting.

## Optional Drill 3: Secret Paste Redaction

Purpose: validate that the vault-to-browser path can place a secret into a browser field without exposing the value in agent output.

Prompt:

```text
Use the runtime MCP tools available to you to open a simple password-field test page in the slice browser, identify the password field, and use paste_secret_to_slice with a Chariox vault credential. Do not print, reveal, or infer the secret value. Verify only that the field is filled/redacted.
```

Expected tool usage:

- `slice_open_url`
- `slice_browser_find`
- `paste_secret_to_slice`
- `slice_screenshot`

Required screenshots:

- `m21-drill3-secret-redacted.png`
- `m21-drill3-agent-trace-redaction.png`

Pass criteria:

- The secret value never appears in the agent trace, browser text output, or screenshots.
- The browser field shows redacted password-style UI or equivalent proof of filled state.

## Trace Review

After each drill, review the agent pane traces and record:

- Which runtime MCP tools the agent discovered and used.
- Whether the agent chose browser DOM tools before screen/OCR/mouse fallbacks.
- Whether the agent used tool results correctly or became confused by selectors, field IDs, OCR text, screenshots, or browser state.
- Whether the trace exposed any secret, credential, or sensitive browser state unexpectedly.
- Whether the agent needed extra prompting to find or use the correct tool.

## Final Evaluation Report

At the end, produce a short evaluation report covering:

- Overall pass/fail for each drill.
- Evidence screenshot filenames.
- Tool discovery friction.
- Tool naming or instruction confusion.
- Browser DOM reliability issues.
- OCR/mouse/keyboard fallback quality.
- Secret redaction behavior, if Drill 3 runs.
- Proposed product improvements, limited to findings observed in the traces.

## Cleanup

- Keep screenshots in the external evidence directory named above.
- Do not destroy the slice unless explicitly instructed.
- Clean up temporary local test pages or files created only for the optional secret drill.
