export const freeformStylesPart003 = `
  padding: 0;
  color: var(--muted);
  background: color-mix(in srgb, var(--surface-raised) 72%, transparent);
  pointer-events: auto;
  cursor: pointer;
}

.freeform-prompt-jump-button:hover:not(:disabled),
.freeform-prompt-jump-button:focus-visible {
  border-color: color-mix(in srgb, var(--accent-neutral) 48%, var(--line));
  color: var(--text);
  background: color-mix(in srgb, var(--surface-raised) 92%, transparent);
}

.freeform-prompt-jump-button:focus-visible {
  outline: 1px solid color-mix(in srgb, var(--accent-neutral) 68%, transparent);
  outline-offset: 2px;
}

.freeform-prompt-jump-button:disabled {
  color: var(--subtle);
  cursor: default;
  opacity: 0.38;
}

.freeform-turn-actions {
  display: none;
  flex: 0 0 auto;
  align-items: center;
  gap: 4px;
  min-width: 0;
  white-space: nowrap;
}

.freeform-turn-actions[hidden] {
  display: none;
}

.freeform-turn-action {
  display: inline-flex;
  min-height: 22px;
  align-items: center;
  justify-content: center;
  gap: 4px;
  border: 1px solid color-mix(in srgb, var(--line) 78%, transparent);
  border-radius: 6px;
  background: color-mix(in srgb, var(--surface) 82%, transparent);
  color: inherit;
  font-size: var(--text-2xs);
  line-height: 1;
  cursor: pointer;
}

.freeform-turn-undo {
  padding: 0 7px;
}

.freeform-turn-fork {
  padding: 0 7px;
}

.freeform-turn-action svg {
  width: 14px;
  height: 14px;
}

.freeform-turn-action:disabled {
  cursor: default;
  opacity: 0.45;
}

.freeform-footer-inline-menu {
  position: relative;
  min-width: 0;
  color: var(--muted);
  font-family: var(--font-display);
  font-size: var(--text-2xs);
}

.freeform-footer-inline-menu:not([open]):not(:focus-within) {
  contain: paint;
}

.freeform-footer-inline-menu > summary {
  display: inline-flex;
  max-width: 112px;
  min-height: 20px;
  align-items: center;
  border: 1px solid transparent;
  border-radius: var(--radius-xs);
  padding: 0 5px;
  color: var(--muted);
  background: transparent;
  cursor: pointer;
  list-style: none;
}

.freeform-footer-inline-menu > summary::-webkit-details-marker {
  display: none;
}

.freeform-footer-inline-menu > summary span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.freeform-footer-inline-menu[open] > summary,
.freeform-footer-inline-menu > summary:hover {
  border-color: var(--line);
  color: var(--text);
  background: color-mix(in srgb, var(--surface-raised) 50%, transparent);
}

.freeform-footer-inline-menu[data-disabled="true"] > summary,
.freeform-footer-inline-menu[data-disabled="true"] > summary:hover {
  border-color: transparent;
  color: var(--subtle);
  background: transparent;
  cursor: not-allowed;
  opacity: 0.58;
}

.freeform-footer-inline-panel {
  display: none;
  position: absolute;
  right: 0;
  bottom: calc(100% + 4px);
  width: min(190px, calc(100vw - 24px));
  max-height: min(360px, calc(100dvh - 96px));
  overflow-y: auto;
  overscroll-behavior: contain;
  scrollbar-gutter: stable;
  border: 1px solid var(--line-strong);
  border-radius: var(--radius-xs);
  padding: var(--space-1);
  background: color-mix(in srgb, var(--brand-black) 86%, var(--surface));
  box-shadow: var(--shadow);
  scrollbar-color: color-mix(in srgb, var(--accent-neutral) 38%, transparent) transparent;
  scrollbar-width: thin;
  z-index: var(--z-dropdown);
}

.freeform-footer-inline-menu[open] .freeform-footer-inline-panel {
  display: grid;
  gap: 1px;
}

.freeform-substitute-action {
  min-height: 20px;
  border: 1px solid transparent;
  border-radius: var(--radius-xs);
  padding: 0 6px;
  color: var(--muted);
  background: transparent;
  font-family: var(--font-display);
  font-size: var(--text-2xs);
  cursor: pointer;
  white-space: nowrap;
}

.freeform-substitute-action:hover {
  border-color: var(--line);
  color: var(--text);
  background: color-mix(in srgb, var(--surface-raised) 50%, transparent);
}

.freeform-substitute-action.menu {
  min-height: 24px;
  padding: 0 7px;
  text-align: left;
}

.freeform-substitute-status {
  display: inline-flex;
  max-width: 120px;
  min-height: 20px;
  align-items: center;
  border: 1px solid var(--line);
  border-radius: var(--radius-xs);
  padding: 0 6px;
  overflow: hidden;
  color: var(--accent-neutral);
  background: color-mix(in srgb, var(--accent-neutral) 8%, transparent);
  font-family: var(--font-display);
  font-size: var(--text-2xs);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.freeform-substitute-status.menu {
  max-width: none;
  min-height: 24px;
  color: var(--muted);
  background: transparent;
}

.freeform-footer-menu {
  display: block;
  position: relative;
  min-width: 0;
  overflow: visible;
  justify-self: end;
}

.freeform-footer-menu > summary {
  display: inline-flex;
  width: 20px;
  min-height: 20px;
  align-items: center;
  justify-content: center;
  border: 1px solid var(--line);
  border-radius: var(--radius-xs);
  padding: 0;
  color: var(--muted);
  background: color-mix(in srgb, var(--surface-raised) 44%, transparent);
  cursor: pointer;
  font-weight: 700;
  list-style: none;
}

.freeform-footer-menu summary::-webkit-details-marker {
  display: none;
}

.freeform-footer-menu-panel {
  display: grid;
  position: absolute;
  right: 0;
  bottom: calc(100% + 4px);
  width: min(218px, calc(100vw - 24px));
  gap: 2px;
  border: 1px solid var(--line-strong);
  border-radius: var(--radius-xs);
  padding: 4px;
  background: color-mix(in srgb, var(--brand-black) 86%, var(--surface));
  box-shadow: var(--shadow);
  z-index: var(--z-dropdown);
}

.freeform-footer-menu-controls {
  display: grid;
  gap: 1px;
}

.freeform-footer-menu-action {
  min-height: 21px;
  border: 1px solid transparent;
  border-radius: var(--radius-xs);
  padding: 0 6px;
  color: var(--muted);
  background: transparent;
  font-family: var(--font-display);
  font-size: var(--text-2xs);
  text-align: left;
  cursor: pointer;
}

.freeform-footer-menu-group {
  position: relative;
  color: var(--muted);
  font-family: var(--font-display);
  font-size: var(--text-2xs);
}

.freeform-footer-menu-group > summary {
  display: grid;
  grid-template-columns: minmax(54px, 0.7fr) minmax(0, 1fr) auto;
  align-items: center;
  min-height: 21px;
  gap: 6px;
  border: 1px solid transparent;
  border-radius: var(--radius-xs);
  padding: 0 6px;
  color: var(--muted);
  background: transparent;
  cursor: pointer;
  list-style: none;
}

.freeform-footer-menu-group > summary::-webkit-details-marker {
  display: none;
}

.freeform-footer-menu-group > summary span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  text-transform: uppercase;
}

.freeform-footer-menu-current {
  display: block;
  min-width: 0;
  overflow: hidden;
  color: var(--text);
  font-style: normal;
  text-align: right;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.freeform-footer-menu-chevron {
  color: var(--subtle);
  transform: rotate(0deg);
  transition: transform var(--motion-fast);
}

.freeform-footer-menu-group[open] > summary .freeform-footer-menu-chevron {
  transform: rotate(90deg);
}

.freeform-footer-menu-group[open] > summary,
.freeform-footer-menu-group > summary:hover {
  border-color: var(--line);
  color: var(--text);
  background: color-mix(in srgb, var(--surface-raised) 50%, transparent);
}

.freeform-footer-menu-group[data-disabled="true"] > summary,
.freeform-footer-menu-group[data-disabled="true"] > summary:hover {
  border-color: transparent;
  color: var(--subtle);
  background: transparent;
  cursor: not-allowed;
  opacity: 0.58;
}

.freeform-footer-submenu {
  display: grid;
  position: fixed;
  width: min(190px, calc(100vw - 24px));
  max-height: calc(100vh - 24px);
  gap: 1px;
  overflow: auto;
  border: 1px solid var(--line-strong);
  border-radius: var(--radius-xs);
  padding: 4px;
  background: color-mix(in srgb, var(--brand-black) 86%, var(--surface));
  box-shadow: var(--shadow);
  color: var(--muted);
  font-family: var(--font-display);
  font-size: var(--text-2xs);
  z-index: var(--z-modal);
}

.freeform-footer-menu-group button,
.freeform-footer-submenu button,
.freeform-footer-inline-menu button {
  min-height: 21px;
  border: 1px solid transparent;
  border-radius: var(--radius-xs);
  padding: 0 6px;
  color: var(--muted);
  background: transparent;
  font: inherit;
  text-align: left;
  cursor: pointer;
}

.freeform-footer-menu-group button:hover,
.freeform-footer-submenu button:hover,
.freeform-footer-menu-action:hover,
.freeform-footer-menu-action[data-editing="true"],
.freeform-footer-inline-menu button:hover,
.freeform-footer-inline-menu button[aria-pressed="true"],
.freeform-footer-menu-group button[aria-pressed="true"],
.freeform-footer-submenu button[aria-pressed="true"],
.freeform-footer-submenu button[aria-selected="true"] {
  border-color: var(--line);
  color: var(--text);
  background: color-mix(in srgb, var(--surface-raised) 50%, transparent);
}

.freeform-footer-menu-group button:disabled,
.freeform-footer-submenu button:disabled,
.freeform-footer-inline-menu button:disabled {
  cursor: not-allowed;
  opacity: 0.5;
}

.freeform-footer-menu-group button[data-tone="danger"],
.freeform-footer-submenu button[data-tone="danger"],
.freeform-footer-inline-menu button[data-tone="danger"] {
  color: var(--color-danger);
}

.freeform-footer-menu-group button[data-tone="warning"],
.freeform-footer-submenu button[data-tone="warning"],
.freeform-footer-inline-menu button[data-tone="warning"] {
  color: var(--color-warning);
}

[data-freeform-inline-value][data-tone="danger"] {
  color: var(--color-danger);
}

[data-freeform-inline-value][data-tone="warning"] {
  color: var(--color-warning);
}

.account-default-mark {
  display: inline-flex;
  flex-shrink: 0;
  align-items: center;
  margin-left: var(--space-1);
  color: var(--accent-neutral);
}

@container (max-width: 500px) {
  .freeform-pane-footer {
    grid-template-columns: auto auto minmax(0, 1fr) auto;
  }
}

@container (min-width: 501px) and (max-width: 640px) {
  .freeform-runtime-placement {
    max-width: 100%;
  }
}

@container (max-width: 300px) {
  .freeform-pane-footer {
    gap: 4px;
    padding-inline: 6px;
  }

  .freeform-agent-footer-leading {
    max-width: 6ch;
    overflow: hidden;
  }

  .freeform-runtime-placement {
    min-width: 4ch;
    max-width: 6ch;
    gap: 4px;
    padding-inline: 4px;
  }

  .freeform-status-badge {
    min-width: 0;
    padding: 0 5px;
  }
}

.freeform-agent-info {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.freeform-prompt-shell {
  display: block;
  position: relative;
  min-width: 0;
  min-height: 72px;
  border-top: 1px solid color-mix(in srgb, var(--line) 78%, transparent);
  padding: 0;
  background: color-mix(in srgb, var(--footer-surface-strong) 92%, transparent);
  font-family: var(--font-display);
  z-index: var(--z-overlay);
}

.freeform-command-center {
  position: absolute;
  right: 8px;
  bottom: calc(100% + 8px);
  left: 8px;
  max-height: min(320px, 42vh);
  overflow: hidden;
  border: 1px solid var(--line-strong);
  border-radius: var(--radius-xs);
  background: color-mix(in srgb, var(--brand-black) 88%, var(--surface));
  box-shadow: var(--shadow);
  z-index: var(--z-dropdown);
}

.freeform-command-center[hidden] {
  display: none;
}

.freeform-command-center-list {
  display: grid;
  max-height: inherit;
  overflow-y: auto;
  padding: 3px;
  font-size: var(--text-sm);
  line-height: 1.22;
  scrollbar-color: color-mix(in srgb, var(--accent-neutral) 62%, transparent) transparent;
  scrollbar-width: thin;
}

.freeform-command-center-row {
  display: grid;
  grid-template-columns: max-content minmax(0, 1fr) max-content;
  align-items: center;
  min-height: 26px;
  gap: 8px;
  border: 1px solid transparent;
  border-radius: var(--radius-xs);
  padding: 0 7px;
  color: var(--muted);
  background: transparent;
  font: inherit;
  text-align: left;
  cursor: pointer;
}

.freeform-command-center-row[aria-selected="true"],
.freeform-command-center-row:hover {
  border-color: color-mix(in srgb, var(--accent-neutral) 58%, var(--line));
  color: var(--text);
  background: color-mix(in srgb, var(--surface-raised) 54%, transparent);
}

.freeform-command-center-label {
  color: var(--text);
  font-weight: 600;
}

.freeform-command-center-description,
.freeform-command-center-value {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.freeform-command-center-value {
  justify-self: end;
  color: var(--subtle);
}

.freeform-composer {
  display: grid;
  position: relative;
  grid-template-rows: auto auto auto;
  min-width: 0;
  min-height: 72px;
  border: 1px solid color-mix(in srgb, var(--line-strong) 72%, transparent);
  border-right: 0;
  border-left: 0;
  border-radius: 0;
  background: color-mix(in srgb, var(--line-strong) 18%, var(--surface));
  box-shadow: inset 0 1px 0 color-mix(in srgb, var(--brand-white) 4%, transparent);
  transition: border-top-color var(--motion-fast);
}

.freeform-composer:has(textarea:focus) {
  border-top-color: color-mix(in srgb, var(--accent-neutral) 26%, var(--line-strong));
}

.freeform-composer.drop-active {
  box-shadow:
    inset 0 0 0 1px color-mix(in srgb, var(--accent-neutral) 82%, transparent),
    inset 0 1px 0 color-mix(in srgb, var(--brand-white) 4%, transparent);
}

.freeform-composer.drop-active::after {
  display: grid;
  position: absolute;
  inset: 8px;
  place-items: center;
  border: 1px dashed color-mix(in srgb, var(--accent-neutral) 72%, transparent);
  border-radius: var(--radius-md);
  color: var(--accent-neutral);
  background: color-mix(in srgb, var(--brand-black) 72%, transparent);
  content: "Drop files to attach";
  font-size: var(--text-sm);
  pointer-events: none;
  z-index: var(--z-overlay);
}

.freeform-file-input {
  display: none;
}

.freeform-attachment-tray {
  display: block;
  box-sizing: border-box;
  width: 100%;
  min-width: 0;
  padding: var(--space-2) var(--space-3) 0;
}

.freeform-attachment-tray[hidden] {
  display: none;
}

.freeform-attachment-strip {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  min-width: 0;
}

.freeform-attachment-strip[data-context="sent"] {
  margin-bottom: 7px;
}

.freeform-attachment-chip {
  display: grid;
  grid-template-columns: 34px minmax(0, 1fr) auto;
  align-items: center;
  max-width: min(250px, 100%);
  min-width: 0;
  gap: 7px;
  border: 1px solid color-mix(in srgb, var(--line-strong) 70%, transparent);
  border-radius: var(--radius-md);
  padding: var(--space-1) 5px var(--space-1) var(--space-1);
  color: var(--text);
  background: color-mix(in srgb, var(--surface-raised) 66%, transparent);
}

.freeform-attachment-chip[data-kind="image"] {
  grid-template-columns: 52px auto;
  gap: 4px;
  padding: 3px;
}

.freeform-attachment-strip[data-context="sent"] .freeform-attachment-chip {
  max-width: 210px;
  background: color-mix(in srgb, var(--brand-black) 18%, transparent);
}

.freeform-attachment-thumb {
  display: grid;
  position: relative;
  width: 34px;
  height: 30px;
  border: 0;
  padding: 0;
  overflow: hidden;
  place-items: center;
  border-radius: var(--radius-xs);
  color: var(--accent-neutral);
  background: color-mix(in srgb, var(--brand-black) 42%, var(--surface));
  font-size: var(--text-2xs);
  font-weight: 700;
}

.freeform-attachment-thumb:not(:disabled) {
  cursor: pointer;
}

.freeform-attachment-edit-hint {
  grid-column: 1 / -1;
  color: var(--subtle);
  font-family: var(--font-display);
  font-size: var(--text-2xs);
  font-weight: 400;
  line-height: 1.4;
  white-space: nowrap;
}

.freeform-attachment-edit-overlay {
  position: absolute;
  inset: 0;
  display: grid;
  place-items: center;
  padding: var(--space-1);
  background: color-mix(in srgb, var(--surface) 88%, transparent);
  color: var(--text);
  font-family: var(--font-display);
  font-size: var(--text-2xs);
  font-weight: 400;
  line-height: 1.2;
  pointer-events: none;
  opacity: 0;
}

.freeform-attachment-thumb:hover .freeform-attachment-edit-overlay,
.freeform-attachment-thumb:focus-visible .freeform-attachment-edit-overlay {
  opacity: 1;
}

@media (hover: none) {
  .freeform-attachment-edit-overlay {
    inset: auto 0 0 0;
    opacity: 1;
  }
}

.freeform-attachment-thumb:focus-visible {
  outline: 1px solid var(--accent-neutral);
  outline-offset: 2px;
}

.freeform-attachment-chip[data-kind="image"] .freeform-attachment-thumb {
  width: 52px;
  height: 42px;
}

.freeform-attachment-thumb img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.freeform-attachment-file {
  padding: 0 3px;
}

.freeform-attachment-progress {
  display: grid;
  position: absolute;
  inset: 0;
  place-items: center;
  color: var(--text);
  background: rgb(0 0 0 / 54%);
  font-size: var(--text-2xs);
}

.freeform-attachment-meta {
  display: grid;
  min-width: 0;
  gap: 1px;
}

.freeform-attachment-meta span,
.freeform-attachment-meta em {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.freeform-attachment-meta span {
  font-size: var(--text-xs);
}

.freeform-attachment-meta em {
  color: var(--subtle);
  font-size: var(--text-2xs);
  font-style: normal;
}

.freeform-attachment-remove {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  border: 0;
  border-radius: var(--radius-xs);
  color: var(--muted);
  background: transparent;
  font: inherit;
  cursor: pointer;
}

.freeform-attachment-chip[data-kind="image"] .freeform-attachment-remove {
  align-self: start;
}

.freeform-attachment-remove:hover,
.freeform-attachment-remove:focus-visible {
  color: var(--text);
  background: color-mix(in srgb, var(--surface) 72%, transparent);
  outline: none;
}



.terminal-image-viewer {
  position: fixed;
  inset: 0;
  z-index: calc(var(--z-modal) + 1);
  display: grid;
  place-items: center;
  padding: 40px 72px;
  background: #000;
}

.terminal-image-viewer:focus {
  outline: none;
}

.terminal-image-viewer-frame {
  position: relative;
  display: grid;
  max-width: 100%;
  max-height: 100%;
}

.terminal-image-viewer-frame > img {
  display: block;
  max-width: calc(100vw - 144px);
  max-height: calc(100vh - 80px);
  object-fit: contain;
}

.terminal-image-viewer-actions {
  position: absolute;
  top: 12px;
  right: 12px;
  display: flex;
  gap: 5px;
}

.terminal-image-viewer-actions a,
.terminal-image-viewer-actions button,
.terminal-image-viewer-nav {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: var(--radius-xs);
  color: var(--text);
  background: rgb(0 0 0 / 66%);
  font: inherit;
  cursor: pointer;
}

.terminal-image-viewer-actions a {
  gap: 5px;
  padding: 7px 9px;
  color: var(--text);
  font-size: var(--text-xs);
  text-decoration: none;
}

.terminal-image-viewer-actions button {
  width: 31px;
  height: 31px;
}

.terminal-image-viewer-nav {
  position: absolute;
  top: 50%;
  width: 42px;
  height: 58px;
  transform: translateY(-50%);
}

.terminal-image-viewer-nav.left { left: -56px; }
.terminal-image-viewer-nav.right { right: -56px; }

.terminal-image-viewer-count {
  position: absolute;
  bottom: 12px;
  left: 50%;
  padding: 4px 7px;
  border-radius: var(--radius-xs);
  color: var(--text);
  background: rgb(0 0 0 / 66%);
  font-size: var(--text-xs);
  transform: translateX(-50%);
}

.terminal-image-viewer-actions a:hover,
.terminal-image-viewer-actions button:hover,
.terminal-image-viewer-nav:hover,
.terminal-image-viewer-actions a:focus-visible,
.terminal-image-viewer-actions button:focus-visible,
.terminal-image-viewer-nav:focus-visible {
  color: var(--accent-neutral);
  background: rgb(24 20 16 / 92%);
  outline: 1px solid var(--accent-neutral);
  outline-offset: 2px;
}

.freeform-composer-progress {
  display: grid;
  position: absolute;
  inset: 0;
  place-items: center;
  gap: 7px;
  color: var(--text);
  background: rgb(0 0 0 / 34%);
  pointer-events: none;
  z-index: var(--z-overlay);
}

.freeform-composer-progress[hidden] {
  display: none;
}

.freeform-spinner {
  width: 28px;
  height: 28px;
  border: 2px solid color-mix(in srgb, var(--accent-neutral) 28%, transparent);
  border-top-color: var(--accent-neutral);
  border-radius: var(--radius-full);
  animation: freeform-spin 720ms linear infinite;
}

.freeform-composer-progress strong {
  font-size: var(--text-xs);
  font-weight: 700;
}

@keyframes freeform-spin {
  to {
    transform: rotate(360deg);
  }
}

@keyframes waitingRoomRedactedSweep {
  0% {
    background-position: 140% 0, 0 0;
  }

  100% {
    background-position: -80% 0, 0 0;
  }
}

.freeform-prompt-shell textarea {
  field-sizing: content;
  width: 100%;
  min-height: calc(1.45em + 9px);
  max-height: min(calc(12 * 1.45em + 9px), 40vh);
  min-width: 0;
  overflow-y: auto;
  box-sizing: border-box;
  resize: none;
  border: 0;
  padding: 6px 14px 3px;
  color: var(--text);
  background: transparent;
  font: inherit;
  font-size: var(--text-sm);
  line-height: 1.45;
  outline: none;
}

.freeform-prompt-shell textarea::placeholder {
  color: var(--subtle);
}

.freeform-prompt-shell textarea:focus {
  outline: none;
}

.freeform-composer-bar {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto auto;
  align-items: center;
  gap: var(--space-3);
  min-height: 38px;
  padding: 0 10px 6px 10px;
}

.freeform-composer-bar [data-terminal-prompt-tools-root] {
  min-width: 0;
}

.freeform-composer-bar [data-context-meter-root] {
  display: flex;
  min-width: 0;
  align-items: center;
  justify-content: flex-end;
}

.freeform-composer-bar [data-terminal-send-root] {
  display: inline-flex;
  justify-self: end;
}

.freeform-composer-actions {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  min-width: 0;
}

.freeform-prompt-agent-controls {
  display: flex;
  flex: 1 1 auto;
  min-width: 0;
  align-items: center;
  gap: 4px;
}`
