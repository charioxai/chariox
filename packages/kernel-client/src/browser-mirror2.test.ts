// MP-08/MP-10/MP-11: protocol 489 mirror v2 — fail closed before DOM construction.
import test from 'node:test'
import assert from 'node:assert/strict'
import { createDeflateRaw, constants as zlib } from 'node:zlib'
import { validateMirror2Packet, validateMirror2Record, validateMirror2Css, mirror2SandboxCsp, decodeMirror2Records, resolveMirror2Sheets } from './browser-mirror2-security.js'
import { Mirror2Inflater, browserMirror2MinimumProtocolVersion, attachBrowserMirror2, BrowserMirror2Renderer } from './browser-mirror2.js'
import type { Mirror2Packet, Mirror2Record } from './browser-mirror2-types.js'

const base = (nodes: Mirror2Record[], extra: Partial<Mirror2Packet> = {}): Mirror2Packet => ({ wire: 2, subscription_id: 's', tab_id: 't', generation: 1, document_id: 'd', sequence: 1, base_sequence: null, reset: true, root: nodes[0]!.id, nodes, ops: [], scroll: [0, 0], focused: null, selection: null, resources: [], tiles: [], css_width: 1280, css_height: 800, device_scale_factor: 1, ...extra })
const tree = (...rest: Mirror2Record[]): Mirror2Record[] => [{ id: 'n1', parent: null, kind: 'document' }, { id: 'n2', parent: 'n1', kind: 'element', tag: 'html' }, ...rest]
const element = (tag: string, attrs: Record<string, string> = {}, extra: Partial<Mirror2Record> = {}): Mirror2Record => ({ id: 'n3', parent: 'n2', kind: 'element', tag, attrs, ...extra })

test('MP-11: v2 refuses active elements, handlers, navigable URLs and fetching CSS', () => {
  for (const tag of ['script', 'noscript', 'template', 'object', 'embed', 'base', 'meta', 'link', 'frame', 'portal', 'Script', 'x y'])
    assert.throws(() => validateMirror2Packet(base(tree(element(tag))), new Map()), /unsafe mirror/)
  for (const [name, value] of ([['onerror', 'x'], ['onclick', 'x'], ['src', '/a.png'], ['srcset', 'a.png 1x'], ['srcdoc', '<b>'], ['action', '/x'], ['formaction', '/x'], ['href', 'https://origin.test/'], ['xlink:href', '/a'], ['poster', '/p.png'], ['data-x', 'javascript:alert(1)'], ['fill', 'url(https://leak.test/#a)']] as Array<[string, string]>))
    assert.throws(() => validateMirror2Packet(base(tree(element('a', { [name]: value }))), new Map()), /unsafe mirror/, `${name}`)
  for (const css of ['a{background:url(https://leak.test/x)}', 'a{background:url("data:image/png;base64,AA==")}', '@import url(x.css);', 'a{width:expression(alert(1))}', 'a{--x:u\\72l(//leak.test)}', 'a{-moz-binding:url("mr:r1")}', 'a{behavior:url("mr:r1")}'])
    assert.throws(() => validateMirror2Css(css), /unsafe mirror CSS/, css)
  for (const css of ['a{background:url("mr:r12")}', 'a{mask:url("#clip")}', '.md\\:flex:not(.x){display:flex}', 'a{scroll-behavior:smooth;overscroll-behavior:none}', 'a{background:url("mr:r1000003")}'])
    validateMirror2Css(css)
  // Links keep `#` (never a URL); SVG references only same-document fragments.
  validateMirror2Packet(base(tree(element('a', { href: '#', class: 'external', 'data-x': 'y', 'aria-label': 'z' }))), new Map())
  validateMirror2Packet(base(tree({ id: 'n3', parent: 'n2', kind: 'element', tag: 'use', ns: 'svg', attrs: { href: '#icon' } })), new Map())
  assert.throws(() => validateMirror2Packet(base(tree({ id: 'n3', parent: 'n2', kind: 'element', tag: 'use', ns: 'svg', attrs: { href: 'https://x/#icon' } })), new Map()), /unsafe mirror/)
  assert.throws(() => validateMirror2Packet(base(tree({ id: 'n3', parent: 'n2', kind: 'element', tag: 'foreignObject', ns: 'svg' })), new Map()), /unsafe mirror/)
  assert.match(mirror2SandboxCsp, /script-src 'none'/); assert.match(mirror2SandboxCsp, /connect-src 'none'/); assert.match(mirror2SandboxCsp, /img-src blob: data:;/)
  assert.equal(browserMirror2MinimumProtocolVersion, 489)
})

