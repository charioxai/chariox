// Offset-preserving scanning helpers over `productionSource` text (comments and literals already blanked).
const pairs = { '(': ')', '{': '}', '[': ']' }

// Index of the bracket closing the one at `open`, or -1.
export function closing(code, open) {
  const stack = []
  for (let i = open; i < code.length; i++) {
    const c = code[i]
    if (pairs[c]) stack.push(pairs[c])
    else if (c === ')' || c === '}' || c === ']') { if (stack.pop() !== c) return -1; if (!stack.length) return i }
  }
  return -1
}

// Spans of `async {}` / `async move {}` blocks that run elsewhere (spawned, pushed, returned) because
// nothing awaits them on the spot. Their awaits do not suspend the enclosing function.
export function deferredAsyncBlocks(code, from = 0, to = code.length) {
  const spans = []
  for (const match of code.slice(from, to).matchAll(/\basync\s+(?:move\s+)?\{/g)) {
    const open = from + match.index + match[0].length - 1, end = closing(code, open)
    if (end < 0 || end >= to) continue
    if (/^\s*\.await\b/.test(code.slice(end + 1, end + 12))) continue
    spans.push([open, end])
  }
  return spans
}

export const inside = (spans, at) => spans.some(([a, b]) => at > a && at < b)
export const lineOf = (code, at) => code.slice(0, at).split('\n').length
