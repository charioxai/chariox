// Preserve offsets while hiding what is not executable in TypeScript: comments and the text of string and template literals.
// A `${...}` inside a template is code and stays visible. A String.raw template carries a whole program for another runtime (the
// publication viewer's browser script), so its text stays visible too: the rules apply to that code as they do to the rest. A regex literal is skipped so a quote inside one does not open a string.
const REGEX_MAY_START = /[(,=:[!&|?{};+\-*%<>~^]$|^$|\b(?:return|typeof|case|throw|else|do|void|yield|await|in|of)$/

export function tsCode(source) {
  const out = source.split('')
  const hide = (from, to) => { for (let j = from; j < to; j++) if (out[j] !== '\n') out[j] = ' ' }
  // Scans code from `i` to the end of the source, or to the `}` closing a template expression when `inTemplate`.
  // Returns the index after that `}`.
  const code = (start, inTemplate) => {
    let i = start, depth = 0
    while (i < source.length) {
      const c = source[i]
      if (c === '/' && source[i + 1] === '/') { const end = source.indexOf('\n', i); const to = end < 0 ? source.length : end; hide(i, to); i = to; continue }
      if (c === '/' && source[i + 1] === '*') { const end = source.indexOf('*/', i + 2); const to = end < 0 ? source.length : end + 2; hide(i, to); i = to; continue }
      if (c === '"' || c === "'") {
        let j = i + 1
        while (j < source.length && source[j] !== c && source[j] !== '\n') j += source[j] === '\\' ? 2 : 1
        hide(i + 1, Math.min(j, source.length)); i = j + 1; continue
      }
      if (c === '`') { i = source.slice(Math.max(0, i - 10), i) === 'String.raw' ? embeddedProgram(i) : template(i); continue }
      if (c === '/' && REGEX_MAY_START.test(source.slice(Math.max(0, i - 12), i).trimEnd())) {
        let j = i + 1, inClass = false
        while (j < source.length && source[j] !== '\n' && (inClass || source[j] !== '/')) {
          if (source[j] === '\\') j++
          else if (source[j] === '[') inClass = true
          else if (source[j] === ']') inClass = false
          j++
        }
        if (source[j] === '/') { hide(i + 1, j); i = j + 1; continue }
      }
      if (inTemplate) {
        if (c === '{') depth++
        else if (c === '}' && depth-- === 0) return i + 1
      }
      i++
    }
    return i
  }
  const embeddedProgram = open => {
    let i = open + 1
    while (i < source.length && source[i] !== '`') i += source[i] === '\\' ? 2 : 1
    return i + 1
  }
  const template = open => {
    let i = open + 1
    while (i < source.length && source[i] !== '`') {
      if (source[i] === '\\') { hide(i, i + 2); i += 2 } else if (source[i] === '$' && source[i + 1] === '{') i = code(i + 2, true)
      else { hide(i, i + 1); i++ }
    }
    return i + 1
  }
  code(0, false)
  return out.join('')
}