test('MP-11: masks carry no content and leaves never parent nodes', () => {
  for (const extra of [{ text: 'secret' }, { res: 'r1' }, { css: 'a{}' }, { attrs: { title: 'secret' } }, { form: { value: 'secret', checked: false, selected_index: -1, selection_start: null, selection_end: null } }])
    assert.throws(() => validateMirror2Record({ id: 'n3', parent: 'n2', kind: 'mask', tag: 'span', size: [10, 10], ...extra } as Mirror2Record), /unsafe mirror/)
  validateMirror2Record({ id: 'n3', parent: 'n2', kind: 'mask', tag: 'input', attrs: { class: 'field' }, size: [10, 10] })
  assert.throws(() => validateMirror2Packet(base(tree({ id: 'n3', parent: 'n2', kind: 'mask', tag: 'span', size: [1, 1] }, { id: 'n4', parent: 'n3', kind: 'text', text: 'under mask' })), new Map()), /unsafe mirror leaf/)
  assert.throws(() => validateMirror2Packet(base(tree({ id: 'n3', parent: 'n9', kind: 'text', text: 'detached' })), new Map()), /unsafe mirror tree/)
  assert.throws(() => validateMirror2Packet(base([{ id: 'n1', parent: null, kind: 'element', tag: 'div' }]), new Map()), /unsafe mirror snapshot/)
})

test('MP-11: deltas reference known nodes; ops cannot write attributes into masks or style into text', () => {
  const previous = new Map<string, Mirror2Record>([['n2', { id: 'n2', parent: 'n1', kind: 'element', tag: 'html' }], ['n3', { id: 'n3', parent: 'n2', kind: 'mask', tag: 'span', size: [1, 1] }], ['n5', { id: 'n5', parent: 'n2', kind: 'text', text: 'a' }]])
  const delta = (ops: NonNullable<Mirror2Packet['ops']>): Mirror2Packet => { const { nodes: _n, root: _r, ...rest } = base([{ id: 'n1', parent: null, kind: 'document' }], { reset: false, base_sequence: 1, sequence: 2, ops }); return rest }
  validateMirror2Packet(delta([{ op: 'children', id: 'n2', children: ['n5', 'n6'], nodes: [{ id: 'n6', parent: 'n2', kind: 'text', text: 'new' }] }, { op: 'text', id: 'n5', text: 'b' }]), previous)
  assert.throws(() => validateMirror2Packet(delta([{ op: 'attr', id: 'n3', name: 'title', value: 'secret' }]), previous), /unsafe mirror/)
  assert.throws(() => validateMirror2Packet(delta([{ op: 'attr', id: 'n2', name: 'onload', value: 'x' }]), previous), /unsafe mirror/)
  assert.throws(() => validateMirror2Packet(delta([{ op: 'css', id: 'n5', css: 'a{}' }]), previous), /unsafe mirror/)
  assert.throws(() => validateMirror2Packet(delta([{ op: 'children', id: 'n2', children: ['n7'], nodes: [{ id: 'n7', parent: 'n8', kind: 'text', text: 'x' }] }]), previous), /unsafe mirror tree/)
  assert.throws(() => validateMirror2Packet(delta([{ op: 'form', id: 'n3', form: { value: 'x', checked: false, selected_index: -1, selection_start: null, selection_end: null } }]), previous), /unsafe mirror/)
  assert.throws(() => validateMirror2Packet({ ...delta([]), resources: [{ key: 'r1', resource_id: 'a'.repeat(64), mime_type: 'text/html', data_base64: '' }] }, previous), /executable/)
  // Protocol 489: a form op carries only changed, typed properties.
  validateMirror2Packet(delta([{ op: 'form', id: 'n2', form: { value: 'ab', selection_start: 2, selection_end: 2 } }]), previous)
  for (const form of [{ value: 1 }, { checked: 'yes' }, { selection_start: 1.5 }, { innerHTML: '<b>' }, null])
    assert.throws(() => validateMirror2Packet(delta([{ op: 'form', id: 'n2', form } as unknown as NonNullable<Mirror2Packet['ops']>[number]]), previous), /unsafe mirror op/, JSON.stringify(form))
  // Slices: 4-aligned offsets inside a bounded whole.
  const slice = (offset: unknown, total: unknown, data = 'AAAA') => ({ ...delta([]), resources: [{ key: 'r1', resource_id: 'a'.repeat(64), mime_type: 'image/png', data_base64: data, offset, total }] } as Mirror2Packet)
  validateMirror2Packet(slice(0, 8), previous); validateMirror2Packet(slice(4, 8), previous)
  for (const [offset, total, data] of [[2, 8], [8, 8], [0, 7 * 1024 * 1024], [0, undefined], [0, 8, '']] as Array<[unknown, unknown, string?]>) assert.throws(() => validateMirror2Packet(slice(offset, total, data), previous), /executable/, `${offset}/${total}`)
})

