const FENCE_LANGUAGE_ALIASES: Record<string, string> = {
  adb: "ada",
  asy: "assembly",
  bat: "batch",
  cplusplus: "cpp",
  cxx: "cpp",
  cc: "cpp",
  "c++": "cpp",
  cs: "csharp",
  docker: "dockerfile",
  env: "dotenv",
  golang: "go",
  htm: "html",
  ini: "toml",
  jinja: "jinja-html",
  jinja2: "jinja-html",
  js: "javascript",
  jsx: "javascriptreact",
  kt: "kotlin",
  md: "markdown",
  objc: "objective-c",
  objcpp: "objective-cpp",
  py: "python",
  rb: "ruby",
  rs: "rust",
  sh: "shellscript",
  shell: "shellscript",
  tf: "terraform",
  tfvars: "terraform-vars",
  ts: "typescript",
  tsx: "typescriptreact",
  vim: "viml",
  yml: "yaml",
  zsh: "shellscript",
}

const PATH_LANGUAGE_ALIASES: Record<string, string> = {
  ".abap": "abap",
  ".astro": "astro",
  ".bat": "bat",
  ".bib": "bibtex",
  ".bibtex": "bibtex",
  ".bash": "shellscript",
  ".c": "c",
  ".cc": "cpp",
  ".clj": "clojure",
  ".cljs": "clojure",
  ".cljc": "clojure",
  ".coffee": "coffeescript",
  ".cpp": "cpp",
  ".cs": "csharp",
  ".cjs": "javascript",
  ".css": "css",
  ".cts": "typescript",
  ".ctsx": "typescriptreact",
  ".cxx": "cpp",
  ".d": "d",
  ".dart": "dart",
  ".diff": "diff",
  ".dockerfile": "dockerfile",
  ".edn": "clojure",
  ".erb": "erb",
  ".erl": "erlang",
  ".ets": "typescript",
  ".ex": "elixir",
  ".exs": "elixir",
  ".fs": "fsharp",
  ".fsi": "fsharp",
  ".fsscript": "fsharp",
  ".fsx": "fsharp",
  ".gitcommit": "git-commit",
  ".gitrebase": "git-rebase",
  ".gleam": "gleam",
  ".go": "go",
  ".gemspec": "ruby",
  ".groovy": "groovy",
  ".hbs": "handlebars",
  ".hcl": "hcl",
  ".handlebars": "handlebars",
  ".hrl": "erlang",
  ".hs": "haskell",
  ".htm": "html",
  ".html": "html",
  ".html.erb": "erb",
  ".ini": "ini",
  ".jade": "jade",
  ".java": "java",
  ".jl": "julia",
  ".js": "javascript",
  ".js.erb": "erb",
  ".json": "json",
  ".json.erb": "erb",
  ".jsx": "javascriptreact",
  ".ksh": "shellscript",
  ".kt": "kotlin",
  ".kts": "kotlin",
  ".latex": "latex",
  ".less": "less",
  ".lhs": "haskell",
  ".lua": "lua",
  ".m": "objective-c",
  ".md": "markdown",
  ".markdown": "markdown",
  ".mjs": "javascript",
  ".ml": "ocaml",
  ".mli": "ocaml",
  ".mm": "objective-cpp",
  ".mts": "typescript",
  ".mtsx": "typescriptreact",
  ".nix": "nix",
  ".pas": "pascal",
  ".pascal": "pascal",
  ".patch": "diff",
  ".php": "php",
  ".pl": "perl",
  ".pm": "perl",
  ".pm6": "perl6",
  ".ps1": "powershell",
  ".psm1": "powershell",
  ".pug": "jade",
  ".py": "python",
  ".r": "r",
  ".rake": "ruby",
  ".razor": "razor",
  ".rb": "ruby",
  ".rs": "rust",
  ".ru": "ruby",
  ".scss": "scss",
  ".sass": "sass",
  ".scala": "scala",
  ".shader": "shaderlab",
  ".sh": "shellscript",
  ".sql": "sql",
  ".svelte": "svelte",
  ".swift": "swift",
  ".tex": "latex",
  ".tf": "terraform",
  ".tfvars": "terraform-vars",
  ".toml": "toml",
  ".ts": "typescript",
  ".tsx": "typescriptreact",
  ".typ": "typst",
  ".typc": "typst",
  ".vue": "vue",
  ".xml": "xml",
  ".xsl": "xsl",
  ".yaml": "yaml",
  ".yml": "yaml",
  ".zig": "zig",
  ".zon": "zig",
  ".zsh": "shellscript",
}

const PATH_BASENAME_ALIASES: Record<string, string> = {
  ".env": "dotenv",
  "dockerfile": "dockerfile",
  "makefile": "makefile",
}

export function guessPathFenceLanguage(filePath: string) {
  const lower = filePath.toLowerCase()
  const name = lower.split(/[\\/]/).pop() ?? lower
  const exact = PATH_BASENAME_ALIASES[name]
  if (exact) {
    return exact
  }

  const suffix = Object.keys(PATH_LANGUAGE_ALIASES)
    .sort((a, b) => b.length - a.length)
    .find((ext) => lower.endsWith(ext))
  if (suffix) {
    return PATH_LANGUAGE_ALIASES[suffix]!
  }

  return "text"
}

export function normalizeMarkdownFenceInfoStrings(text: string) {
  return text.replace(/(^|\n)```([^\n`]*)/g, (match, prefix: string, rawInfo: string) => {
    const info = rawInfo.trim()
    if (!info) {
      return `${prefix}\`\`\``
    }
    const [language = "", ...rest] = info.split(/\s+/)
    const normalized = FENCE_LANGUAGE_ALIASES[language.toLowerCase()] ?? language.toLowerCase()
    const suffix = rest.length > 0 ? ` ${rest.join(" ")}` : ""
    return `${prefix}\`\`\`${normalized}${suffix}`
  })
}

export function shouldRenderTranscriptAsMarkdown(role: string, text: string) {
  if (role !== "assistant" && role !== "reasoning" && role !== "tool" && role !== "error") {
    return false
  }
  return text.trim().length > 0
}
