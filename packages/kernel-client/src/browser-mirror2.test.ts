// MP-08/MP-10/MP-11: protocol 482 mirror v2 — fail closed before DOM construction.
import test from 'node:test'
import assert from 'node:assert/strict'
import { gzipSync } from 'node:zlib'
import { validateMirror2Packet, validateMirror2Record, validateMirror2Css, mirror2SandboxCsp, decodeMirror2Records, resolveMirror2Sheets } from './browser-mirror2-security.js'
import { inflateMirror2Packet, browserMirror2MinimumProtocolVersion, attachBrowserMirror2, BrowserMirror2Renderer } from './browser-mirror2.js'
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
  assert.equal(browserMirror2MinimumProtocolVersion, 482)
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
})

test('MP-10: gzip packet bodies inflate exactly and refuse lying sizes', async () => {
  const packet = base(tree(element('p', {}, {})))
  const { resources, tiles, ...body } = packet
  const json = Buffer.from(JSON.stringify(body))
  const wire = { wire: 2 as const, encoding: 'gzip', packet_bytes: json.length, packet_base64: gzipSync(json).toString('base64'), resources, tiles }
  assert.deepEqual(await inflateMirror2Packet(wire), packet)
  await assert.rejects(inflateMirror2Packet({ ...wire, packet_bytes: json.length - 1 }), /mirror packet bounds/)
  await assert.rejects(inflateMirror2Packet({ ...wire, packet_bytes: json.length + 1 }), /mirror packet bounds/)
  await assert.rejects(inflateMirror2Packet({ ...wire, encoding: 'br' }), /mirror packet bounds/)
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
  let failNext = false
  const renderer = () => ({ frame: {} as HTMLIFrameElement, ready: async () => {}, close: () => {}, apply: async (packet: Mirror2Packet) => { if (failNext) { failNext = false; throw Error('MP-11: nested mirror unavailable') } applied.push(packet.sequence) } })
  const requests: Array<Record<string, unknown>> = []
  const transport = { protocolVersion: 482, request: async (request: unknown) => { const command = (request as { KernelBrowser: { command: Record<string, unknown> } }).KernelBrowser.command; requests.push(command); return { KernelBrowser: { result: await script(command) } } } }
  return { applied, failures, requests, failOnce: () => { failNext = true }, start: () => attachBrowserMirror2(transport, {} as HTMLElement, { tab_id: 't', generation: 1, device_scale_factor: 1 }, { failure: error => failures.push(error) }, { credits: 1, waitMs: 0, renderer }) }
}
const packet = (sequence: number, reset: boolean, base: number | null) => ({ wire: 2, subscription_id: 's', tab_id: 't', generation: 1, document_id: 'd', sequence, base_sequence: base, reset, css_width: 1280, css_height: 800, device_scale_factor: 1, scroll: [0, 0], focused: null, selection: null, resources: [], tiles: [], ...(reset ? { root: 'n1', nodes: [[1, 0, 1], [1, 1, 'html']], ops: [] } : { ops: [] }) })
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