// The kernel's encoder: one raw deflate context per subscription, sync flush per packet.
const kernelContext = () => {
  const deflate = createDeflateRaw({ level: 9, memLevel: 9 }), chunks: Buffer[] = []
  deflate.on('data', chunk => chunks.push(chunk))
  return { close: () => deflate.close(), encode: (text: string) => new Promise<{ packet_base64: string; packet_bytes: number }>(resolve => { deflate.write(Buffer.from(text)); deflate.flush(zlib.Z_SYNC_FLUSH, () => resolve({ packet_base64: Buffer.concat(chunks.splice(0)).toString('base64'), packet_bytes: Buffer.byteLength(text) })) }) }
}
test('MP-10: context-deflated bodies inflate in order; a repeat costs a few bytes; lying sizes are refused', async () => {
  const kernel = kernelContext(), inflater = new Mirror2Inflater()
  const rows = JSON.stringify({ wire: 2, sequence: 2, ops: [{ op: 'children', id: 'n9', children: ['n10'], nodes: Array.from({ length: 6 }, (_, i) => [1, 1, 'a', { class: 'suggestion-link', href: '#' }, { title: `Suggestion ${i}` }]) }] })
  const first = await kernel.encode(rows), again = await kernel.encode(rows.replace('"sequence":2', '"sequence":3'))
  assert.equal(await inflater.inflate(first.packet_base64, first.packet_bytes), rows)
  assert.equal(await inflater.inflate(again.packet_base64, again.packet_bytes), rows.replace('"sequence":2', '"sequence":3'))
  assert.ok(again.packet_base64.length < first.packet_base64.length / 3, `MP-10: a repeated typeahead body is cheap in the context (${again.packet_base64.length} vs ${first.packet_base64.length})`)
  const lying = await kernel.encode(rows)
  await assert.rejects(inflater.inflate(lying.packet_base64, lying.packet_bytes - 1), /mirror packet bounds/)
  await assert.rejects(new Mirror2Inflater().inflate(first.packet_base64, 1), /mirror packet bounds/)
  inflater.close(); kernel.close()
})

test('MP-10/MP-11: compact rows decode to records (kernel encoder fixture) and refuse malformed rows', () => {
  const rows = [[1, 0, 1], [1, 1, 'html', { lang: 'en' }], [1, 1, 'body'], [2, 1, 0, 'Hello'], [1, 2, 4, 0, { size: [10, 20], display: 'inline-block', tag: 'span' }], [1, 3, 'svg', { viewBox: '0 0 1 1' }, { ns: 'svg' }], [999999994, 4, 3, 0, { tag: 'iframe' }]]
  assert.deepEqual(decodeMirror2Records(rows, null), [
    { id: 'n1', parent: null, kind: 'document' },
    { id: 'n2', parent: 'n1', kind: 'element', tag: 'html', attrs: { lang: 'en' } },
    { id: 'n3', parent: 'n2', kind: 'element', tag: 'body' },
    { id: 'n5', parent: 'n3', kind: 'text', text: 'Hello' },
    { size: [10, 20], display: 'inline-block', tag: 'span', id: 'n6', parent: 'n3', kind: 'mask' },
    { ns: 'svg', id: 'n7', parent: 'n3', kind: 'element', tag: 'svg', attrs: { viewBox: '0 0 1 1' } },
    { tag: 'iframe', id: 'n1000000001', parent: 'n3', kind: 'frame' },
  ])
  for (const bad of [[[1, 2, 'div']], [[0, 0, 'div']], [[1, 0, 9]], [[1, 0, 0, 5]], [[1, 0, 'div', [1]]], [[1, 0, 'div', 0, 'x']], [['1', 0, 'div']]]) assert.throws(() => decodeMirror2Records(bad, null), /unsafe mirror/, JSON.stringify(bad))
  // Extra keys cannot smuggle unknown record fields past validation.
  assert.throws(() => validateMirror2Record(decodeMirror2Records([[1, 0, 'div', 0, { onload: 'x' }]], 'n1')[0]!), /unsafe mirror node/)
})

