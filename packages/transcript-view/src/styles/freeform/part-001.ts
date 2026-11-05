export const freeformStylesPart001 = `

.freeform-pane-grid-count-3 .freeform-agent-pane:nth-child(3),
.freeform-pane-grid-count-3 .freeform-agent-pending-pane:nth-child(3) {
  grid-column: 1 / -1;
}

.freeform-pane-grid-count-4 {
  grid-template-columns: repeat(2, minmax(0, 1fr));
  grid-template-rows: repeat(2, minmax(0, 1fr));
}

.freeform-pane-grid-count-5 {
  grid-template-columns: repeat(6, minmax(0, 1fr));
  grid-template-rows: minmax(0, 1fr) minmax(0, 0.9fr);
}

.freeform-pane-grid-count-5 .freeform-agent-pane,
.freeform-pane-grid-count-5 .freeform-agent-pending-pane {
  grid-column: span 2;
}

.freeform-pane-grid-count-5 .freeform-agent-pane:nth-child(4),
.freeform-pane-grid-count-5 .freeform-agent-pane:nth-child(5),
.freeform-pane-grid-count-5 .freeform-agent-pending-pane:nth-child(4),
.freeform-pane-grid-count-5 .freeform-agent-pending-pane:nth-child(5) {
  grid-column: span 3;
}

.freeform-pane-grid-count-6 {
  grid-template-columns: repeat(3, minmax(0, 1fr));
  grid-template-rows: repeat(2, minmax(0, 1fr));
}

.freeform-agent-pane,
.freeform-agent-pending-pane {
  display: grid;
  grid-template-rows: minmax(0, 1fr) auto auto auto;
  container-type: inline-size;
  height: 100%;
  min-width: 0;
  min-height: 0;
  overflow: clip;
  background:
    linear-gradient(180deg, color-mix(in srgb, var(--surface) 92%, transparent), color-mix(in srgb, var(--brand-black) 62%, var(--surface))),
    var(--surface);
}

.freeform-agent-pending-pane {
  background: var(--brand-black);
}

.freeform-agent-pane.focused {
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent-neutral) 72%, transparent);
}

.freeform-agent-pane.focused .freeform-pane-footer {
  border-top-color: transparent;
  box-shadow:
    inset 1px 0 0 color-mix(in srgb, var(--accent-neutral) 72%, transparent),
    inset -1px 0 0 color-mix(in srgb, var(--accent-neutral) 72%, transparent),
    inset 0 -1px 0 color-mix(in srgb, var(--accent-neutral) 72%, transparent);
}

.freeform-pane-body {
  position: relative;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  scrollbar-color: color-mix(in srgb, var(--accent-neutral) 38%, transparent) transparent;
  scrollbar-gutter: stable;
  scrollbar-width: thin;
}

.freeform-empty-mark {
  display: grid;
  min-height: 100%;
  place-content: center;
  justify-items: center;
  gap: 10px;
  padding: 18px;
  color: var(--muted);
  font-family: var(--font-display);
  pointer-events: none;
}

.freeform-empty-mark .terminal-logo {
  margin: 0;
  font-size: clamp(8px, 1.02vw, 15px);
}

.freeform-pane-grid-count-2 .freeform-empty-mark .terminal-logo,
.freeform-pane-grid-count-3 .freeform-empty-mark .terminal-logo {
  font-size: clamp(7px, 0.86vw, 12px);
}

.freeform-pane-grid-count-4 .freeform-empty-mark .terminal-logo,
.freeform-pane-grid-count-5 .freeform-empty-mark .terminal-logo,
.freeform-pane-grid-count-6 .freeform-empty-mark .terminal-logo {
  font-size: clamp(6px, 0.72vw, 10px);
}

.freeform-empty-mark > span {
  font-size: var(--text-sm);
}

.freeform-empty-spinner {
  display: none;
  width: 18px;
  height: 18px;
  border: 2px solid color-mix(in srgb, var(--muted) 34%, transparent);
  border-top-color: var(--accent-neutral);
  border-radius: 999px;
  animation: freeform-empty-spinner 800ms linear infinite;
}

.freeform-empty-mark[data-freeform-empty-state="creating-agent"] .freeform-empty-message {
  display: block;
}

.freeform-empty-mark[data-freeform-empty-state="creating-agent"] .freeform-empty-spinner,
.freeform-empty-mark[data-freeform-empty-state="history-loading"] .freeform-empty-spinner {
  display: block;
}

@keyframes freeform-empty-spinner {
  to {
    transform: rotate(360deg);
  }
}

.freeform-pane-body.has-output .freeform-empty-mark {
  display: none;
}

.freeform-transcript {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
  min-height: 0;
  margin: 0;
  padding: var(--space-1) 6px;
  color: var(--text);
  background: transparent;
  font-family: var(--font-display);
  font-size: var(--text-sm);
  line-height: 1.45;
  white-space: pre-wrap;
  word-break: break-word;
}

.freeform-live-output {
  --freeform-history-anchor-offset: 0;
  --freeform-output-frame-background: color-mix(in srgb, var(--surface-raised) 64%, transparent);
  --freeform-output-frame-border: 1px solid color-mix(in srgb, var(--line) 62%, transparent);
  --freeform-output-frame-padding: 3px;
  --freeform-output-frame-radius: 4px;
  display: flex;
  position: absolute;
  inset: 8px 0;
  box-sizing: border-box;
  flex-direction: column;
  gap: 3px;
  margin: 0;
  min-height: 0;
  overflow-x: hidden;
  overflow-y: auto;
  overflow-anchor: none;
  padding: 6px 14px var(--space-2);
  overscroll-behavior: none;
  color: var(--text);
  background: transparent;
  font-family: var(--font-display);
  font-size: var(--text-sm);
  line-height: 1.45;
  white-space: pre-wrap;
  word-break: break-word;
  overflow-wrap: anywhere;
  scrollbar-color: transparent transparent;
  scrollbar-width: thin;
  transition: scrollbar-color var(--motion-fast);
}

.freeform-live-output::-webkit-scrollbar {
  width: 4px;
  height: 4px;
}

.freeform-live-output::-webkit-scrollbar-track {
  background: transparent;
}

.freeform-live-output::-webkit-scrollbar-thumb {
  border-radius: var(--radius-full);
  background: transparent;
}

.freeform-live-output[data-freeform-scrollbar="user"] {
  scrollbar-color: color-mix(in srgb, var(--accent-neutral) 40%, transparent) transparent;
}

.freeform-live-output[data-freeform-scrollbar="user"]::-webkit-scrollbar-thumb {
  background: color-mix(in srgb, var(--accent-neutral) 38%, transparent);
}

.freeform-live-output[data-freeform-scrollbar="user"]::-webkit-scrollbar-thumb:hover {
  background: color-mix(in srgb, var(--accent-neutral) 56%, transparent);
}

.freeform-live-output::before {
  content: "";
  margin-top: auto;
}

.freeform-live-output > [data-render-key]:not([data-render-key="history-top-status"]) {
  transform: translateY(var(--freeform-history-anchor-offset, 0));
}

.freeform-history-page-button {
  align-self: center;
  border: 1px solid color-mix(in srgb, var(--line) 72%, transparent);
  border-radius: var(--radius-xs);
  padding: 2px var(--space-2);
  color: var(--muted);
  background: color-mix(in srgb, var(--surface-raised) 68%, transparent);
  font: inherit;
  font-size: var(--text-xs);
  line-height: 1.25;
  cursor: pointer;
}

.freeform-history-page-button:hover,
.freeform-history-page-button:focus-visible {
  color: var(--text);
  background: color-mix(in srgb, var(--surface-2) 74%, transparent);
  outline: none;
}

.freeform-history-page-button:disabled {
  cursor: default;
  opacity: 0.68;
}

.freeform-history-page-status {
  display: inline-flex;
  position: absolute;
  z-index: 2;
  top: var(--space-1);
  left: 50%;
  align-items: center;
  gap: var(--space-1);
  min-height: 24px;
  margin: 0;
  border: 1px solid color-mix(in srgb, var(--line) 64%, transparent);
  border-radius: var(--radius-xs);
  padding: 2px var(--space-2);
  color: var(--muted);
  background: color-mix(in srgb, var(--surface-raised) 62%, transparent);
  font-size: var(--text-xs);
  line-height: 1.25;
  transform: translateX(-50%);
}

.freeform-history-page-status[data-state="error"] {
  color: var(--danger);
  border-color: color-mix(in srgb, var(--danger) 44%, transparent);
}

.freeform-history-page-spinner {
  width: 12px;
  height: 12px;
  border: 2px solid color-mix(in srgb, var(--muted) 34%, transparent);
  border-top-color: var(--accent-neutral);
  border-radius: var(--radius-full);
  animation: freeform-empty-spinner 800ms linear infinite;
}

.freeform-message {
  align-self: stretch;
  width: 100%;
  min-width: 0;
  overflow-wrap: anywhere;
  margin: 0;
}

.freeform-message p {
  margin: 0;
  color: inherit;
  line-height: 1.22;
}

.freeform-turn {
  display: flex;
  flex-direction: column;
  gap: 0;
  width: 100%;
  min-width: 0;
}

.freeform-turn + .freeform-turn {
  margin-top: 6px;
}

.freeform-turn-body {
  display: flex;
  flex-direction: column;
  gap: 9px;
  margin: 9px 0;
  min-width: 0;
  max-height: var(--freeform-turn-body-height, none);
  overflow: hidden;
  opacity: 1;
  transform: translateY(0);
  transition:
    max-height var(--motion-med) cubic-bezier(0.22, 1, 0.36, 1),
    margin var(--motion-med) cubic-bezier(0.22, 1, 0.36, 1),
    opacity var(--motion-fast) ease,
    transform var(--motion-med) cubic-bezier(0.22, 1, 0.36, 1);
  will-change: max-height, margin, opacity, transform;
}

[data-terminal-output][data-freeform-history-prepend-pending="true"] .freeform-turn-body {
  transition: none;
}

.freeform-turn-body[hidden] {
  display: none;
}

.freeform-turn-body[data-empty="true"] {
  margin-top: 0;
  margin-bottom: 0;
}

.freeform-turn-body[data-collapsed="true"] {
  max-height: 0;
  margin-top: 0;
  margin-bottom: 0;
  opacity: 0;
  pointer-events: none;
  transform: translateY(-5px);
}

.freeform-turn-meta-row {
  display: flex;
  width: 100%;
  min-width: 0;
  align-items: center;
  gap: 7px;
  padding: 2px 0 var(--space-1);
  color: var(--muted);
  font-size: var(--text-xs);
  line-height: 1.3;
}

.freeform-turn-meta {
  display: inline-flex;
  flex: 0 1 auto;
  min-width: 0;
  align-items: center;
  gap: 7px;
}

.freeform-turn-meta svg {
  width: 12px;
  height: 12px;
}

.freeform-turn-meta-main {
  display: inline-flex;
  position: relative;
  min-width: 0;
  align-items: center;
  gap: 8px;
}

.freeform-turn-meta-tooltip {
  left: 0;
  max-width: min(100%, 460px);
  background: var(--surface-raised);
  color: var(--text);
  box-shadow: 0 8px 18px rgb(0 0 0 / 44%);
  transform: none;
  white-space: normal;
  overflow-wrap: anywhere;
}

.freeform-turn-completion-row .freeform-turn-meta-main {
  flex-wrap: nowrap;
}

.freeform-turn-meta-text {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.freeform-turn-external-marker {
  display: inline-flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 3px;
  color: inherit;
  font-weight: 400;
}

.freeform-turn-external-info {
  display: inline-grid;
  width: 13px;
  height: 13px;
  place-items: center;
  color: var(--muted);
  outline: none;
}

.freeform-turn-external-info svg {
  width: 12px;
  height: 12px;
}

.freeform-turn-chevron-button {
  display: inline-grid;
  width: 22px;
  height: 22px;
  margin-left: auto;
  flex: 0 0 auto;
  place-items: center;
  border: 0;
  padding: 0;
  color: var(--muted);
  background: transparent;
  cursor: pointer;
}

.freeform-turn-chevron-button:disabled {
  cursor: default;
}

.freeform-turn-chevron-button:hover,
.freeform-turn-chevron-button:focus-visible {
  color: var(--text);
}

.freeform-turn-chevron-button:focus-visible {
  outline: 1px solid color-mix(in srgb, var(--line-strong) 84%, transparent);
  outline-offset: 2px;
}

.freeform-turn-chevron {
  display: inline-grid;
  width: 16px;
  height: 16px;
  place-items: center;
  color: var(--text);
}

.freeform-turn-chevron svg {
  transition: transform 120ms ease;
}

.freeform-turn-chevron-button[aria-expanded="true"] .freeform-turn-chevron svg {
  transform: rotate(90deg);
}

.freeform-turn-summary {
  margin: 6px 0 0;
  border-left: 0;
  padding-left: 0;
  color: var(--text);
  line-height: 1.22;
}

.freeform-turn-summary p {
  margin: 0;
  color: var(--text);
  line-height: 1.22;
}

.freeform-turn-summary[hidden] {
  display: none;
}

.freeform-user-prompt {
  display: flow-root;
  flex: 0 0 auto;
  justify-self: stretch;
  align-self: stretch;
  align-content: start;
  box-sizing: border-box;
  width: 100%;
  max-width: none;
  height: auto;
  min-block-size: 0;
  border: var(--freeform-output-frame-border);
  border-radius: var(--freeform-output-frame-radius);
  padding: var(--freeform-output-frame-padding);
  color: var(--text);
  background: color-mix(in srgb, var(--surface-raised) 86%, var(--brand-white) 8%);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--line-strong) 52%, transparent);
  line-height: 1.22;
  white-space: normal;
}

.freeform-user-prompt p {
  display: block;
  margin: 0;
  min-height: 0;
  color: inherit;
  line-height: 1.22;
  white-space: pre-wrap;
}

.freeform-agent-message-header {
  margin: 0 0 6px;
  color: var(--subtle);
  font-size: 0.78em;
  line-height: 1.2;
}

.freeform-agent-message-header strong {
  color: var(--text);
  font-weight: 600;
}

.freeform-agent-output {
  display: block;
  padding: 0;
  color: var(--text);
  line-height: 1.22;
}

.freeform-agent-output code,
.freeform-user-prompt code {
  border-radius: var(--radius-sm);
  padding: 0 3px;
  color: color-mix(in srgb, var(--accent-neutral) 70%, var(--text));
  background: color-mix(in srgb, var(--surface-raised) 74%, transparent);
  font: inherit;
  overflow-wrap: anywhere;
}

.freeform-code-reference {
  display: inline;
  border-radius: var(--radius-sm);
  margin: 0 1px;
  padding: 0 3px;
  color: color-mix(in srgb, var(--accent-neutral) 70%, var(--text));
  background: color-mix(in srgb, var(--surface-raised) 74%, transparent);
  box-decoration-break: clone;
  -webkit-box-decoration-break: clone;
  font: inherit;
  text-decoration: underline;
  text-decoration-color: color-mix(in srgb, var(--accent-neutral) 48%, transparent);
  text-underline-offset: 2px;
  overflow-wrap: anywhere;
  cursor: default;
}

.freeform-code-reference:focus-visible {
  outline: 1px solid color-mix(in srgb, var(--accent-neutral) 58%, transparent);
  outline-offset: 2px;
}

.freeform-agent-output strong,
.freeform-user-prompt strong {
  color: var(--text);
  font-weight: 700;
}

.freeform-reasoning-output {
  color: var(--muted);
}

.freeform-status-output {
  color: var(--subtle);
}

.freeform-error-output {
  color: var(--color-danger);
}

.freeform-collapsible-output {
  white-space: normal;
}

.freeform-blob-header {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) 16px;
  width: 100%;
  min-width: 0;
  align-items: baseline;
  gap: 7px;
  border: 0;
  padding: 0;
  color: inherit;
  background: transparent;
  font: inherit;
  line-height: 1.22;
  text-align: left;
  cursor: pointer;
}

.freeform-blob-header:hover .freeform-blob-title,
.freeform-blob-header:focus-visible .freeform-blob-title {
  color: var(--text);
}

.freeform-blob-header:focus-visible {
  outline: 1px solid color-mix(in srgb, var(--line-strong) 84%, transparent);
  outline-offset: 2px;
}

.freeform-blob-title {
  min-width: 0;
  overflow: hidden;
  color: var(--text);
  font-weight: 700;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.freeform-blob-header[data-tool-icon] {
  grid-template-columns: 14px auto minmax(0, 1fr) 16px;
}

.freeform-blob-icon {
  display: inline-grid;
  width: 14px;
  height: 14px;
  align-self: center;
  place-items: center;
  color: var(--subtle);
}

.freeform-blob-icon svg {
  width: 14px;
  height: 14px;
}

.freeform-tool-chip .freeform-blob-icon {
  margin-right: 6px;
  vertical-align: -2px;
}

/* A running call or live thought: a bright band moves across the title. */
.freeform-blob-title[data-running="true"] {
  background: linear-gradient(90deg, var(--subtle) 0%, var(--subtle) 35%, var(--text) 50%, var(--subtle) 65%, var(--subtle) 100%);
  background-size: 250% 100%;
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
  animation: freeform-title-shimmer 1.8s linear infinite;
}

@keyframes freeform-title-shimmer {
  from {
    background-position: 100% 0;
  }
  to {
    background-position: -150% 0;
  }
}

@media (prefers-reduced-motion: reduce) {
  .freeform-blob-title[data-running="true"] {
    animation: none;
    background: none;
    color: var(--subtle);
  }
}

.freeform-blob-action {
  min-width: 0;
  overflow: hidden;
  color: var(--subtle);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.freeform-blob-chevron {
  display: inline-grid;
  width: 16px;
  height: 16px;
  place-items: center;
  color: var(--text);
}

.freeform-blob-chevron svg {
  width: 12px;
  height: 12px;
  transition: transform 120ms ease;
}

.freeform-blob-header[aria-expanded="true"] .freeform-blob-chevron svg {
  transform: rotate(90deg);
}

.freeform-blob-body {
  margin-top: 9px;
}

.freeform-blob-body[hidden] {
  display: none;
}

.freeform-interaction-output {
  white-space: normal;
  border: var(--freeform-output-frame-border);
  border-radius: var(--freeform-output-frame-radius);
  padding: var(--freeform-output-frame-padding);
  background: color-mix(in srgb, var(--surface-raised) 74%, transparent);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--line-strong) 38%, transparent);
}

.freeform-interaction-output[data-level="warning"] {
  border-color: color-mix(in srgb, var(--accent-neutral) 48%, var(--line));
}

.freeform-interaction-output[data-level="critical"] {
  border-color: color-mix(in srgb, var(--color-danger) 58%, var(--line));
}

.freeform-interaction-output:focus-visible {
  outline: 1px solid color-mix(in srgb, var(--line-strong) 86%, transparent);
  outline-offset: 2px;
}

.freeform-interaction-header {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  align-items: baseline;
  gap: 7px;
  min-width: 0;
}

.freeform-interaction-message {
  margin: 3px 0 0;
  color: var(--text);
  line-height: 1.22;
  white-space: pre-wrap;
}

.freeform-interaction-error {
  grid-column: 1 / -1;
  margin: 4px 0 0;
  color: var(--danger);
  line-height: 1.22;
}

.freeform-interaction-choices {
  display: flex;
  flex-direction: column;
  flex-wrap: nowrap;
  align-items: flex-start;
  gap: 5px;
  margin-top: 5px;
  min-width: 0;
}

.freeform-interaction-choice {
  display: inline-grid;
  grid-template-columns: 10px auto minmax(0, auto);
  align-items: baseline;
  gap: var(--space-1);
  min-width: 0;
  max-width: 100%;
  width: fit-content;
  border: 1px solid color-mix(in srgb, var(--line-strong) 54%, transparent);
  border-radius: var(--radius-xs);
  padding: 2px 7px;
  color: var(--text);
  background: color-mix(in srgb, var(--surface) 78%, transparent);
  font: inherit;
  line-height: 1.2;
  cursor: pointer;
}

.freeform-interaction-choice:hover,
.freeform-interaction-choice:focus-visible,
.freeform-interaction-choice[aria-selected="true"] {
  border-color: color-mix(in srgb, var(--accent-neutral) 58%, var(--line-strong));
  color: var(--text);
  background: color-mix(in srgb, var(--surface-raised) 88%, transparent);
}

.freeform-interaction-choice[data-style="secondary"] {
  color: var(--text);
}

.freeform-interaction-choice[data-style="danger"] {
  color: var(--text);
}

.freeform-interaction-choice:disabled {
  cursor: wait;
  opacity: 0.62;
}

.freeform-interaction-choice-marker,
.freeform-interaction-choice-number {
  color: inherit;
}

.freeform-interaction-choice-label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.freeform-interaction-custom-choice {
  display: grid;
  grid-template-columns: minmax(7rem, max-content) minmax(10rem, 1fr) max-content;
  gap: 6px;
  align-items: center;
  min-width: min(100%, 34rem);
}

.freeform-interaction-custom-note {
  margin: 2px 0 0;
  color: var(--muted);
  line-height: 1.22;
}

.freeform-interaction-custom-input {
  min-width: 0;
  height: 1.85rem;
  border: 1px solid color-mix(in srgb, var(--line-strong) 54%, transparent);
  border-radius: var(--radius-xs);
  padding: 0 0.55rem;
  background: color-mix(in srgb, var(--surface) 88%, transparent);
  color: var(--text);
  font: inherit;
}

.freeform-interaction-custom-input:focus {
  outline: 2px solid color-mix(in srgb, var(--accent-neutral) 72%, transparent);
  outline-offset: 1px;
}

.freeform-interaction-custom-submit {
  height: 1.85rem;
  border: 1px solid color-mix(in srgb, var(--line-strong) 54%, transparent);
  border-radius: var(--radius-xs);
  padding: 0 0.65rem;
  background: color-mix(in srgb, var(--surface-raised) 88%, transparent);
  color: var(--text);
  font: inherit;`
