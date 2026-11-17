// Preserve offsets while hiding what is not executable in TypeScript: comments and the text of string and template literals.
// A `${...}` inside a template is code and stays visible. A String.raw template carries a whole program for another runtime (the
// publication viewer's browser script): that program is scanned as code too, so its code stays visible and its own comments and string data are hidden.
// The host template still owns its backtick and its `${...}`: they act inside the embedded program's strings, comments and regexes as well.
// A regex literal is skipped so a quote inside one does not open a string; a `/` starts one when the previous significant token cannot end an expression.
const REGEX_AFTER_WORD = new Set(['return', 'typeof', 'case', 'throw', 'else', 'do', 'void', 'yield', 'await', 'in', 'of', 'delete', 'new', 'instanceof'])
const CONTROL_WORD = new Set(['if', 'while', 'for', 'with'])
const REGEX_AFTER_PUNCTUATION = new Set('(,=:[!&|?{};+-*%<>~^'.split(''))
const WORD = /[A-Za-z0-9_$]/

export function tsCode(source) {
  const out = source.split('')
  const hide = (from, to) => { for (let j = from; j < to; j++) if (out[j] !== '\n') out[j] = ' ' }
  // Hides the non-code text from `from` up to the character `closes` accepts and returns where it stopped. Inside a String.raw program the
  // host template still ends at a backtick and runs `${...}` as TypeScript, whatever string, comment or regex of the embedded program they sit in.
  // A backslash always pairs with the next character in a string or regex; elsewhere only where the host template would pair it.
  const data = (from, closes, escapes, raw) => {
    let j = from
    while (j < source.length) {
      if (raw && source[j] === '`') break
      if (raw && source[j] === '$' && source[j + 1] === '{') { j = code(j + 2, '}'); continue }
      if (closes(j)) break
      const pairs = source[j] === '\\' && (escapes || (raw && '`$\\'.includes(source[j + 1])))
      hide(j, j + (pairs ? 2 : 1)); j += pairs ? 2 : 1
    }
    return Math.min(j, source.length)
  }
  // Scans code from `start`; `end` is what closes it: `}` for a template expression, a backtick for a String.raw program, null for the file.
  // Returns the index after the closing character.
  const code = (start, end) => {
    let i = start, depth = 0
    const raw = end === '`'
    // The previous significant token: '' at the start, a word, a punctuation character, 'literal' after a string, template or regex,
    // 'property' after `.name`, or 'control)' after the `)` that closes an if/while/for/with condition.
    let previous = ''
    const parens = []
    const regexAllowed = () => previous === '' || previous === 'control)' || REGEX_AFTER_WORD.has(previous) || REGEX_AFTER_PUNCTUATION.has(previous)
    while (i < source.length) {
      const c = source[i]
      if (c === '/' && source[i + 1] === '/') { i = data(i, j => source[j] === '\n', false, raw); continue }
      if (c === '/' && source[i + 1] === '*') {
        hide(i, i + 2)
        const to = data(i + 2, j => source[j] === '*' && source[j + 1] === '/', false, raw)
        if (source[to] === '*') { hide(to, to + 2); i = to + 2 } else i = to
        continue
      }
      if (c === '"' || c === "'") {
        const to = data(i + 1, j => source[j] === c || source[j] === '\n', true, raw)
        i = source[to] === c ? to + 1 : to; previous = 'literal'; continue
      }
      if (c === '`') {
        if (raw) return i + 1
        i = source.slice(Math.max(0, i - 10), i) === 'String.raw' ? code(i + 1, '`') : template(i)
        previous = 'literal'; continue
      }
      if (c === '/' && regexAllowed()) {
        let inClass = false
        const to = data(i + 1, j => {
          if (source[j] === '\n') return true
          if (source[j] === '[') inClass = true
          else if (source[j] === ']') inClass = false
          return source[j] === '/' && !inClass
        }, true, raw)
        if (source[to] === '/') { i = to + 1; previous = 'literal'; continue }
        // Not a regex: undo the hiding and read the slash as a division.
        for (let k = i + 1; k < to; k++) out[k] = source[k]
      }
      if (raw) {
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