test('MP-10/MP-11: sheet references resolve within the epoch and are validated as text', () => {
  const known = new Map<string, string>(), digest = 'a'.repeat(24)
  const first = resolveMirror2Sheets({ ...base(tree({ id: 'n3', parent: 'n2', kind: 'element', tag: 'style', css_ref: digest })), sheets: { [digest]: 'p{color:red}' } }, known)
  assert.equal(first.nodes![2]!.css, 'p{color:red}'); assert.equal(first.sheets, undefined)
  const delta = { ...base([{ id: 'n1', parent: null, kind: 'document' }], { reset: false, base_sequence: 1, sequence: 2 }), ops: [{ op: 'css' as const, id: 'n3', css: '', css_ref: digest }] }
  assert.deepEqual(resolveMirror2Sheets(delta, known).ops, [{ op: 'css', id: 'n3', css: 'p{color:red}' }])
  assert.throws(() => resolveMirror2Sheets({ ...delta, ops: [{ op: 'css', id: 'n3', css: '', css_ref: 'b'.repeat(24) }] }, known), /sheet reference/)
  assert.throws(() => validateMirror2Packet(resolveMirror2Sheets({ ...base(tree({ id: 'n3', parent: 'n2', kind: 'element', tag: 'style', css_ref: digest })), sheets: { [digest]: 'p{background:url(https://leak.test)}' } }, known), new Map()), /unsafe mirror CSS/)
  // A new snapshot epoch forgets earlier sheets.
  assert.throws(() => resolveMirror2Sheets(base(tree({ id: 'n3', parent: 'n2', kind: 'element', tag: 'style', css_ref: 'c'.repeat(24) })), known), /sheet reference/)
})

// Viewer flow with a scripted kernel and a recording renderer (no DOM needed).
const flow = (script: (command: Record<string, unknown>) => Promise<unknown>) => {
  const applied: number[] = [], failures: unknown[] = []
  let failNext = false, always = false
  const packets: Mirror2Packet[] = []
  const renderer = () => ({ frame: {} as HTMLIFrameElement, ready: async () => {}, close: () => {}, apply: async (packet: Mirror2Packet) => { if (failNext) { failNext = always; throw Error('MP-11: nested mirror unavailable') } applied.push(packet.sequence); packets.push(packet) } })
  const requests: Array<Record<string, unknown>> = []
  const transport = { protocolVersion: 489, request: async (request: unknown) => { const command = (request as { KernelBrowser: { command: Record<string, unknown> } }).KernelBrowser.command; requests.push(command); return { KernelBrowser: { result: await script(command) } } } }
  return { applied, packets, failures, requests, failOnce: () => { failNext = true }, failAlways: () => { failNext = true; always = true }, start: (failingMs?: number) => attachBrowserMirror2(transport, {} as HTMLElement, { tab_id: 't', generation: 1, device_scale_factor: 1 }, { failure: error => failures.push(error) }, { credits: 1, waitMs: 0, renderer, ...(failingMs ? { failingMs } : {}) }) }
}
// Protocol 489: a reset carries the binding and header; a delta only what changed.
const packet = (sequence: number, reset: boolean, _base?: number | null) => reset ? { wire: 2, subscription_id: 's', tab_id: 't', generation: 1, document_id: 'd', sequence, base_sequence: null, reset, css_width: 1280, css_height: 800, device_scale_factor: 1, scroll: [0, 0], focused: null, selection: null, root: 'n1', nodes: [[1, 0, 1], [1, 1, 'html']], ops: [] } : { wire: 2, sequence, ops: [] }
const until = async (check: () => boolean, ms = 3000) => { const end = Date.now() + ms; while (!check() && Date.now() < end) await new Promise(resolve => setTimeout(resolve, 10)) }

