// Preserve offsets while hiding comments, literals, and explicitly test-only items.
export function rustCode(source) {
  let out = source.split(''), i = 0;
  const hide = (start, end) => { for (let j = start; j < end; j++) if (out[j] !== '\n') out[j] = ' ' }
  while (i < source.length) {
    let end
    if (source.startsWith('//', i)) end = source.indexOf('\n', i)
    else if (source.startsWith('/*', i)) {
      let depth = 1, j = i + 2
      while (j < source.length && depth) {
        if (source.startsWith('/*', j)) { depth++; j += 2 }
        else if (source.startsWith('*/', j)) { depth--; j += 2 }
        else j++
      }
      end = j
    } else {
      const raw = source.slice(i).match(/^(?:br|r)(#{0,255})"/)
      if (raw) {
        const close = '"' + raw[1], at = source.indexOf(close, i + raw[0].length)
        end = at < 0 ? source.length : at + close.length
      } else if (source[i] === '"' || (source[i] === "'" && /^'(?:\\.|[^'\\\n])'/.test(source.slice(i)))) {
        const quote = source[i]; let j = i + 1
        while (j < source.length) { if (source[j] === '\\') j += 2; else if (source[j++] === quote) break }
        end = j
      }
    }
    if (end !== undefined) { if (end < 0) end = source.length; hide(i, end); i = end }
    else i++
  }
  return out.join('')
}

export function productionSource(source, path = '') {
  if (/(?:^|\/)(?:tests?|test_support)(?:[\/. _]|$)|(?:_tests|_test_support)(?:\.rs$|\/)/.test(path)) return source.replace(/[^\n]/g, ' ')
  const code = rustCode(source), out = code.split('')
  const attribute = /#\[\s*(?:test|(?:tokio::)?test(?:\([^\]]*\))?|cfg\(\s*(?:test\s*|all\(\s*test\b[^\]]*)\))\s*\]/g
  for (const match of code.matchAll(attribute)) {
    let at = match.index + match[0].length
    // Skip any other attributes before the item.
    while (/\s/.test(code[at] ?? '') && at < code.length) at++
    while (code.startsWith('#[', at)) { at = code.indexOf(']', at) + 1; while (/\s/.test(code[at] ?? '') && at < code.length) at++ }
    let depth = 0, end = at
    for (; end < code.length; end++) {
      if (code[end] === '{') depth++
      if (code[end] === '}' && --depth === 0) { end++; break }
      if (code[end] === ';' && depth === 0) { end++; break }
    }
    for (let j = match.index; j < end; j++) if (out[j] !== '\n') out[j] = ' '
  }
  return out.join('')
}
