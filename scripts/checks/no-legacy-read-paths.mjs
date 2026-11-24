// Development stage: no shims, legacy read paths or version-gated fallbacks; re-provision or re-pair old kernels instead.
// Two shapes a legacy read path takes: a serde alias that still accepts the old field or variant name, and a declaration named legacy.
// A listed file may keep today's count (aliases for a name the provider or another system still sends carry that why); a new one anywhere else is red.
// Review after 2026-12-15: drop the list entries that never shrank or add the next shim shape that review finds.
import { tsCode } from '../lib/ts-code.mjs'
import { productionSource } from '../lib/rust-production-source.mjs'

const RUST = /^(apps|packages|adapters)\/.*\.rs$/
const TS = /^(apps|packages)\/.*\.tsx?$/
const TS_TEST = /(\.test\.tsx?$|\.spec\.tsx?$|\/tests?\/|\/__tests__\/|\.d\.ts$)/
const SERDE_ATTRIBUTE = /#\[\s*serde\s*\(/g
const ALIAS = /\balias\s*=/g
const RUST_LEGACY = /\b(?:fn|struct|enum|type|const|static|mod|trait)\s+((?:legacy_|Legacy)\w*)/g
// Top-level declarations only: a local variable that happens to be called legacy is not a path.
const TS_LEGACY = /^(?:export\s+)?(?:async\s+)?(?:function|const|class|interface|type|enum)\s+((?:legacy|Legacy)\w+)/gm

// Every `alias =` inside a #[serde(...)] attribute: one attribute can carry several old names, each a read path.
// String literals are blanked in `code`, so the parentheses that close the attribute are the real ones.
function* serdeAliases(code) {
  for (const attribute of code.matchAll(SERDE_ATTRIBUTE)) {
    let depth = 1, end = attribute.index + attribute[0].length
    for (; end < code.length && depth; end++) {
      if (code[end] === '(') depth++
      else if (code[end] === ')') depth--
    }
    for (const alias of code.slice(attribute.index, end).matchAll(ALIAS)) yield attribute.index + alias.index
  }
}

export default {
  name: 'no-legacy-read-paths',
  why: 'a serde alias or legacy declaration from before this check existed; delete the old path when its writers are gone',
  rule: 'AGENTS.md "Development stage": backward compatibility is not a goal; no shims, legacy read paths or version-gated fallbacks',
  rootFix: 'delete the old name, alias or fallback and re-provision or re-pair old kernels; a name an external system still sends goes in the list with the system that sends it',
  scan(ctx) {
    const findings = []
    const at = (code, index) => code.slice(0, index).split('\n').length
    for (const file of ctx.files) {
      const isRust = RUST.test(file)
      if (!isRust && !(TS.test(file) && !TS_TEST.test(file))) continue
      const text = ctx.read(file)
      if (text === null) continue
      const code = isRust ? productionSource(text, file) : tsCode(text)
      if (isRust) for (const alias of serdeAliases(code)) findings.push({ file, line: at(code, alias), what: 'keeps a serde alias that accepts an old name' })
      for (const m of code.matchAll(isRust ? RUST_LEGACY : TS_LEGACY)) findings.push({ file, line: at(code, m.index), what: `declares ${m[1]}, a legacy path` })
    }
    return findings
  },
}