test('MP-08/MP-10: a failed packet application does not stop later packets (the reset applies)', async () => {
  let sequence = 0
  const f = flow(async command => command.op === 'mirror_subscribe' ? { subscription_id: 's' } : command.op === 'mirror_close' ? { closed: true } : (sequence++, command.after_sequence === 0 ? packet(sequence, true, null) : packet(sequence, false, sequence - 1)))
  f.failOnce(); const mirror = await f.start()
  await until(() => f.applied.length >= 2)
  await mirror.close()
  assert.ok(f.applied.length >= 2, `MP-08: packets after the failed application still apply: ${f.applied}`)
  assert.deepEqual(f.failures, [])
})

test('MP-08/MP-10: a retired subscription surfaces a failure; transient failures back off', async () => {
  const retired = flow(async command => { if (command.op === 'mirror_subscribe') return { subscription_id: 's' }; if (command.op === 'mirror_close') return { closed: true }; throw Error('kernel_browser_failed: MP-11: stale or foreign mirror') })
  const first = await retired.start()
  await until(() => retired.failures.length > 0)
  assert.equal(retired.failures.length, 1); await first.close()
  const flaky = flow(async command => { if (command.op === 'mirror_subscribe') return { subscription_id: 's' }; if (command.op === 'mirror_close') return { closed: true }; throw Error('local_transport_error: timeout') })
  const second = await flaky.start()
  await new Promise(resolve => setTimeout(resolve, 1500))
  const credits = flaky.requests.filter(command => command.op === 'mirror_next').length
  await second.close()
  assert.ok(credits <= 6, `MP-10: credits back off after failures (${credits} in 1.5 s)`)
  assert.deepEqual(flaky.failures, [], 'MP-10: a short failure streak is not terminal')
})

test('MP-10: removing an inline style forgets it, so a later resource cannot restore it', () => {
  const element = () => { const attributes = new Map<string, string>(); return { nodeType: 1, attributes, setAttribute: (name: string, value: string) => attributes.set(name, value), removeAttribute: (name: string) => attributes.delete(name), addEventListener: () => {}, style: {} } }
  const iframe = element(), container = { ownerDocument: { createElement: () => iframe }, append: () => {} }
  const renderer = new BrowserMirror2Renderer(container as unknown as HTMLElement, async () => {}) as unknown as { dom: Map<string, unknown>; records: Map<string, Mirror2Record>; styled: Map<string, { keys: string[] }>; op(op: unknown, scrolls: unknown[]): void }
  const node = element(); renderer.dom.set('n5', node); renderer.records.set('n5', { id: 'n5', parent: 'n2', kind: 'element', tag: 'div' })
  renderer.op({ op: 'attr', id: 'n5', name: 'style', value: 'background:url("mr:r1")' }, [])
  assert.deepEqual(renderer.styled.get('n5#style')?.keys, ['r1'])
  renderer.op({ op: 'attr', id: 'n5', name: 'style', value: null }, [])
  assert.equal(renderer.styled.has('n5#style'), false); assert.equal(node.attributes.has('style'), false)
})

