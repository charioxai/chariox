import { deferredAsyncBlocks, inside, lineOf } from './rust-scan.mjs'

const binding = /\blet\s+(?:mut\s+)?(\w+)\s*(?::[^=;{}]*)?=\s*/g
const unwraps = new Set(['unwrap', 'expect', 'unwrap_or_else', 'map_err'])

// The end of a `let` initializer (the `;` at bracket depth 0), or -1 when it holds a block.
function initializerEnd(code, from) {
  let depth = 0
  for (let i = from; i < code.length; i++) {
    const c = code[i]
    if (c === '{') return -1
    if (c === '(' || c === '[') depth++
    else if (c === ')' || c === ']') depth--
    else if (c === ';' && depth === 0) return i
  }
  return -1
}

// True when the initializer is exactly `<mutex>.lock()` / `.read()` / `.write()` in the std or parking_lot calling
// convention (no `.await`), optionally unwrapped: the binding holds the guard. A trailing `.clone()`, `.get(..)` or
// any other method means a value was taken out of the guard, which is dropped at the end of the statement.
function holdsGuard(init) {
  let rest = init.trim().replace(/\?$/, '')
  for (;;) {
    if (!rest.endsWith(')')) return false
    let depth = 0, open = rest.length - 1
    for (; open >= 0; open--) {
      if (rest[open] === ')') depth++
      else if (rest[open] === '(' && --depth === 0) break
    }
    const method = /\.(\w+)\s*$/.exec(rest.slice(0, open))?.[1]
    if (['lock', 'read', 'write'].includes(method) && rest.slice(open) === '()') return true
    if (!unwraps.has(method)) return false
    rest = rest.slice(0, open).replace(/\.\w+\s*$/, '').trim().replace(/\?$/, '')
  }
}

// A guard binding still live at an `.await` (or a `block_on`) of the same block. The guard ends at its block's closing brace, at the
// first `drop(guard)`, or when the name is shadowed. Awaits inside spawned `async` blocks do not count.
export function lockAcrossAwait(code, path, allowed) {
  const hits = []
  for (const match of code.matchAll(binding)) {
    const [, name] = match, initAt = match.index + match[0].length, semicolon = initializerEnd(code, initAt)
    if (semicolon < 0) continue
    const init = code.slice(initAt, semicolon)
    if (init.includes('.await') || !holdsGuard(init)) continue
    const start = semicolon + 1
    let depth = 0, end = start
    for (; end < code.length; end++) {
      if (code[end] === '{') depth++
      else if (code[end] === '}' && depth-- === 0) break
    }
    const rest = code.slice(start, end)
    const cut = [new RegExp(`\\bdrop\\(\\s*${name}\\s*\\)`), new RegExp(`\\blet\\s+(?:mut\\s+)?${name}\\b`)]
      .map(re => rest.search(re)).filter(at => at >= 0)
    const live = start + (cut.length ? Math.min(...cut) : rest.length)
    const spans = deferredAsyncBlocks(code, start, live)
    for (const wait of code.slice(start, live).matchAll(/\.await\b|\bblock_on\s*\(/g)) {
      const at = start + wait.index
      if (inside(spans, at)) continue
      const line = lineOf(code, match.index)
      if (!allowed.some(site => site.path === path && site.guard === name && site.init === init.replace(/\s+/g, ' ').trim())) {
        hits.push({ path, line, rule: `std/parking_lot guard \`${name}\` held across .await/block_on at line ${lineOf(code, at)}: drop it first or use an async lock` })
      }
      break
    }
  }
  return hits
}
