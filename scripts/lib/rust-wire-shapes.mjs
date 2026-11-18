// The lines of a Rust file that decide what goes on the wire: public struct fields, enum variants, the fields inside a variant (no `pub`
// there) and serde attributes. Each carries its container path, so a field that moves inside its type or a variant that is reordered
// is the same shape, and the same field name in another type is not.
import { productionSource } from './rust-production-source.mjs'

const FIELD = /^pub(?:\([^)]*\))?\s+[a-z_]\w*\s*:(?!:)/
const VARIANT = /^[A-Z]\w*\s*(?:,|\(|\{|=|$)/
const VARIANT_FIELD = /^[a-z_]\w*\s*:(?!:)/
const SERDE = /^#\[serde\(/

function opened(line, top) {
  const type = /\b(enum|struct)\s+([A-Za-z_]\w*)/.exec(line)
  if (type) return { kind: type[1], name: type[2] }
  const variant = top?.kind === 'enum' ? /^([A-Z]\w*)/.exec(line) : null
  return variant ? { kind: 'variant', name: variant[1] } : { kind: 'other', name: '' }
}

export function wireShapes(source, path) {
  const raw = source.split('\n')
  const shapes = []
  const stack = []
  productionSource(source, path).split('\n').forEach((codeLine, i) => {
    const line = codeLine.trim()
    const top = stack.at(-1)
    const path = stack.filter(c => c.name).map(c => c.name).join('::')
    const wire = SERDE.test(line) || (top?.kind === 'struct' && FIELD.test(line)) || (top?.kind === 'enum' && VARIANT.test(line)) || (top?.kind === 'variant' && VARIANT_FIELD.test(line))
    if (wire) shapes.push({ key: `${path}|${SERDE.test(line) ? raw[i].trim() : line.replace(/\s+/g, ' ')}`, line: i + 1, text: raw[i].trim() })
    for (const c of line) {
      if (c === '{') stack.push(opened(line, stack.at(-1)))
      else if (c === '}') stack.pop()
    }
  })
  return shapes
}

// The value of `const NAME: u32 = 123;` in a file, or null.
export function u32Constant(source, name) {
  const m = new RegExp(`\\bconst\\s+${name}\\s*:\\s*u32\\s*=\\s*(\\d[\\d_]*)\\s*;`).exec(productionSource(source))
  return m ? Number(m[1].replace(/_/g, '')) : null
}
