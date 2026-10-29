export const freeformStylesPart002 = `
  cursor: pointer;
}

.freeform-interaction-custom-submit:disabled {
  cursor: default;
  opacity: 0.45;
}

.freeform-agent-output header {
  display: none;
  min-width: 0;
  align-items: center;
  gap: 7px;
  overflow: hidden;
  color: var(--subtle);
  font-size: var(--text-2xs);
  line-height: 1.2;
  white-space: nowrap;
}

.freeform-tool-output {
  width: 100%;
  border-left: 0;
  padding: 0;
  white-space: normal;
}

.freeform-tool-output header {
  display: flex;
  margin-bottom: 0;
}

.freeform-agent-output header span {
  overflow: hidden;
  text-overflow: ellipsis;
}

.freeform-tool-chip {
  flex: 0 0 auto;
  border: 0;
  border-radius: 0;
  padding: 0;
  color: var(--text);
  background: transparent;
  font-weight: 700;
  text-transform: none;
}

.freeform-tool-status {
  color: var(--muted);
  text-transform: uppercase;
}

.freeform-tool-body {
  display: block;
}

.freeform-tool-body > * {
  margin: 0;
}

.freeform-tool-body > * + * {
  margin-top: 0;
}

.freeform-tool-body p {
  margin: 0;
  white-space: pre-wrap;
}

.freeform-tool-code,
.freeform-terminal-code,
.freeform-code-block {
  align-self: flex-start;
  width: max-content;
  max-width: 100%;
  min-width: 0;
  margin: 0;
}

.freeform-tool-code figcaption,
.freeform-terminal-code figcaption,
.freeform-code-block figcaption {
  margin: 0;
  color: var(--subtle);
  font-size: var(--text-2xs);
  line-height: 1.22;
}

.freeform-tool-code pre,
.freeform-terminal-code pre,
.freeform-code-block pre {
  margin: 0;
  overflow-x: auto;
  overflow-y: hidden;
  border: var(--freeform-output-frame-border);
  border-radius: var(--freeform-output-frame-radius);
  padding: var(--freeform-output-frame-padding);
  color: var(--text);
  background: var(--freeform-output-frame-background);
  font: inherit;
  line-height: 1.22;
  scrollbar-width: none;
  white-space: pre;
}

.freeform-terminal-code {
  width: max-content;
  max-width: 100%;
  position: relative;
  border: var(--freeform-output-frame-border);
  border-radius: var(--freeform-output-frame-radius);
  background: color-mix(in srgb, var(--brand-black) 88%, var(--surface) 12%);
  overflow: hidden;
}

.freeform-terminal-code figcaption {
  padding: 2px 4px 0;
  color: color-mix(in srgb, var(--accent-neutral) 64%, var(--text));
  background: transparent;
  text-align: right;
  text-transform: lowercase;
}

.freeform-terminal-code pre {
  border: 0;
  border-radius: 0;
  padding: 2px 4px;
  background: transparent;
  scrollbar-width: thin;
}

.freeform-tool-code code,
.freeform-terminal-code code,
.freeform-code-block code {
  border-radius: 0;
  padding: 0;
  color: inherit;
  background: transparent;
}

.freeform-tool-code pre::-webkit-scrollbar,
.freeform-code-block pre::-webkit-scrollbar {
  display: none;
  width: 0;
  height: 0;
}

.freeform-terminal-code pre::-webkit-scrollbar {
  height: 6px;
}

.freeform-terminal-code pre::-webkit-scrollbar-thumb {
  border-radius: 999px;
  background: color-mix(in srgb, var(--subtle) 46%, transparent);
}

.freeform-tool-patch {
  display: block;
}

.freeform-tool-table {
  width: 100%;
  margin: 0;
  border-collapse: collapse;
  color: var(--text);
}

.freeform-tool-table th,
.freeform-tool-table td {
  border-bottom: 1px solid color-mix(in srgb, var(--line) 52%, transparent);
  padding: 2px var(--space-1);
  text-align: left;
}

.freeform-pane-footer {
  display: grid;
  grid-template-columns: auto auto minmax(0, 1fr) auto;
  align-items: center;
  gap: var(--space-2);
  min-width: 0;
  min-height: 30px;
  border-top: 1px solid color-mix(in srgb, var(--line) 76%, transparent);
  padding: 0 var(--space-2);
  color: var(--muted);
  background: color-mix(in srgb, var(--footer-surface-strong) 86%, transparent);
  font-family: var(--font-display);
  font-size: var(--text-2xs);
}

.freeform-agent-footer-leading {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  width: auto;
  max-width: 100%;
  min-width: min(8ch, 100%);
  justify-self: start;
  overflow: visible;
}

.freeform-footer-center {
  display: inline-flex;
  width: 100%;
  min-width: 0;
  max-width: 100%;
  align-items: center;
  justify-content: center;
  justify-self: center;
  gap: var(--space-2);
  overflow: visible;
}

.freeform-metaagent-task-strip {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: center;
  gap: var(--space-2);
  min-width: 0;
  border-top: 1px solid color-mix(in srgb, var(--line) 58%, transparent);
  padding: 4px var(--space-2);
  background: color-mix(in srgb, var(--footer-surface-strong) 78%, transparent);
  font-family: var(--font-display);
  font-size: var(--text-2xs);
}

.freeform-metaagent-task-actions {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

/* MP-08 / MP-09 / MP-10 / MP-11 A12: the goal row keeps its state readable. */
[data-freeform-root-workflow-task][data-goal-revision] {
  grid-template-columns: minmax(0, 1fr);
}
[data-goal-revision] [data-goal-status] {
  grid-column: 1 / -1;
  white-space: pre-line;
  overflow-wrap: anywhere;
  color: var(--muted);
}
[data-goal-revision] [data-goal-status] [data-react-text-region] {
  white-space: inherit;
  color: inherit;
}
[data-goal-revision] .freeform-metaagent-task-actions {
  flex-wrap: wrap;
}
[data-goal-interval] { width: 6ch; }

.freeform-agent-prompt-schedule-countdown {
  min-width: 5ch;
  color: var(--muted);
  font-variant-numeric: tabular-nums;
  text-align: right;
  white-space: nowrap;
}

.freeform-metaagent-task-actions .ds-button {
  min-height: 20px;
  padding: 1px 6px;
  font: inherit;
}

.freeform-metaagent-task-actions .freeform-task-lifecycle-button {
  position: relative;
  width: 22px;
  min-width: 22px;
  height: 22px;
  min-height: 22px;
  border-radius: 50%;
  padding: 0;
}

.freeform-task-lifecycle-button::before,
.freeform-task-lifecycle-button::after {
  position: absolute;
  inset: 50% auto auto 50%;
  content: "";
  transform: translate(-50%, -50%);
}

.freeform-task-lifecycle-button[data-task-lifecycle-action="pause"]::before {
  width: 2px;
  height: 8px;
  border-radius: 1px;
  background: currentColor;
  box-shadow: 4px 0 0 currentColor;
  transform: translate(-3px, -50%);
}

.freeform-task-lifecycle-button[data-task-lifecycle-action="play"]::before {
  width: 0;
  height: 0;
  border-top: 5px solid transparent;
  border-bottom: 5px solid transparent;
  border-left: 7px solid currentColor;
  transform: translate(-35%, -50%);
}

.freeform-task-lifecycle-button[data-task-lifecycle-action="stop"]::before {
  width: 7px;
  height: 7px;
  border-radius: 1px;
  background: currentColor;
}

.freeform-queued-prompts {
  --queued-prompt-row-height: 24px;
  min-width: 0;
  border-top: 1px solid color-mix(in srgb, var(--line) 58%, transparent);
  background: color-mix(in srgb, var(--footer-surface-strong) 82%, transparent);
  font-family: var(--font-display);
  font-size: var(--text-2xs);
}

.freeform-queued-prompt-list {
  display: grid;
  gap: 1px;
  min-width: 0;
  max-height: calc((var(--queued-prompt-row-height) * 3) + 2px + 8px);
  overflow-y: auto;
  scrollbar-gutter: stable;
  padding: 4px var(--space-2);
}

.freeform-queued-prompt {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto auto;
  align-items: center;
  gap: var(--space-2);
  min-height: var(--queued-prompt-row-height);
  min-width: 0;
}

.freeform-queued-prompt-state {
  display: inline-grid;
  grid-template-columns: auto auto;
  align-items: center;
  gap: 5px;
  min-width: 0;
  color: var(--subtle);
  white-space: nowrap;
}

.freeform-queued-prompt-state svg {
  color: color-mix(in srgb, var(--subtle) 86%, var(--text));
  flex: none;
}

.freeform-queued-prompt-text {
  display: block;
  width: 100%;
  min-width: 0;
  border: 0;
  padding: 0;
  overflow: hidden;
  background: transparent;
  color: var(--text);
  font: inherit;
  text-align: left;
  text-overflow: ellipsis;
  white-space: nowrap;
  cursor: pointer;
}

.freeform-queued-prompt-text:hover,
.freeform-queued-prompt-text:focus-visible {
  color: var(--accent-neutral);
  outline: none;
}

.freeform-queued-prompt-text:focus-visible {
  text-decoration: underline;
  text-underline-offset: 3px;
}

.freeform-queued-prompt-attachments {
  color: var(--subtle);
  white-space: nowrap;
}

.freeform-queued-prompt-actions {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.freeform-queued-prompt-protected {
  color: var(--subtle);
  font-size: 0.72rem;
  font-weight: 600;
  white-space: nowrap;
}

.freeform-queued-prompt-actions button {
  min-height: 20px;
  padding: 1px 7px;
  font-size: var(--text-2xs);
}

.freeform-queued-prompt-editor-backdrop {
  position: fixed;
  inset: 0;
  z-index: 80;
  display: grid;
  place-items: center;
  padding: var(--space-4);
  background: color-mix(in srgb, var(--surface) 54%, transparent);
}

.freeform-queued-prompt-editor {
  display: grid;
  gap: var(--space-3);
  width: min(640px, 100%);
  border: 1px solid var(--line-strong);
  border-radius: var(--radius-sm);
  padding: var(--space-3);
  background: var(--surface-raised);
  box-shadow: var(--shadow-lg);
  font-family: var(--font-display);
}

.freeform-queued-prompt-editor textarea {
  min-height: 180px;
  resize: vertical;
  border: 1px solid var(--line-strong);
  border-radius: var(--radius-xs);
  padding: var(--space-2);
  background: var(--surface);
  color: var(--text);
  font: inherit;
  line-height: 1.35;
}

.freeform-queued-prompt-editor-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-2);
}

.freeform-metaagent-task-open {
  display: inline-grid;
  grid-template-columns: auto minmax(0, 1fr);
  align-items: center;
  gap: 6px;
  min-width: 0;
  border: 0;
  padding: 0;
  background: transparent;
  color: var(--text);
  font: inherit;
  text-align: left;
}

.freeform-metaagent-task-open span {
  color: var(--text);
  white-space: nowrap;
}

.freeform-metaagent-task-open em {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--muted);
  font-style: normal;
}

.freeform-status-badge {
  display: inline-flex;
  position: relative;
  min-width: 58px;
  min-height: 18px;
  align-items: center;
  justify-content: center;
  border: 1px solid var(--line);
  border-radius: var(--radius-xs);
  padding: 0 6px;
  color: var(--muted);
  overflow: hidden;
  transition: color var(--motion-fast), background var(--motion-fast), border-color var(--motion-fast), box-shadow var(--motion-fast);
}

.freeform-status-badge[data-tone="working"] {
  border-color: color-mix(in srgb, var(--accent-neutral) 62%, var(--line));
  color: var(--accent-neutral);
  background: color-mix(in srgb, var(--accent-neutral) 6%, transparent);
  box-shadow: 0 0 8px color-mix(in srgb, var(--accent-neutral) 10%, transparent);
}


.freeform-status-badge[data-tone="ready"] {
  border-color: var(--color-success-border);
  color: var(--color-success);
  background: var(--color-success-bg);
}

.freeform-status-badge[data-tone="idle"] {
  border-color: var(--color-success-dim);
  color: color-mix(in srgb, var(--color-success) 70%, var(--muted));
}

.freeform-status-badge[data-tone="done"] {
  border-color: var(--color-done-border);
  color: var(--color-done);
  background: var(--color-done-bg);
  box-shadow: 0 0 8px color-mix(in srgb, var(--color-done) 12%, transparent);
}

.freeform-status-badge[data-tone="error"] {
  border-color: var(--color-danger-border);
  color: var(--color-danger);
  background: var(--color-danger-bg);
}

.freeform-pane-footer.compact-status .freeform-status-badge {
  min-width: 0;
  width: 24px;
  padding: 0;
  gap: 0;
  font-size: 0;
}

.freeform-pane-footer.compact-status .freeform-status-badge::after {
  content: attr(data-short-status);
  position: absolute;
  inset: 0;
  display: grid;
  place-items: center;
  font-size: var(--text-2xs);
  line-height: 1;
}

.freeform-agent-identity {
  display: inline-flex;
  position: relative;
  flex: 0 1 auto;
  align-items: center;
  gap: 7px;
  width: auto;
  max-width: 100%;
  min-width: 0;
  overflow: visible;
}

.freeform-runtime-placement {
  display: inline-flex;
  position: relative;
  flex: 0 1 max-content;
  align-items: center;
  justify-content: center;
  gap: 6px;
  width: max-content;
  inline-size: max-content;
  max-width: 100%;
  min-width: 28px;
  min-height: 18px;
  overflow: visible;
  border: 1px solid color-mix(in srgb, var(--color-success) 18%, transparent);
  border-radius: var(--radius-full);
  padding: 1px 8px 1px 6px;
  color: var(--muted);
  background: color-mix(in srgb, var(--color-success) 8%, transparent);
  font: inherit;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.freeform-runtime-placement-tooltip {
  max-width: min(420px, 72vw);
}

.freeform-runtime-placement::before {
  content: "";
  flex: 0 0 6px;
  width: 6px;
  height: 6px;
  border-radius: var(--radius-full);
  background: var(--color-success);
}

.freeform-runtime-placement-label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.freeform-runtime-placement[data-freeform-runtime-placement-status="disconnected"],
.freeform-pane-footer:has(.freeform-status-badge[data-tone="error"]) .freeform-runtime-placement {
  border-color: color-mix(in srgb, var(--color-danger) 18%, transparent);
  color: var(--color-danger);
  background: color-mix(in srgb, var(--color-danger) 8%, transparent);
}

.freeform-runtime-placement[data-freeform-runtime-placement-status="disconnected"]::before,
.freeform-pane-footer:has(.freeform-status-badge[data-tone="error"]) .freeform-runtime-placement::before {
  background: var(--color-danger);
}

.freeform-runtime-placement[hidden] {
  display: none;
}

.freeform-slice-screen-link,
.ds-badge.freeform-slice-screen-link {
  justify-self: center;
  flex: 0 1 max-content;
  width: max-content;
  inline-size: max-content;
  max-width: 100%;
  min-width: 0;
  overflow: hidden;
  border-color: var(--line);
  color: var(--muted);
  background: color-mix(in srgb, var(--surface-raised) 44%, transparent);
  text-overflow: ellipsis;
  text-decoration: none;
  white-space: nowrap;
}

.freeform-slice-screen-link:hover,
.ds-badge.freeform-slice-screen-link:hover {
  border-color: var(--line-strong);
  color: var(--text);
  background: color-mix(in srgb, var(--surface-raised) 58%, transparent);
}

.freeform-slice-actions {
  display: grid;
  min-width: 0;
  gap: 1px;
}

.freeform-slice-actions:empty {
  display: none;
}

.freeform-slice-action {
  display: flex;
  min-height: 21px;
  max-width: none;
  align-items: center;
  justify-content: flex-start;
  border: 1px solid transparent;
  border-radius: var(--radius-xs);
  padding: 0 6px;
  color: var(--muted);
  background: transparent;
  font: inherit;
  line-height: 1;
  text-overflow: ellipsis;
  white-space: nowrap;
  overflow: hidden;
  cursor: pointer;
}

.freeform-slice-action:hover,
.freeform-slice-action:focus-visible {
  border-color: color-mix(in srgb, var(--accent-neutral) 68%, var(--line));
  color: var(--text);
  outline: none;
}

.freeform-slice-action:disabled {
  cursor: default;
  opacity: 0.56;
}

.freeform-agent-alias,
.freeform-agent-alias-input {
  min-width: 0;
  border: 1px solid transparent;
  border-radius: var(--radius-xs);
  color: var(--text);
  background: transparent;
  font: inherit;
}

.freeform-agent-alias {
  position: relative;
  max-width: min(160px, 26vw);
  overflow: visible;
  flex: 999 1 auto;
  padding: 2px 0;
  text-align: left;
}

.freeform-agent-alias [data-freeform-alias-display] {
  display: block;
  min-width: 0;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.freeform-agent-alias-tooltip {
  max-width: min(360px, 66vw);
}

.freeform-pane-footer .ds-tooltip {
  visibility: hidden;
}

.freeform-pane-footer .ds-tooltip-host:hover .ds-tooltip,
.freeform-pane-footer .ds-tooltip-host:focus-within .ds-tooltip {
  visibility: visible;
}

.freeform-pane-footer.compact-placement .freeform-runtime-placement {
  max-width: 9ch;
  min-width: 4ch;
}

.freeform-pane-footer.compact-agent-mild .freeform-agent-alias {
  max-width: 14ch;
  flex: 1 1 auto;
}

.freeform-pane-footer.compact-agent .freeform-agent-alias {
  max-width: 4ch;
  flex: 1 1 auto;
}

.freeform-agent-alias-input {
  box-sizing: border-box;
  display: none;
  padding: 2px 5px;
  outline: none;
}

.freeform-agent-alias-input[data-editing="true"] {
  display: inline-block;
  border-color: color-mix(in srgb, var(--accent-neutral) 62%, var(--line));
  background: color-mix(in srgb, var(--surface) 86%, transparent);
}

.freeform-agent-alias[data-editing="true"] {
  display: none;
}

.freeform-turn-clock {
  display: inline-flex;
  position: absolute;
  left: 8px;
  bottom: 8px;
  z-index: 4;
  justify-content: flex-end;
  min-width: 0;
  border: 0;
  padding: 0;
  color: var(--subtle);
  background: transparent;
  font-size: var(--text-sm);
  font-variant-numeric: tabular-nums;
  line-height: 1.22;
  visibility: hidden;
  white-space: nowrap;
}

.freeform-turn-clock[data-active="true"] {
  visibility: visible;
}

.freeform-turn-clock[hidden] {
  display: none;
  visibility: hidden;
}

.freeform-prompt-jump-controls {
  display: grid;
  position: absolute;
  right: 8px;
  bottom: 7px;
  z-index: 5;
  gap: 4px;
  pointer-events: none;
}

.freeform-prompt-jump-controls[hidden] {
  display: none;
}

.freeform-pane-body:not(.has-output) .freeform-prompt-jump-controls {
  display: none;
}

.freeform-prompt-jump-button {
  display: inline-flex;
  width: 22px;
  height: 22px;
  min-height: 22px;
  align-items: center;
  justify-content: center;
  border: 1px solid color-mix(in srgb, var(--line) 68%, transparent);
  border-radius: var(--radius-full);`
