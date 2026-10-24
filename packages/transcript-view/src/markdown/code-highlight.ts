// MP-08 / MP-10 (PANE-09): lazy, cached, debounced highlighting for fenced
// code (see lazy-render-cache.ts). The Shiki chunk is loaded by URL on the
// first highlightable fence and never part of the first paint.
import { createLazyRenderer } from "./lazy-render-cache.js"

declare const __CHARIOX_CODE_HIGHLIGHTER_URL__: string | undefined

type HighlightInput = { readonly code: string; readonly language: string }

const languageAliases: Record<string, string> = {
  bash: "bash", sh: "bash", shell: "bash", shellscript: "bash", zsh: "bash", console: "bash",
  c: "c", h: "c",
  cpp: "cpp", "c++": "cpp", cc: "cpp", cxx: "cpp", hpp: "cpp",
  cs: "csharp", csharp: "csharp",
  css: "css",
  docker: "docker", dockerfile: "docker",
  go: "go", golang: "go",
  html: "html", htm: "html",
  ini: "ini", cfg: "ini", conf: "ini",
  java: "java",
  js: "javascript", javascript: "javascript", mjs: "javascript", cjs: "javascript", node: "javascript",
  json: "json", jsonc: "json", json5: "json",
  jsx: "jsx",
  kotlin: "kotlin", kt: "kotlin",
  lua: "lua",
  make: "make", makefile: "make",
  markdown: "markdown", md: "markdown",
  php: "php",
  py: "python", python: "python", python3: "python",
  rb: "ruby", ruby: "ruby",
  rs: "rust", rust: "rust",
  sql: "sql",
  swift: "swift",
  toml: "toml",
  tsx: "tsx",
  ts: "typescript", typescript: "typescript", mts: "typescript", cts: "typescript",
  xml: "xml", svg: "xml",
  yaml: "yaml", yml: "yaml",
}

let loadAttempt = 0

const highlighter = createLazyRenderer<HighlightInput>({
  attribute: "data-code-key",
  available: () => highlighterUrl() !== null,
  maxInputChars: 120_000,
  load: async () => {
    const url = highlighterUrl()
    if (!url) return null
    const module = await import(/* @vite-ignore */ (++loadAttempt === 1 ? url : `${url}?retry=${loadAttempt}`)) as { highlightCode?: (code: string, language: string) => Promise<string | null> }
    const highlightCode = module.highlightCode
    return highlightCode ? (input) => highlightCode(input.code, input.language) : null
  },
  apply: (element, html) => {
    if (html === null) {
      element.dataset.codeHighlight = "none"
      return
    }
    element.innerHTML = html
    element.dataset.codeHighlight = "done"
  },
})

export function highlightLanguage(language: string): string | null {
  return languageAliases[language.trim().toLowerCase()] ?? null
}

export function codeHighlightKey(code: string, language: string): string {
  return highlighter.key({ code, language })
}

export function cachedHighlight(key: string): string | null | undefined {
  return highlighter.cached(key)
}

export function requestHighlight(key: string, code: string, language: string): void {
  highlighter.request(key, { code, language })
}

export const flushHighlights = highlighter.flush

export function setCodeHighlighterForTests(render: ((code: string, language: string) => Promise<string | null>) | null): void {
  highlighter.setRenderForTests(render ? (input) => render(input.code, input.language) : null)
}

function highlighterUrl(): string | null {
  return typeof __CHARIOX_CODE_HIGHLIGHTER_URL__ === "string" && __CHARIOX_CODE_HIGHLIGHTER_URL__
    ? __CHARIOX_CODE_HIGHLIGHTER_URL__
    : null
}
