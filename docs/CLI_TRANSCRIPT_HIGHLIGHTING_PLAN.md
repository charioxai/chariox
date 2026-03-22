# CLI Transcript Highlighting Plan

## Goal

Improve the TypeScript CLI transcript so code shown by providers is syntax highlighted in the terminal and markdown output is rendered more cleanly.

This is not an LSP project. For transcript rendering, Arroba should follow the same broad approach as OpenCode:

- use terminal-native markdown and code renderers
- use syntax-highlighting/parser infrastructure for fenced code blocks
- keep LSP separate for future diagnostics/code-intelligence work

## Design

### Rendering strategy

- Keep Arroba's current transcript layout, entry roles, and session chrome.
- Upgrade assistant and reasoning transcript entries from plain text rendering to markdown-aware rendering.
- Highlight fenced code blocks through OpenTUI's markdown/code renderables and Tree-sitter parser pipeline.
- Continue rendering user, status, and notice entries as simple text.
- Keep the current tool transcript formatting for now.

### Parser strategy

- Use OpenTUI parser registration in the TypeScript CLI.
- Rely on built-in OpenTUI parsers for markdown, javascript, and typescript.
- Register a trimmed Arroba parser set for common additional languages:
  - python
  - rust
  - go
  - bash
  - c / cpp
  - java
  - ruby
  - php
  - html / css
  - json / yaml
  - swift
- Normalize common fenced-code aliases before rendering:
  - `ts` -> `typescript`
  - `js` -> `javascript`
  - `py` -> `python`
  - `sh` -> `bash`
  - `yml` -> `yaml`

### Styling strategy

- Add a transcript syntax theme in the TypeScript CLI theme layer.
- Reuse Arroba colors instead of copying OpenCode's full theme system.
- Style syntax tokens plus key markdown affordances such as headings, links, code, quotes, and lists.

## Phases

### Phase 1: Fenced Code Blocks

Deliver:

- parser bootstrap in the TypeScript CLI
- fence-language normalization
- syntax-highlighted fenced code blocks in assistant and reasoning transcript entries

Status:

- implemented

### Phase 2: Better Markdown Structure

Deliver:

- markdown-aware rendering for assistant and reasoning transcript entries
- better display of headings, block quotes, links, inline code, and lists
- keep existing Arroba transcript layout while improving content rendering inside entries

Status:

- implemented

### Later, Not in Scope Now

- tool-output migration to richer markdown/code cards
- diff/file preview widgets in the transcript
- LSP-powered diagnostics or semantic-token transcript coloring

## Operational Notes

- Some syntax parsers are downloaded on demand the first time a matching fenced language is rendered.
- If a parser is unavailable, the transcript should still show the code block as plain text rather than failing the CLI.

## Verification

```bash
pnpm --filter @arroba/cli run test
```
