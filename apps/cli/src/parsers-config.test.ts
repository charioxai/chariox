import assert from "node:assert/strict"
import test from "node:test"

import parserConfig from "./parsers-config.js"

test("typescript-family parsers include base JavaScript highlight queries", () => {
  const parsers = new Map(parserConfig.parsers.map((parser) => [parser.filetype, parser]))

  assert.deepEqual(parsers.get("typescript")?.queries.highlights, [
    "https://raw.githubusercontent.com/tree-sitter/tree-sitter-javascript/refs/heads/master/queries/highlights.scm",
    "https://raw.githubusercontent.com/tree-sitter/tree-sitter-typescript/master/queries/highlights.scm",
  ])

  assert.deepEqual(parsers.get("typescriptreact")?.queries.highlights, [
    "https://raw.githubusercontent.com/tree-sitter/tree-sitter-javascript/refs/heads/master/queries/highlights.scm",
    "https://raw.githubusercontent.com/tree-sitter/tree-sitter-javascript/refs/heads/master/queries/highlights-jsx.scm",
    "https://raw.githubusercontent.com/tree-sitter/tree-sitter-typescript/master/queries/highlights.scm",
  ])
})

test("parser config covers OpenCode-style extended languages", () => {
  const filetypes = new Set(parserConfig.parsers.map((parser) => parser.filetype))
  for (const filetype of ["csharp", "scala", "haskell", "julia", "ocaml", "clojure", "nix"]) {
    assert.equal(filetypes.has(filetype), true)
  }
})
