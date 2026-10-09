// MP-11: independent client validation of a mirror v2 packet before any DOM,
// Blob or stylesheet is created. The kernel sanitizer is not trusted alone.
import type { Mirror2Op, Mirror2Packet, Mirror2Record } from './browser-mirror2-types.js'
export const mirror2SandboxCsp = "default-src 'none'; script-src 'none'; connect-src 'none'; img-src blob: data:; font-src blob:; style-src 'unsafe-inline'; media-src 'none'; frame-src 'self' about:; base-uri 'none'; form-action 'none'; object-src 'none'"
const id = /^n[1-9][0-9]{0,15}$/
const FORBIDDEN_HTML = new Set('script noscript template meta base link object embed applet frame frameset portal fencedframe param'.split(' '))
export const mirror2SvgTags = new Set('svg g path circle ellipse rect line polyline polygon text tspan textPath use defs symbol clipPath mask marker pattern linearGradient radialGradient stop title desc a image switch view filter feBlend feColorMatrix feComponentTransfer feComposite feConvolveMatrix feDiffuseLighting feDisplacementMap feDistantLight feDropShadow feFlood feFuncA feFuncB feFuncG feFuncR feGaussianBlur feMerge feMergeNode feMorphology feOffset fePointLight feSpecularLighting feSpotLight feTile feTurbulence style'.split(' '))
export const mirror2MathTags = new Set('math mi mo mn ms mtext mrow mfrac msqrt mroot msub msup msubsup munder mover munderover mtable mtr mtd mspace mstyle mpadded mphantom menclose semantics annotation merror'.split(' '))
const DENY_ATTR = new Set('src srcset href xlink:href action formaction srcdoc ping data codebase nonce integrity poster background lowsrc dynsrc manifest autofocus target download http-equiv is sizes imagesrcset imagesizes'.split(' '))
const INPUT_TYPES = new Set('text search email url number tel checkbox radio range button submit reset date time color hidden'.split(' '))
const MIME = new Set(['image/png', 'image/jpeg', 'image/gif', 'image/webp', 'image/avif', 'image/svg+xml', 'font/woff', 'font/woff2', 'font/ttf', 'font/otf'])
const fail = (why: string): never => { throw Error(`MP-11: unsafe mirror ${why}`) }
const finite = (values: unknown[]): boolean => values.every(v => typeof v === 'number' && Number.isFinite(v) && Math.abs(v) <= 1e7)

