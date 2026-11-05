// MP-08 / MP-10 (PANE-09): lazily loaded Shiki chunk. The main bundles never
// import this module statically; `code-highlight.ts` loads it by URL so the
// grammars stay out of the first paint. Grammars share this lazy chunk to make transport retries atomic.
import { createHighlighterCore, type HighlighterCore, type LanguageRegistration } from "shiki/core"
import { createJavaScriptRegexEngine } from "shiki/engine/javascript"

type LanguageModule = { readonly default: LanguageRegistration[] }

const languageLoaders: Record<string, () => Promise<LanguageModule>> = {
  bash: () => import("shiki/langs/bash.mjs"),
  c: () => import("shiki/langs/c.mjs"),
  cpp: () => import("shiki/langs/cpp.mjs"),
  csharp: () => import("shiki/langs/csharp.mjs"),
  css: () => import("shiki/langs/css.mjs"),
  docker: () => import("shiki/langs/docker.mjs"),
  go: () => import("shiki/langs/go.mjs"),
  html: () => import("shiki/langs/html.mjs"),
  ini: () => import("shiki/langs/ini.mjs"),
  java: () => import("shiki/langs/java.mjs"),
  javascript: () => import("shiki/langs/javascript.mjs"),
  json: () => import("shiki/langs/json.mjs"),
  jsx: () => import("shiki/langs/jsx.mjs"),
  kotlin: () => import("shiki/langs/kotlin.mjs"),
  lua: () => import("shiki/langs/lua.mjs"),
  make: () => import("shiki/langs/make.mjs"),
  markdown: () => import("shiki/langs/markdown.mjs"),
  php: () => import("shiki/langs/php.mjs"),
  python: () => import("shiki/langs/python.mjs"),
  ruby: () => import("shiki/langs/ruby.mjs"),
  rust: () => import("shiki/langs/rust.mjs"),
  sql: () => import("shiki/langs/sql.mjs"),
  swift: () => import("shiki/langs/swift.mjs"),
  toml: () => import("shiki/langs/toml.mjs"),
  tsx: () => import("shiki/langs/tsx.mjs"),
  typescript: () => import("shiki/langs/typescript.mjs"),
  xml: () => import("shiki/langs/xml.mjs"),
  yaml: () => import("shiki/langs/yaml.mjs"),
}

const fontStyleItalic = 1
const fontStyleBold = 2
const fontStyleUnderline = 4

let highlighter: Promise<HighlighterCore> | null = null
const loadedLanguages = new Map<string, Promise<boolean>>()

export const supportedLanguages: readonly string[] = Object.keys(languageLoaders)

// Returns the inner HTML of a <code> element: escaped text in spans that only
// carry `--hl`/`--hd` colour variables (light/dark) plus font flags. Callers
// treat `null` as "leave the plain code".
export async function highlightCode(code: string, language: string): Promise<string | null> {
  const loader = languageLoaders[language]
  if (!loader) return null
  const core = await loadHighlighter()
  if (!await loadLanguage(core, language, loader)) return null
  const lines = core.codeToTokens(code, {
    lang: language,
    themes: { light: "github-light", dark: "github-dark" },
    defaultColor: false,
  }).tokens
  return lines.map((line) => line.map((token) => {
    const style = token.htmlStyle ?? {}
    const light = colorValue(style["--shiki-light"])
    const dark = colorValue(style["--shiki-dark"])
    const flags = token.fontStyle ?? 0
    const parts = [
      light ? `--hl:${light}` : "",
      dark ? `--hd:${dark}` : "",
      flags & fontStyleItalic ? "font-style:italic" : "",
      flags & fontStyleBold ? "font-weight:bold" : "",
      flags & fontStyleUnderline ? "text-decoration:underline" : "",
    ].filter(Boolean)
    const text = escapeHtml(token.content)
    return parts.length > 0 ? `<span style="${parts.join(";")}">${text}</span>` : text
  }).join("")).join("\n")
}

function loadHighlighter(): Promise<HighlighterCore> {
  highlighter ??= Promise.all([
    import("shiki/themes/github-light.mjs"),
    import("shiki/themes/github-dark.mjs"),
  ]).then(([light, dark]) => createHighlighterCore({
    themes: [light.default, dark.default],
    langs: [],
    engine: createJavaScriptRegexEngine({ forgiving: true }),
  }))
  return highlighter
}

function loadLanguage(core: HighlighterCore, language: string, loader: () => Promise<LanguageModule>): Promise<boolean> {
  let loaded = loadedLanguages.get(language)
  if (!loaded) {
    loaded = loader()
      .then((module) => core.loadLanguage(...module.default))
      .then(() => true, () => false)
    loadedLanguages.set(language, loaded)
  }
  return loaded
}

function colorValue(value: string | undefined): string {
  return value && /^#[0-9a-f]{3,8}$/i.test(value) ? value.toLowerCase() : ""
}

function escapeHtml(value: string): string {
  return value.replace(/[&<>"']/g, (match) => `&#${match.charCodeAt(0)};`)
}