test('MP-08/MP-10: compact deltas inherit binding and header from their base; deflated bodies inflate in sequence order; late copies are not gaps', async () => {
  const kernel = kernelContext()
  const reset = { ...packet(1, true), scroll: [0, 40], focused: 'n2' }
  const bodies = [reset, { wire: 2, sequence: 2, ops: [{ op: 'text', id: 'n2', text: 'x'.repeat(600) }] }, { wire: 2, sequence: 3, focused: null, ops: [{ op: 'text', id: 'n2', text: 'x'.repeat(600) }] }, { wire: 2, sequence: 4, ops: [] }]
  const wires: unknown[] = []
  for (const body of bodies) { const text = JSON.stringify(body); wires.push(text.length >= 512 ? { wire: 2, sequence: body.sequence, ...(body.sequence === 1 ? { reset: true } : {}), encoding: 'deflate', ...await kernel.encode(text) } : body) }
  kernel.close()
  let next = 0
  // Replies arrive out of order (3 before 2) and one late copy repeats.
  const order = [0, 2, 1, 1, 3]
  const f = flow(async command => command.op === 'mirror_subscribe' ? { subscription_id: 's' } : command.op === 'mirror_close' ? { closed: true } : next < order.length ? wires[order[next++]!] : new Promise(() => {}))
  const mirror = await f.start()
  await until(() => f.applied.length >= 4)
  await mirror.close()
  assert.deepEqual(f.applied, [1, 2, 3, 4]); assert.deepEqual(f.failures, [])
  const [, second, third, fourth] = f.packets
  assert.deepEqual([second!.subscription_id, second!.tab_id, second!.document_id, second!.base_sequence, second!.reset, second!.scroll, second!.focused], ['s', 't', 'd', 1, false, [0, 40], 'n2'])
  assert.equal(third!.focused, null, 'an explicit null replaces the base value'); assert.equal(fourth!.focused, null); assert.deepEqual(fourth!.resources, [])
})

test('MP-10: resource slices reassemble in order; a reset restarts them', () => {
  const iframe = { setAttribute: () => {}, addEventListener: () => {}, style: {} }, container = { ownerDocument: { createElement: () => iframe }, append: () => {} }
  const renderer = new BrowserMirror2Renderer(container as unknown as HTMLElement, async () => {}) as unknown as { assemble(r: Record<string, unknown>): Record<string, unknown> | null; slices: Map<string, unknown> }
  const slice = (offset: number, data: string) => ({ key: 'r1', resource_id: 'a'.repeat(64), mime_type: 'image/png', data_base64: data, offset, total: 12 })
  assert.equal(renderer.assemble(slice(0, 'AAAA')), null)
  assert.equal(renderer.assemble(slice(4, 'BBBB')), null)
  assert.deepEqual(renderer.assemble(slice(8, 'CCCC')), { key: 'r1', resource_id: 'a'.repeat(64), mime_type: 'image/png', data_base64: 'AAAABBBBCCCC' })
  assert.equal(renderer.slices.size, 0)
  assert.equal(renderer.assemble(slice(0, 'AAAA')), null)
  assert.throws(() => renderer.assemble(slice(8, 'CCCC')), /slice out of order/)
  const whole = { key: 'r2', resource_id: 'b'.repeat(64), mime_type: 'image/png', data_base64: 'AAAA' }
  assert.equal(renderer.assemble(whole), whole)
})

test('MP-08: Page Up/Page Down scroll the viewer natively (no preventDefault); other navigation keys go to the kernel', () => {
  const iframe = { setAttribute: () => {}, addEventListener: () => {}, style: {} }, container = { ownerDocument: { createElement: () => iframe }, append: () => {} }
  const sent: unknown[] = []
  const renderer = new BrowserMirror2Renderer(container as unknown as HTMLElement, async action => { sent.push(action) }) as unknown as { bind(doc: unknown): void; documentId: string }
  const handlers = new Map<string, (event: unknown) => void>()
  renderer.bind({ addEventListener: (type: string, fn: (event: unknown) => void) => handlers.set(type, fn), getSelection: () => null })
  renderer.documentId = 'd'
  const press = (key: string) => { let prevented = false; handlers.get('keydown')!({ key, shiftKey: false, ctrlKey: false, metaKey: false, altKey: false, preventDefault: () => { prevented = true } }); return prevented }
  assert.equal(press('PageDown'), false); assert.equal(press('PageUp'), false)
  assert.equal(press('Tab'), true)
  assert.deepEqual(sent, [{ kind: 'key', key: 'Tab' }])
})

test('MP-08/MP-10: replies that never apply keep the failure streak, so the mirror ends (video fallback) instead of retrying forever', async () => {
  let sequence = 0
  const f = flow(async command => command.op === 'mirror_subscribe' ? { subscription_id: 's' } : command.op === 'mirror_close' ? { closed: true } : packet(++sequence, true))
  f.failAlways(); const mirror = await f.start(400)
  await until(() => f.failures.length > 0, 3000)
  await mirror.close()
  assert.equal(f.failures.length, 1, 'MP-08: a renderer that cannot apply any snapshot reaches the terminal failure')
  assert.deepEqual(f.applied, [])
})