// Every url() must be a kernel resource key or a same-document fragment.
export function validateMirror2Css(text: unknown): void {
  if (typeof text !== 'string' || text.length > 16 * 1024 * 1024) fail('CSS')
  const css = text as string
  const all = css.match(/url\s*\(/gi)?.length ?? 0, allowed = css.match(/url\("(?:mr:r[0-9]{1,9}|#[\w-]*)"\)/g)?.length ?? 0
  if (all !== allowed || /@import|expression\s*\(|javascript:|-moz-binding|(?:^|[;{\s])behavior\s*:/i.test(css)) fail('CSS')
  // A CSS-escaped function name (`u\\72l(`) would still fetch: none may remain.
  if (/(?:[a-zA-Z_-]|\\[0-9a-fA-F]{1,6}\s?|\\[^\n0-9a-fA-F])*\\(?:[0-9a-fA-F]{1,6}\s?|[^\n0-9a-fA-F])(?:[a-zA-Z_-]|\\[0-9a-fA-F]{1,6}\s?|\\[^\n0-9a-fA-F])*\(/.test(css)) fail('CSS')
}
function validateAttrs(record: Mirror2Record): void {
  for (const [name, value] of Object.entries(record.attrs ?? {})) {
    const lower = name.toLowerCase()
    if (!/^[a-zA-Z_:][-a-zA-Z0-9_:.]*$/.test(name) || name.length > 256 || typeof value !== 'string' || value.length > 65536 || lower.startsWith('on') || /javascript:/i.test(value)) fail('attribute')
    if (lower === 'style') { validateMirror2Css(value); continue }
    if (lower === 'href') { if (!(value === '#' && !record.ns && ['a', 'area'].includes(record.tag ?? '') || record.ns === 'svg' && /^#[\w-]*$/.test(value))) fail('link'); continue }
    if (DENY_ATTR.has(lower) || /url\s*\(/i.test(value) && !/^\s*url\(\s*["']?#[\w-]+["']?\s*\)\s*$/.test(value)) fail('attribute')
  }
  if (!record.ns && record.tag === 'input' && record.attrs?.type !== undefined && !INPUT_TYPES.has(record.attrs.type.toLowerCase())) fail('form type')
  if (record.attrs?.contenteditable !== undefined && !['true', 'false', 'plaintext-only'].includes(record.attrs.contenteditable)) fail('editable state')
}
// Compact wire rows (see the kernel encoder): [idDelta, parentBack, tag | kindCode, attrs | 0, extra].
const KINDS = ['text', 'document', 'shadow', 'frame', 'mask', 'tile'] as const
export function decodeMirror2Records(rows: unknown, context: string | null): Mirror2Record[] {
  if (!Array.isArray(rows) || rows.length > 200000) fail('records')
  const out: Mirror2Record[] = []; let previous = 0
  ;(rows as unknown[]).forEach((row, i) => {
    if (!Array.isArray(row) || row.length < 3 || row.length > 5) fail('record')
    const [delta, back, head, attrs, extra] = row as [number, number, string | number, unknown, unknown]
    if (!Number.isSafeInteger(delta) || !Number.isSafeInteger(back) || back < 0 || back > i) fail('record')
    const number = previous + delta; if (!Number.isSafeInteger(number) || number < 1) fail('record'); previous = number
    const parent = back ? out[i - back]!.id : context
    if (head === 0) { if ((row as unknown[]).length !== 4 || typeof attrs !== 'string') fail('record'); out.push({ id: `n${number}`, parent, kind: 'text', text: attrs as string }); return }
    const kind = typeof head === 'string' ? 'element' : KINDS[head]
    if (!kind || kind === 'text') fail('record')
    if (attrs !== undefined && attrs !== 0 && (typeof attrs !== 'object' || attrs === null || Array.isArray(attrs))) fail('record')
    if (extra !== undefined && (typeof extra !== 'object' || extra === null || Array.isArray(extra))) fail('record')
    const record = { ...(extra as object ?? {}), id: `n${number}`, parent, kind } as Mirror2Record
    if (typeof head === 'string') record.tag = head
    if (attrs) record.attrs = attrs as Record<string, string>
    out.push(record)
  })
  return out
}
// Stylesheet digests resolve against this snapshot epoch's sheet table (the
// texts are validated wherever they are used, like inline texts).
export function resolveMirror2Sheets(packet: Mirror2Packet, known: Map<string, string>): Mirror2Packet {
  if (packet.reset) known.clear()
  if (packet.sheets !== undefined) {
    if (!packet.sheets || typeof packet.sheets !== 'object' || Array.isArray(packet.sheets) || Object.keys(packet.sheets).length > 4096) fail('sheets')
    for (const [digest, text] of Object.entries(packet.sheets)) { if (!/^[a-f0-9]{24}$/.test(digest) || typeof text !== 'string') fail('sheets'); known.set(digest, text) }
  }
  const text = (ref: unknown): string => { if (typeof ref !== 'string' || !known.has(ref)) fail('sheet reference'); return known.get(ref as string)! }
  const sheet = (item: string | { ref: string }): string => typeof item === 'string' ? item : item && typeof item === 'object' && Object.keys(item).length === 1 ? text(item.ref) : fail('sheet reference')
  const record = (r: Mirror2Record): Mirror2Record => {
    if (r.css_ref === undefined && !r.adopted?.some(item => typeof item !== 'string')) return r
    const { css_ref, ...rest } = r
    return { ...rest, ...(css_ref !== undefined ? { css: text(css_ref) } : {}), ...(r.adopted ? { adopted: r.adopted.map(sheet) } : {}) }
  }
  const { sheets: _sheets, ...out } = packet
  if (packet.nodes) out.nodes = packet.nodes.map(record)
  if (packet.ops) out.ops = packet.ops.map(op => op.op === 'children' ? { ...op, nodes: op.nodes.map(record) } : op.op === 'css' && op.css_ref !== undefined ? { op: 'css', id: op.id, css: text(op.css_ref) } : op.op === 'adopted' ? { ...op, sheets: op.sheets.map(sheet) } : op)
  return out
}
export function decodeMirror2Packet(packet: Mirror2Packet): Mirror2Packet {
  const decoded = { ...packet }
  if (packet.nodes !== undefined) decoded.nodes = decodeMirror2Records(packet.nodes, null)
  if (packet.ops !== undefined) { if (!Array.isArray(packet.ops)) fail('ops'); decoded.ops = packet.ops.map(op => op?.op === 'children' ? { ...op, nodes: decodeMirror2Records(op.nodes, op.id) } : op) }
  return decoded
}
const RECORD_KEYS = new Set(['id', 'parent', 'kind', 'tag', 'ns', 'attrs', 'text', 'css', 'res', 'form', 'scroll', 'size', 'display', 'reason', 'adopted'])
export function validateMirror2Record(record: Mirror2Record): void {
  if (record && typeof record === 'object' && Object.keys(record).some(key => !RECORD_KEYS.has(key))) fail('node')
  if (!record || typeof record !== 'object' || !id.test(record.id) || record.parent !== null && !id.test(record.parent) || !['document', 'shadow', 'element', 'text', 'frame', 'mask', 'tile'].includes(record.kind)) fail('node')
  if (['element', 'frame', 'mask', 'tile'].includes(record.kind)) {
    const tag = record.tag ?? ''
    if (record.ns === 'svg') { if (!mirror2SvgTags.has(tag)) fail('element') }
    else if (record.ns === 'math') { if (!mirror2MathTags.has(tag)) fail('element') }
    else if (record.ns !== undefined || !/^[a-z][a-z0-9-]*$/.test(tag) || FORBIDDEN_HTML.has(tag)) fail('element')
    if (tag === 'iframe' && !['frame', 'tile'].includes(record.kind) || record.kind === 'frame' && tag !== 'iframe') fail('frame')
    if (tag === 'style' && (record.kind !== 'element' || typeof record.css !== 'string')) fail('style')
  } else if (record.tag !== undefined || record.attrs !== undefined) fail('node')
  if (record.css !== undefined) { if (record.tag !== 'style') fail('style'); validateMirror2Css(record.css) }
  validateAttrs(record)
  if (record.text !== undefined && (record.kind !== 'text' || typeof record.text !== 'string' || record.text.length > 16 * 1024 * 1024)) fail('text')
  if (record.res !== undefined && !/^r[0-9]{1,9}$/.test(record.res)) fail('resource')
  if (record.scroll !== undefined && (!Array.isArray(record.scroll) || record.scroll.length !== 2 || !finite(record.scroll))) fail('scroll')
  if (record.size !== undefined && (!Array.isArray(record.size) || record.size.length !== 2 || !finite(record.size))) fail('geometry')
  if (record.form !== undefined && (typeof record.form.value !== 'string' || record.form.value.length > 65536 || typeof record.form.checked !== 'boolean' || !Number.isInteger(record.form.selected_index))) fail('form')
  if (record.adopted !== undefined) { if (!Array.isArray(record.adopted) || record.adopted.length > 256 || !['document', 'shadow'].includes(record.kind)) fail('adopted'); record.adopted.forEach(validateMirror2Css) }
  if (record.kind === 'mask' && (record.text !== undefined || record.res !== undefined || record.form !== undefined || record.css !== undefined || Object.keys(record.attrs ?? {}).some(name => !['class', 'id'].includes(name)) || !record.size)) fail('protected payload')
  if (record.kind === 'tile' && !record.size) fail('tile')
}
// Pre-order: every parent precedes its children; leaves never parent nodes.
function validateTree(records: Mirror2Record[], known: (id: string) => Mirror2Record | undefined, root: string | null): void {
  const seen = new Map<string, Mirror2Record>()
  for (const record of records) {
    validateMirror2Record(record)
    if (seen.has(record.id)) fail('duplicate node')
    const parent = record.parent === null ? null : seen.get(record.parent) ?? (record.parent === root ? known(record.parent) : undefined)
    if (record.parent !== null && !parent) fail('tree')
    if (parent && ['text', 'mask', 'tile'].includes(parent.kind)) fail('leaf')
    if (record.kind === 'shadow' && parent && !['element'].includes(parent.kind)) fail('shadow')
    if (record.kind === 'document' && parent && parent.kind !== 'frame') fail('document')
    seen.set(record.id, record)
  }
}
function validateOp(op: Mirror2Op, known: (id: string) => Mirror2Record | undefined): void {
  if (!op || typeof op !== 'object' || !id.test(op.id)) fail('op')
  switch (op.op) {
    case 'children': if (!Array.isArray(op.children) || op.children.length > 200000 || op.children.some(c => !id.test(c)) || !Array.isArray(op.nodes)) fail('op'); validateTree(op.nodes, known, op.id); break
    case 'attr': { const record = known(op.id); if (!record) break; validateAttrs({ ...record, attrs: op.value === null ? {} : { [op.name]: op.value } }); if (typeof op.name !== 'string' || op.value !== null && typeof op.value !== 'string' || record.kind === 'mask' || record.kind === 'tile') fail('op'); break }
    case 'text': if (typeof op.text !== 'string' || op.text.length > 16 * 1024 * 1024 || known(op.id) && known(op.id)!.kind !== 'text') fail('op'); break
    case 'css': validateMirror2Css(op.css); if (known(op.id) && known(op.id)!.tag !== 'style') fail('op'); break
    case 'adopted': if (!Array.isArray(op.sheets) || op.sheets.length > 256) fail('op'); op.sheets.forEach(validateMirror2Css); break
    case 'form': if (typeof op.form?.value !== 'string' || op.form.value.length > 65536 || known(op.id)?.kind === 'mask') fail('op'); break
    case 'scroll': case 'size': { const value = op.op === 'scroll' ? op.scroll : op.size; if (!Array.isArray(value) || value.length !== 2 || !finite(value)) fail('op'); break }
    case 'res': if (op.res !== null && !/^r[0-9]{1,9}$/.test(op.res)) fail('op'); break
    default: fail('op')
  }
}
export function validateMirror2Packet(packet: Mirror2Packet, previous: ReadonlyMap<string, Mirror2Record>): void {
  if (packet.wire !== 2 || !Number.isSafeInteger(packet.sequence) || packet.sequence <= 0 || packet.css_width !== 1280 || packet.css_height !== 800 || ![1, 2].includes(packet.device_scale_factor) || !Array.isArray(packet.resources) || !Array.isArray(packet.tiles) || packet.resources.length > 4096 || packet.tiles.length > 256) throw Error('MP-11: mirror packet bounds')
  if (!Array.isArray(packet.scroll) || packet.scroll.length !== 2 || !finite(packet.scroll) || packet.focused !== null && !id.test(packet.focused)) fail('header')
  if (packet.fallback !== undefined) { if (typeof packet.fallback !== 'string' || !/^[a-z_]{1,64}$/.test(packet.fallback)) fail('fallback'); return }
  const fresh = new Map<string, Mirror2Record>()
  const known = (key: string): Mirror2Record | undefined => fresh.get(key) ?? (packet.reset ? undefined : previous.get(key))
  if (packet.reset) {
    if (!Array.isArray(packet.nodes) || !packet.nodes.length || packet.nodes.length > 200000 || packet.nodes[0]!.kind !== 'document' || packet.nodes[0]!.parent !== null || packet.root !== packet.nodes[0]!.id) fail('snapshot')
    validateTree(packet.nodes!, () => undefined, null)
    for (const record of packet.nodes!) fresh.set(record.id, record)
  }
  for (const op of packet.ops ?? []) { validateOp(op, known); if (op.op === 'children') for (const record of op.nodes) fresh.set(record.id, record) }
  for (const resource of packet.resources) if (!/^r[0-9]{1,9}$/.test(resource.key) || !/^[a-f0-9]{64}$/.test(resource.resource_id) || !MIME.has(resource.mime_type) || typeof resource.data_base64 !== 'string' || resource.data_base64.length > 6 * 1024 * 1024) throw Error('MP-11: executable/oversized mirror resource')
  for (const tile of packet.tiles) if (!id.test(tile.node_id) || !finite([tile.x, tile.y, tile.width, tile.height]) || tile.width <= 0 || tile.height <= 0 || typeof tile.data_base64 !== 'string' || tile.data_base64.length > 8 * 1024 * 1024 || known(tile.node_id) && known(tile.node_id)!.kind !== 'tile') throw Error('MP-11: invalid mirror tile')
  if (packet.selection) { const s = packet.selection; if (!id.test(s.anchor_id) || !id.test(s.focus_id) || !Number.isInteger(s.anchor_offset) || !Number.isInteger(s.focus_offset) || s.anchor_offset < 0 || s.focus_offset < 0) fail('selection') }
}
