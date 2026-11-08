import { closing, deferredAsyncBlocks, inside, lineOf } from './rust-scan.mjs'

// Awaits that cannot wait on anything outside the loop: a timer, or a cooperative yield.
const pacing = new Set(['sleep', 'sleep_until', 'yield_now'])

function awaitedExpression(code, awaitAt, bodyStart) {
  let depth = 0, i = awaitAt - 1
  for (; i >= bodyStart; i--) {
    const c = code[i]
    if (c === ')' || c === ']') depth++
    else if (c === '(' || c === '[') { if (depth-- === 0) break }
    else if (depth === 0 && (c === ';' || c === '{' || c === '}' || c === ',' || (c === '>' && code[i - 1] === '='))) break
  }
  return code.slice(i + 1, awaitAt).replace(/\s+/g, ' ').trim()
}

// The call the `.await` applies to: the identifier before the final balanced argument list.
function callee(expression) {
  if (!expression.endsWith(')')) return expression
  let depth = 0, i = expression.length - 1
  for (; i >= 0; i--) {
    if (expression[i] === ')') depth++
    else if (expression[i] === '(' && --depth === 0) break
  }
  return /(\w+)\s*(?:::<[^>]*>)?\s*$/.exec(expression.slice(0, i))?.[1] ?? expression
}

// Every `.await` that suspends the body of a long-lived loop function itself must be a pacing await, must sit in
// `tokio::time::timeout(..)`, or must be a reviewed entry: `{ path, fn, call, reason }`. Awaits inside spawned or
// pushed `async` blocks run beside the loop and are not checked.
export function loopWaits(code, path, loops, allowed) {
  const hits = []
  for (const name of loops) {
    const header = new RegExp(`\\bfn\\s+${name}\\b[^{;]*\\{`).exec(code)
    if (!header) {
      hits.push({ path, line: 1, rule: `loop function \`${name}\` named in lint-kernel-blocking.mjs no longer exists: update the table` })
      continue
    }
    const open = header.index + header[0].length - 1, end = closing(code, open)
    const spans = deferredAsyncBlocks(code, open, end)
    for (const wait of code.slice(open, end).matchAll(/\.await\b/g)) {
      const at = open + wait.index
      if (inside(spans, at)) continue
      const expression = awaitedExpression(code, at, open)
      const call = callee(expression)
      if (pacing.has(call) || /\btimeout\s*\(/.test(expression) || call.startsWith('bounded_')) continue
      if (allowed.some(site => site.path === path && site.fn === name && site.call === call)) continue
      hits.push({ path, line: lineOf(code, at), rule: `unbounded \`${call}(..).await\` in loop \`${name}\`: wrap in tokio::time::timeout, spawn it single-flight, or review it into kernel-loop-waits.json` })
    }
  }
  return hits
}
