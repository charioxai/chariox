// Preserve offsets while hiding what is not executable in TypeScript: comments and the text of string and template literals.
// A `${...}` inside a template is code and stays visible. A String.raw template carries a whole program for another runtime (the
// publication viewer's browser script): that program is scanned as code too, so its code stays visible and its own comments and string data are hidden.
// A regex literal is skipped so a quote inside one does not open a string; a `/` starts one when the previous significant token cannot end an expression.
const REGEX_AFTER_WORD = new Set(['return', 'typeof', 'case', 'throw', 'else', 'do', 'void', 'yield', 'await', 'in', 'of', 'delete', 'new', 'instanceof'])
const CONTROL_WORD = new Set(['if', 'while', 'for', 'with'])
const REGEX_AFTER_PUNCTUATION = new Set('(,=:[!&|?{};+-*%<>~^'.split(''))
const WORD = /[A-Za-z0-9_$]/

export function tsCode(source) {
  const out = source.split('')
  const hide = (from, to) => { for (let j = from; j < to; j++) if (out[j] !== '\n') out[j] = ' ' }
  // Scans code from `start`; `end` is what closes it: `}` for a template expression, a backtick for a String.raw program, null for the file.
  // Returns the index after the closing character.
  const code = (start, end) => {
    let i = start, depth = 0
    // The previous significant token: '' at the start, a word, a punctuation character, 'literal' after a string, template or regex,
    // 'property' after `.name`, or 'control)' after the `)` that closes an if/while/for/with condition.
    let previous = ''
    const parens = []
    const regexAllowed = () => previous === '' || previous === 'control)' || REGEX_AFTER_WORD.has(previous) || REGEX_AFTER_PUNCTUATION.has(previous)
    while (i < source.length) {
      const c = source[i]
      if (c === '/' && source[i + 1] === '/') { const nl = source.indexOf('\n', i); const to = nl < 0 ? source.length : nl; hide(i, to); i = to; continue }
      if (c === '/' && source[i + 1] === '*') { const close = source.indexOf('*/', i + 2); const to = close < 0 ? source.length : close + 2; hide(i, to); i = to; continue }
      if (c === '"' || c === "'") {
        let j = i + 1
        while (j < source.length && source[j] !== c && source[j] !== '\n') j += source[j] === '\\' ? 2 : 1
        hide(i + 1, Math.min(j, source.length)); i = j + 1; previous = 'literal'; continue
      }
      if (c === '`') {
        if (end === '`') return i + 1
        i = source.slice(Math.max(0, i - 10), i) === 'String.raw' ? code(i + 1, '`') : template(i)
        previous = 'literal'; continue
      }
      if (c === '/' && regexAllowed()) {
        let j = i + 1, inClass = false
        while (j < source.length && source[j] !== '\n' && (inClass || source[j] !== '/')) {
          if (source[j] === '\\') j++
          else if (source[j] === '[') inClass = true
          else if (source[j] === ']') inClass = false
          j++
        }
        if (source[j] === '/') { hide(i + 1, j); i = j + 1; previous = 'literal'; continue }
      }
      if (end === '`') {
        // In a String.raw program a backslash escapes the next character and `${` still opens a TypeScript expression.
        if (c === '\\') { i += 2; continue }
        if (c === '$' && source[i + 1] === '{') { i = code(i + 2, '}'); previous = 'literal'; continue }
      }
      if (end === '}') {
        if (c === '{') depth++
        else if (c === '}' && depth-- === 0) return i + 1
      }
      if (WORD.test(c)) {
        let j = i + 1
        while (j < source.length && WORD.test(source[j])) j++
        previous = source[i - 1] === '.' ? 'property' : source.slice(i, j)
        i = j; continue
      }
      if (c === '(') parens.push(CONTROL_WORD.has(previous))
      if (c === ')') previous = parens.pop() ? 'control)' : ')'
      else if (!/\s/.test(c)) previous = c
      i++
    }
    return i
  }
  const template = open => {
    let i = open + 1
    while (i < source.length && source[i] !== '`') {
      if (source[i] === '\\') { hide(i, i + 2); i += 2 } else if (source[i] === '$' && source[i + 1] === '{') i = code(i + 2, '}')
      else { hide(i, i + 1); i++ }
    }
    return i + 1
  }
  code(0, null)
  return out.join('')
}
