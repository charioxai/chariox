// MP-08 / MP-10 (PANE-09): rich Markdown content in agent panes. The
// transcript container is `white-space: pre-wrap`; rendered Markdown carries its
// own block structure, so whitespace is normal inside it and preserved only in
// code frames.
export const freeformMarkdownStyles = `
.freeform-markdown {
  display: block;
  min-width: 0;
  white-space: normal;
  line-height: 1.45;
}

.freeform-markdown > * {
  margin-top: 0;
  margin-bottom: 0;
}

.freeform-markdown > * + *,
.freeform-markdown li > * + *,
.freeform-markdown blockquote > * + * {
  margin-top: 0.55em;
}

.freeform-markdown p {
  line-height: 1.45;
}

.freeform-markdown h1,
.freeform-markdown h2,
.freeform-markdown h3,
.freeform-markdown h4,
.freeform-markdown h5,
.freeform-markdown h6 {
  margin-bottom: 0;
  color: var(--text);
  font-weight: 700;
  line-height: 1.25;
}

.freeform-markdown h1 { font-size: 1.32em; }
.freeform-markdown h2 { font-size: 1.18em; }
.freeform-markdown h3 { font-size: 1.06em; }
.freeform-markdown h4,
.freeform-markdown h5,
.freeform-markdown h6 { font-size: 1em; color: color-mix(in srgb, var(--text) 82%, var(--muted)); }

.freeform-markdown > :is(h1, h2, h3, h4, h5, h6) + * {
  margin-top: 0.35em;
}

.freeform-markdown ul,
.freeform-markdown ol {
  margin: 0;
  padding-left: 1.5em;
}

.freeform-markdown li + li {
  margin-top: 0.15em;
}

.freeform-markdown li > ul,
.freeform-markdown li > ol {
  margin-top: 0.15em;
}

.freeform-markdown li::marker {
  color: var(--muted);
}

.freeform-markdown .freeform-md-task-list {
  padding-left: 0.2em;
  list-style: none;
}

.freeform-markdown .freeform-md-task-item {
  list-style: none;
}

.freeform-md-task {
  display: inline-block;
  position: relative;
  width: 0.95em;
  height: 0.95em;
  margin-right: 0.5em;
  border: 1px solid color-mix(in srgb, var(--muted) 72%, transparent);
  border-radius: var(--radius-xs);
  vertical-align: -0.14em;
}

.freeform-md-task[data-checked="true"] {
  border-color: var(--accent-neutral);
  background: var(--accent-neutral);
}

.freeform-md-task[data-checked="true"]::after {
  content: "";
  position: absolute;
  left: 30%;
  top: 8%;
  width: 28%;
  height: 56%;
  border: solid var(--accent-neutral-ink);
  border-width: 0 2px 2px 0;
  transform: rotate(45deg);
}

.freeform-markdown blockquote {
  margin-left: 0;
  margin-right: 0;
  border-left: 3px solid var(--line-strong);
  padding-left: 0.75em;
  color: var(--muted);
}

.freeform-markdown hr {
  height: 0;
  border: 0;
  border-top: 1px solid var(--line-strong);
}

.freeform-md-link {
  color: inherit;
  text-decoration: underline;
  text-decoration-color: color-mix(in srgb, var(--accent-neutral) 55%, transparent);
  text-underline-offset: 2px;
  overflow-wrap: anywhere;
}

.freeform-md-link:hover {
  text-decoration-color: currentColor;
}

.freeform-md-link:focus-visible {
  outline: 1px solid color-mix(in srgb, var(--accent-neutral) 58%, transparent);
  outline-offset: 2px;
}

.freeform-md-link::after {
  content: "\\2197";
  margin-left: 0.12em;
  color: var(--muted);
  font-size: 0.8em;
}

.freeform-md-link-inert {
  text-decoration: underline dotted;
  text-decoration-color: var(--subtle);
}

.freeform-markdown del,
.freeform-markdown s {
  color: var(--muted);
}

.freeform-md-image {
  display: inline-flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 4px;
  max-width: 100%;
  vertical-align: top;
}

.freeform-md-img {
  display: block;
  width: auto;
  height: auto;
  max-width: min(100%, 520px);
  max-height: 360px;
  object-fit: contain;
  border: 1px solid var(--line);
  border-radius: var(--radius-xs);
  background: color-mix(in srgb, var(--surface-raised) 60%, transparent);
  cursor: zoom-in;
}

.freeform-md-image[data-md-image-expanded="true"] .freeform-md-img {
  max-width: 100%;
  max-height: none;
  cursor: zoom-out;
}

.freeform-md-image-card {
  display: inline-flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 2px 10px;
  max-width: 100%;
  border: 1px dashed var(--line-strong);
  border-radius: var(--radius-xs);
  padding: 4px 8px;
  color: var(--muted);
}

.freeform-md-image[data-md-image-state="loaded"] .freeform-md-image-card {
  border: 0;
  padding: 0;
  font-size: var(--text-2xs);
}

.freeform-md-image-alt {
  color: var(--text);
}

.freeform-md-image-alt::before {
  content: "image ";
  color: var(--subtle);
}

.freeform-md-image-meta,
.freeform-md-image-status {
  color: var(--subtle);
  overflow-wrap: anywhere;
}

.freeform-md-image-actions {
  display: inline-flex;
  gap: 8px;
}

.freeform-md-image-load {
  border: 1px solid var(--line-strong);
  border-radius: var(--radius-xs);
  padding: 0 6px;
  color: var(--text);
  background: transparent;
  font: inherit;
  cursor: pointer;
}

.freeform-md-image-load:hover {
  border-color: var(--accent-neutral);
}

.freeform-md-copy {
  border: 0;
  border-radius: var(--radius-xs);
  padding: 0 4px;
  color: var(--subtle);
  background: transparent;
  font: inherit;
  font-size: var(--text-2xs);
  line-height: 1.5;
  text-transform: lowercase;
  cursor: pointer;
  opacity: 0;
  transition: opacity var(--motion-fast), color var(--motion-fast);
}

.freeform-terminal-code:hover .freeform-md-copy,
.freeform-md-table:hover .freeform-md-copy,
.freeform-md-copy:focus-visible {
  opacity: 1;
}

.freeform-md-copy:hover {
  color: var(--text);
}

.freeform-terminal-code figcaption {
  display: flex;
  justify-content: flex-end;
  align-items: center;
  gap: 6px;
}

.freeform-md-table {
  width: max-content;
  max-width: 100%;
  min-width: 0;
}

.freeform-md-table-bar {
  display: flex;
  justify-content: flex-end;
}

.freeform-md-table-scroll {
  max-width: 100%;
  overflow-x: auto;
  border: 1px solid var(--line-strong);
  border-radius: var(--radius-xs);
  scrollbar-width: thin;
}

.freeform-md-table table {
  border-collapse: collapse;
  font-variant-numeric: tabular-nums;
}

.freeform-md-table th,
.freeform-md-table td {
  border: 1px solid var(--line);
  padding: 3px 9px;
  text-align: left;
  vertical-align: top;
  white-space: nowrap;
}

.freeform-md-table tr > :first-child { border-left: 0; }
.freeform-md-table tr > :last-child { border-right: 0; }
.freeform-md-table thead tr:first-child > * { border-top: 0; }
.freeform-md-table tbody tr:last-child > * { border-bottom: 0; }

.freeform-md-table th {
  color: var(--text);
  background: color-mix(in srgb, var(--surface-raised) 80%, transparent);
  font-weight: 700;
}

.freeform-md-table tbody tr:nth-child(even) {
  background: color-mix(in srgb, var(--surface-raised) 38%, transparent);
}

.freeform-md-math[data-math-state="pending"],
.freeform-md-math[data-math-state="error"] {
  color: var(--muted);
  font-family: var(--font-display);
}

.freeform-md-math-display {
  display: block;
  max-width: 100%;
  overflow-x: auto;
  overflow-y: hidden;
  scrollbar-width: thin;
}

.freeform-md-math .katex {
  font-size: 1.1em;
}

.freeform-md-math-display .katex-display {
  margin: 0.25em 0;
  text-align: left;
}

.freeform-md-math-display .katex-display > .katex {
  text-align: left;
}

.freeform-diff {
  display: flex;
  flex-direction: column;
  min-width: 0;
  max-width: 100%;
  white-space: normal;
}

.freeform-diff-header,
.freeform-diff-file {
  color: var(--text);
  line-height: 1.45;
  overflow-wrap: anywhere;
}

.freeform-diff-header {
  padding: 2px 4px;
}

.freeform-diff-verb {
  font-weight: 700;
}

.freeform-diff-file {
  padding: 2px 4px 0;
  color: color-mix(in srgb, var(--text) 86%, var(--muted));
}

.freeform-diff-file::before {
  content: "\\2514  ";
  color: var(--subtle);
}

.freeform-diff-stat {
  color: var(--subtle);
}

.freeform-diff-add {
  color: var(--color-added);
}

.freeform-diff-del {
  color: var(--color-danger);
}

.freeform-diff-lines {
  display: grid;
  grid-template-columns: max-content max-content max-content;
  min-width: 0;
  max-width: 100%;
  overflow-x: auto;
  padding: 2px 0 4px;
  scrollbar-width: thin;
  line-height: 1.35;
}

.freeform-diff-row {
  display: contents;
}

.freeform-diff-overflow {
  color: var(--subtle);
}

.freeform-diff-overflow > summary {
  cursor: pointer;
}

.freeform-diff-overflow > pre {
  max-width: 100%;
  overflow-x: auto;
  color: var(--text);
  scrollbar-width: thin;
}

.freeform-diff-row > span {
  padding: 0 4px;
  white-space: pre;
}

.freeform-diff-ln {
  color: var(--subtle);
  text-align: right;
  user-select: none;
}

.freeform-diff-sign {
  padding-right: 0;
  user-select: none;
}

.freeform-diff-row[data-kind="added"] > span {
  color: var(--color-added);
  background: var(--color-added-bg);
}

.freeform-diff-row[data-kind="removed"] > span {
  color: var(--color-danger);
  background: var(--color-removed-bg);
}

.freeform-diff-row[data-kind] > .freeform-diff-ln {
  color: var(--subtle);
}

.freeform-diff-gap,
.freeform-diff-meta {
  grid-column: 1 / -1;
  padding: 0 4px;
  color: var(--subtle);
}

.freeform-diff-meta {
  font-style: italic;
}

.freeform-diff-figure > figcaption {
  position: absolute;
  top: 2px;
  right: 2px;
  padding: 0;
}

.freeform-diff-figure .freeform-diff-header {
  padding-right: 48px;
}

.freeform-diff-figure > .freeform-diff {
  border-top: 1px solid color-mix(in srgb, var(--line) 62%, transparent);
}

.freeform-tool-patch .freeform-diff {
  border: var(--freeform-output-frame-border);
  border-radius: var(--freeform-output-frame-radius);
  background: color-mix(in srgb, var(--brand-black) 88%, var(--surface) 12%);
}

.freeform-terminal-code code span {
  color: var(--hd, inherit);
}

/* The terminal shell (Room panes) is always dark; light code frames apply only
   to light-theme surfaces outside it (e.g. workflow and recall routes). */
:root[data-theme="light"] .freeform-terminal-code:not(.app-shell[data-shell="terminal"] *) {
  background: color-mix(in srgb, var(--surface-raised) 72%, var(--surface));
}

:root[data-theme="light"] .freeform-terminal-code:not(.app-shell[data-shell="terminal"] *) code span {
  color: var(--hl, inherit);
}

@media (prefers-color-scheme: light) {
  :root:not([data-theme]) .freeform-terminal-code:not(.app-shell[data-shell="terminal"] *) {
    background: color-mix(in srgb, var(--surface-raised) 72%, var(--surface));
  }

  :root:not([data-theme]) .freeform-terminal-code:not(.app-shell[data-shell="terminal"] *) code span {
    color: var(--hl, inherit);
  }
}
`
