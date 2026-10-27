// MP-08/MP-10/MP-11: protocol 489 mirror v2 — fail closed before DOM construction.
import test from 'node:test'
import assert from 'node:assert/strict'
import { createDeflateRaw, constants as zlib } from 'node:zlib'
import { setFlagsFromString } from 'node:v8'
import { runInNewContext } from 'node:vm'
import { validateMirror2Packet, validateMirror2Record, validateMirror2Css, mirror2SandboxCsp, decodeMirror2Records, resolveMirror2Sheets } from './browser-mirror2-security.js'
import { Mirror2Inflater, browserMirror2MinimumProtocolVersion, attachBrowserMirror2, BrowserMirror2Renderer } from './browser-mirror2.js'
import { RelayFrameReceiver } from './relay-frame-transport.js'
import type { Mirror2Packet, Mirror2Record } from './browser-mirror2-types.js'

// MP-08/MP-10/MP-11: these scalar/queue fixtures start with a painted surface;
// staged-document readiness and event retirement use the real-Chromium suite.
function paintedRenderer(container: HTMLElement, send: ConstructorParameters<typeof BrowserMirror2Renderer>[1]): BrowserMirror2Renderer {
  container.ownerDocument.addEventListener ??= () => {}
  container.ownerDocument.removeEventListener ??= () => {}
  const renderer = new BrowserMirror2Renderer(container, send)
  ;(renderer as unknown as { navigation: { painted: boolean } }).navigation.painted = true
  const frame = renderer.frame as HTMLIFrameElement & { removeEventListener?: unknown }
  frame.removeEventListener ??= () => {}
  // Real frames share the registering document; scalar fixtures must expose
  // its pointer-listener cleanup surface too.
  Object.defineProperty(frame, 'ownerDocument', { value: frame.ownerDocument ?? container.ownerDocument, configurable: true })
  frame.ownerDocument.removeEventListener ??= () => {}
  return renderer
}


const base = (nodes: Mirror2Record[], extra: Partial<Mirror2Packet> = {}): Mirror2Packet => ({ wire: 2, subscription_id: 's', tab_id: 't', generation: 1, document_id: 'd', sequence: 1, base_sequence: null, reset: true, root: nodes[0]!.id, nodes, ops: [], scroll: [0, 0], focused: null, selection: null, resources: [], tiles: [], css_width: 1280, css_height: 800, device_scale_factor: 1, ...extra })
const tree = (...rest: Mirror2Record[]): Mirror2Record[] => [{ id: 'n1', parent: null, kind: 'document' }, { id: 'n2', parent: 'n1', kind: 'element', tag: 'html' }, ...rest]
const element = (tag: string, attrs: Record<string, string> = {}, extra: Partial<Mirror2Record> = {}): Mirror2Record => ({ id: 'n3', parent: 'n2', kind: 'element', tag, attrs, ...extra })

test('MP-10: protocol 629 mirror accepts the full shared resize range', () => {
  for (const [css_width, css_height] of [[320, 200], [900, 600], [1920, 1080]] as const)
    assert.doesNotThrow(() => validateMirror2Packet(base(tree(), { css_width, css_height }), new Map()))
  for (const [css_width, css_height] of [[319, 200], [320, 199], [1921, 1080], [1920, 1081], [900.5, 600]] as const)
    assert.throws(() => validateMirror2Packet(base(tree(), { css_width, css_height }), new Map()), /mirror packet bounds/)
})

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
  assert.equal(browserMirror2MinimumProtocolVersion, 550)
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
  // Protocol 492: UA state is a closed set of booleans on elements; the fragment is a bounded string.
  validateMirror2Packet(delta([{ op: 'state', id: 'n2', state: { popover_open: true, defined: false } }]), previous)
  for (const state of [{ open: true }, { modal: 'yes' }, null, []]) assert.throws(() => validateMirror2Packet(delta([{ op: 'state', id: 'n2', state } as unknown as NonNullable<Mirror2Packet['ops']>[number]]), previous), /unsafe mirror op/, JSON.stringify(state))
  assert.throws(() => validateMirror2Packet(delta([{ op: 'state', id: 'n5', state: { modal: true } }]), previous), /unsafe mirror op/)
  assert.throws(() => validateMirror2Record({ id: 'n3', parent: 'n2', kind: 'text', text: 'x', state: { defined: true } } as Mirror2Record), /unsafe mirror/)
  // Protocol 501: custom validity is one more boolean; a message never travels.
  validateMirror2Record({ id: 'n3', parent: 'n2', kind: 'element', tag: 'input', state: { custom_invalid: true } })
  assert.throws(() => validateMirror2Record({ id: 'n3', parent: 'n2', kind: 'element', tag: 'input', state: { custom_invalid: 'too short' } } as unknown as Mirror2Record), /unsafe mirror/)
  validateMirror2Packet({ ...delta([]), fragment: 'section-2' }, previous)
  assert.throws(() => validateMirror2Packet({ ...delta([]), fragment: 'x'.repeat(2049) }, previous), /unsafe mirror header/)
  // Protocol 503: a frame document's fragment, on document records and `fragment` ops only, bounded like the header.
  validateMirror2Record({ id: 'n9', parent: 'n8', kind: 'document', fragment: 'section' })
  for (const record of [{ id: 'n9', parent: 'n8', kind: 'document', fragment: '' }, { id: 'n9', parent: 'n8', kind: 'document', fragment: 'x'.repeat(2049) }, { id: 'n9', parent: 'n8', kind: 'element', tag: 'p', fragment: 'section' }]) assert.throws(() => validateMirror2Record(record as Mirror2Record), /unsafe mirror/, JSON.stringify(record).slice(0, 80))
  validateMirror2Packet(delta([{ op: 'fragment', id: 'n1', fragment: 'section' }, { op: 'fragment', id: 'n1', fragment: null }]), previous)
  for (const fragment of ['', 'x'.repeat(2049), 7]) assert.throws(() => validateMirror2Packet(delta([{ op: 'fragment', id: 'n1', fragment } as unknown as NonNullable<Mirror2Packet['ops']>[number]]), previous), /unsafe mirror op/)
  assert.throws(() => validateMirror2Packet(delta([{ op: 'fragment', id: 'n2', fragment: 'section' }]), previous), /unsafe mirror op/, 'MP-11: an element has no fragment')
  assert.throws(() => validateMirror2Packet(base(tree(element('p', { 'chariox-target': '' }))), new Map()), /unsafe mirror/)
  assert.throws(() => validateMirror2Packet(base(tree(element('x-card', { 'chariox-undefined': '' }))), new Map()), /unsafe mirror/)
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
  const rows = [[1, 0, 1], [1, 1, 'html', { lang: 'en' }], [1, 1, 'body'], [2, 1, 0, 'Hello'], [1, 2, 4, 0, { size: [10, 20], display: 'inline-block', tag: 'span' }], [1, 3, 'svg', { viewBox: '0 0 1 1' }, { ns: 'svg' }], [999999994, 4, 3, 0, { tag: 'iframe' }], [1, 1, 1, 0, { fragment: 'section' }]]
  assert.deepEqual(decodeMirror2Records(rows, null), [
    { id: 'n1', parent: null, kind: 'document' },
    { id: 'n2', parent: 'n1', kind: 'element', tag: 'html', attrs: { lang: 'en' } },
    { id: 'n3', parent: 'n2', kind: 'element', tag: 'body' },
    { id: 'n5', parent: 'n3', kind: 'text', text: 'Hello' },
    { size: [10, 20], display: 'inline-block', tag: 'span', id: 'n6', parent: 'n3', kind: 'mask' },
    { ns: 'svg', id: 'n7', parent: 'n3', kind: 'element', tag: 'svg', attrs: { viewBox: '0 0 1 1' } },
    { tag: 'iframe', id: 'n1000000001', parent: 'n3', kind: 'frame' },
    { fragment: 'section', id: 'n1000000002', parent: 'n1000000001', kind: 'document' },
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
let flowProtocol = 550
const flow = (script: (command: Record<string, unknown>) => Promise<unknown>) => {
  const applied: number[] = [], failures: unknown[] = [], fontNegotiations: boolean[] = []
  let failNext = false, always = false
  const packets: Mirror2Packet[] = []
  const renderer = () => ({ requireNativeFonts: (value: boolean) => { fontNegotiations.push(value) }, frame: {} as HTMLIFrameElement, ready: async () => {}, close: () => {}, apply: async (packet: Mirror2Packet) => { if (failNext) { failNext = always; throw Error('MP-11: nested mirror unavailable') } applied.push(packet.sequence); packets.push(packet) } })
  const requests: Array<Record<string, unknown>> = []
  const transport = { protocolVersion: flowProtocol, request: async (request: unknown) => { const command = (request as { KernelBrowser: { command: Record<string, unknown> } }).KernelBrowser.command; requests.push(command); return { KernelBrowser: { result: await script(command) } } } }
  return { applied, packets, failures, requests, fontNegotiations, failOnce: () => { failNext = true }, failAlways: () => { failNext = true; always = true }, start: (failingMs?: number) => attachBrowserMirror2(transport, {} as HTMLElement, { tab_id: 't', generation: 1, device_scale_factor: 1 }, { failure: error => failures.push(error) }, { credits: 1, waitMs: 0, renderer, ...(failingMs ? { failingMs } : {}) }) }
}
// Protocol 489: a reset carries the binding and header; a delta only what changed.
const packet = (sequence: number, reset: boolean, _base?: number | null) => reset ? { wire: 2, subscription_id: 's', tab_id: 't', generation: 1, document_id: 'd', sequence, base_sequence: null, reset, css_width: 1280, css_height: 800, device_scale_factor: 1, scroll: [0, 0], focused: null, selection: null, root: 'n1', nodes: [[1, 0, 1], [1, 1, 'html']], ops: [] } : { wire: 2, sequence, ops: [] }
const until = async (check: () => boolean, ms = 3000) => { const end = Date.now() + ms; while (!check() && Date.now() < end) await new Promise(resolve => setTimeout(resolve, 10)) }

test('MP-08/MP-10/MP-11 the renderer requires native font certificates only after acknowledgement', async () => {
 for (const acknowledged of [undefined, true]) {
  const f = flow(async command => command.op === 'mirror_subscribe' ? { subscription_id: 's', ...(acknowledged ? { font_requirements: true } : {}) } : command.op === 'mirror_close' ? { closed: true } : new Promise(() => {}))
  const mirror = await f.start()
  assert.equal(f.requests[0]!.font_requirements, true)
  assert.deepEqual(f.fontNegotiations, [acknowledged === true])
  await mirror.close()
 }
})

test('MP-08/MP-10/MP-11 protocol 567 viewers ask for the absent font list, older kernels are not sent the field', async () => {
 for (const [protocol, asked] of [[550, false], [556, false], [567, true], [600, true]] as const) {
  flowProtocol = protocol
  try {
   const f = flow(async command => command.op === 'mirror_subscribe' ? { subscription_id: 's', font_requirements: true } : command.op === 'mirror_close' ? { closed: true } : new Promise(() => {}))
   const mirror = await f.start()
   assert.equal(f.requests[0]!.font_absent, asked ? true : undefined, `protocol ${protocol}`)
   await mirror.close()
  } finally { flowProtocol = 550 }
 }
})

test('MP-08/MP-10/MP-11 a 550 kernel types every v2 field: a subscription refusal is never retried without one', async () => {
 for (const message of ['unknown field `font_requirements`', 'MP-11: mirror font negotiation', 'permission denied', 'unknown field `navigation_preview`']) {
  let calls = 0
  const refused = flow(async () => { calls++; throw Error(message) })
  await assert.rejects(refused.start(), error => error instanceof Error && error.message === message)
  assert.equal(calls, 1, 'no schema fallback')
 }
})

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

// Round 10 hosted drop drill: a plain re-credit after a failed credit waited behind the kernel's long poll for the
// lost credit and then for the 1.5 s resend (3.25 s after reconnect); round 10 reset instead (a full snapshot).
// Round 11 (protocol 501): a resend credit ends that poll too and gets the packet it issued, without a snapshot.
test('MP-10: protocol 501 a failed credit asks for a resend (it ends the kernel long poll of the lost credit), not a reset; the stream continues', async () => {
  let sequence = 0, failures = 0
  const viewer = flow(async command => {
    if (command.op === 'mirror_subscribe') return { subscription_id: 's' }
    if (command.op === 'mirror_close') return { closed: true }
    if (command.after_sequence === 0) return packet(++sequence, true)
    if (failures < 1 && sequence === 2) { failures++; ++sequence; throw Error('local_transport_error: connection closed') } // packet 3 issued, its reply lost
    if (command.resend) return packet(command.after_sequence as number + 1, false)
    if (sequence >= 4) await new Promise(resolve => setTimeout(resolve, 200))
    return packet(++sequence, false)
  })
  const view = await viewer.start()
  await until(() => viewer.applied.length >= 4)
  await view.close()
  const credits = viewer.requests.filter(command => command.op === 'mirror_next'), lost = credits.findIndex(command => command.after_sequence === 2)
  assert.deepEqual([credits[lost + 1]!.after_sequence, credits[lost + 1]!.resend], [2, true], `MP-10: the credit after the failed one is a resend: ${JSON.stringify(credits)}`)
  assert.equal(credits.filter(command => command.after_sequence === 0).length, 1, 'MP-10: no second snapshot')
  assert.deepEqual(viewer.applied.slice(0, 4), [1, 2, 3, 4])
})

test('MP-10: removing an inline style forgets it, so a later resource cannot restore it', () => {
  const element = () => { const attributes = new Map<string, string>(); return { nodeType: 1, attributes, setAttribute: (name: string, value: string) => attributes.set(name, value), removeAttribute: (name: string) => attributes.delete(name), addEventListener: () => {}, style: {} } }
  const iframe = element(), container = { ownerDocument: { createElement: () => iframe }, append: () => {} }
  const renderer = paintedRenderer(container as unknown as HTMLElement, async () => {}) as unknown as { dom: Map<string, unknown>; records: Map<string, Mirror2Record>; styled: Map<string, { keys: string[] }>; op(op: unknown, scrolls: unknown[]): void }
  const node = element(); renderer.dom.set('n5', node); renderer.records.set('n5', { id: 'n5', parent: 'n2', kind: 'element', tag: 'div' })
  renderer.op({ op: 'attr', id: 'n5', name: 'style', value: 'background:url("mr:r1")' }, [])
  assert.deepEqual(renderer.styled.get('n5#style')?.keys, ['r1'])
  renderer.op({ op: 'attr', id: 'n5', name: 'style', value: null }, [])
  assert.equal(renderer.styled.has('n5#style'), false); assert.equal(node.attributes.has('style'), false)
})

test('MP-08/MP-10: compact deltas inherit binding and header from their base; deflated bodies inflate in sequence order; late copies are not gaps', async () => {
  const kernel = kernelContext()
  const reset = { ...packet(1, true), snapshot_complete: false, font_failures:['a'.repeat(64)], scroll: [0, 40], focused: 'n2' }
  const bodies = [reset, { wire: 2, sequence: 2, ops: [{ op: 'text', id: 'n2', text: 'x'.repeat(600) }] }, { wire: 2, sequence: 3, snapshot_complete: true, font_failures:[], focused: null, ops: [{ op: 'text', id: 'n2', text: 'x'.repeat(600) }] }, { wire: 2, sequence: 4, ops: [] }]
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
  assert.deepEqual(f.packets.map(p => p.snapshot_complete), [false, false, true, true], '539 completion changes survive compression, ordering and omitted compact headers')
  assert.deepEqual(f.packets.map(p => p.font_failures), [['a'.repeat(64)],['a'.repeat(64)],[],[]], 'native font failures inherit and clear through compact deflated deltas')
  assert.deepEqual([second!.subscription_id, second!.tab_id, second!.document_id, second!.base_sequence, second!.reset, second!.scroll, second!.focused], ['s', 't', 'd', 1, false, [0, 40], 'n2'])
  assert.equal(third!.focused, null, 'an explicit null replaces the base value'); assert.equal(fourth!.focused, null); assert.deepEqual(fourth!.resources, [])
})

test('MP-10: resource slices reassemble in order; a reset restarts them', () => {
  const iframe = { setAttribute: () => {}, addEventListener: () => {}, style: {} }, container = { ownerDocument: { createElement: () => iframe }, append: () => {} }
  const renderer = paintedRenderer(container as unknown as HTMLElement, async () => {}) as unknown as { assemble(r: Record<string, unknown>): Record<string, unknown> | null; slices: Map<string, unknown> }
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

test('MP-08: Page Up/Page Down scroll the viewer natively (no preventDefault); other navigation keys go to the kernel', async () => {
  const iframe = { setAttribute: () => {}, addEventListener: () => {}, style: {} }, container = { ownerDocument: { createElement: () => iframe }, append: () => {} }
  const sent: unknown[] = []
  const renderer = paintedRenderer(container as unknown as HTMLElement, async action => { sent.push(action) }) as unknown as { bind(doc: unknown): void; documentId: string }
  const handlers = new Map<string, (event: unknown) => void>()
  renderer.bind({ defaultView: { frameElement: iframe }, addEventListener: (type: string, fn: (event: unknown) => void) => handlers.set(type, fn), getSelection: () => null })
  renderer.documentId = 'd'
  const press = (key: string, shiftKey = false) => { let prevented = false; handlers.get('keydown')!({ key, shiftKey, ctrlKey: false, metaKey: false, altKey: false, preventDefault: () => { prevented = true } }); return prevented }
  assert.equal(press('PageDown'), false); assert.equal(press('PageUp'), false)
  assert.equal(press('Tab'), true)
  // Review #941-2: Shift extends the kernel selection (and Shift+Tab moves focus back).
  for (const key of ['Tab', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End', 'Enter']) assert.equal(press(key, true), true)
  await new Promise(resolve => setTimeout(resolve, 0)) // inputs are sent in order
  assert.deepEqual(sent.map(action => (action as { key: string }).key), ['Tab', 'Shift+Tab', 'Shift+ArrowLeft', 'Shift+ArrowRight', 'Shift+ArrowUp', 'Shift+ArrowDown', 'Shift+Home', 'Shift+End', 'Enter'])
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

test('MP-10: review #320-1 a select shows its live selection after its options are built (snapshot and children delta)', () => {
  // Fake DOM with the real select rule: selectedIndex is clamped to the options present, and the first option appended to an empty select becomes selected.
  type Fake = { nodeType: number; localName: string; childNodes: Fake[]; attributes: Map<string, string>; selectedIndex?: number; value?: string; setAttribute(n: string, v: string): void; removeAttribute(n: string): void; appendChild(c: Fake): Fake; addEventListener(): void; style: { setProperty(): void }; getRootNode(): { activeElement: null } }
  const make = (tag: string): Fake => {
    const node: Fake = { nodeType: 1, localName: tag, childNodes: [], attributes: new Map(), setAttribute: (n, v) => { node.attributes.set(n, v) }, removeAttribute: n => { node.attributes.delete(n) }, addEventListener: () => {}, style: { setProperty: () => {} }, getRootNode: () => ({ activeElement: null }),
      appendChild: child => { node.childNodes.push(child); if (tag === 'select' && child.localName === 'option' && index < 0) index = 0; return child } }
    let index = -1
    if (tag === 'select') Object.defineProperty(node, 'selectedIndex', { get: () => index, set: (v: number) => { index = v >= 0 && v < node.childNodes.length ? v : -1 } })
    return node
  }
  const doc = { createElement: make, createElementNS: (_: string, tag: string) => make(tag), createTextNode: () => make('#text'), createDocumentFragment: () => make('#fragment') }
  const iframe = make('iframe'), container = { ownerDocument: { createElement: () => iframe }, append: () => {} }
  const renderer = paintedRenderer(container as unknown as HTMLElement, async () => {}) as unknown as { build(records: Mirror2Record[], doc: unknown, frames: unknown[], scrolls: unknown[]): Map<string, Fake> }
  const form = { value: 'b', checked: false, selected_index: 1, selection_start: null, selection_end: null }
  for (const id of ['n3', 'n13']) {
    const built = renderer.build([{ id, parent: null, kind: 'element', tag: 'select', form }, { id: id + '1', parent: id, kind: 'element', tag: 'option' }, { id: id + '2', parent: id, kind: 'element', tag: 'option' }], doc, [], [])
    assert.equal(built.get(id)?.selectedIndex, 1, 'MP-10: the live selection, not the first option')
  }
})

test('MP-08: review #320-2 a terminal application failure followed by close releases the kernel subscription exactly once', async () => {
  let sequence = 0
  const f = flow(async command => command.op === 'mirror_subscribe' ? { subscription_id: 's' } : command.op === 'mirror_close' ? { closed: true } : packet(++sequence, true))
  f.failAlways(); const mirror = await f.start(400)
  await until(() => f.failures.length > 0, 3000)
  await mirror.close(); await mirror.close()
  assert.equal(f.failures.length, 1)
  assert.equal(f.requests.filter(command => command.op === 'mirror_close').length, 1, 'MP-08: one mirror_close frees the subscription, its observer and the shared budget')
})

test('MP-10: review #941-2 a viewer scroll inside a rendered open shadow root reaches the kernel', async () => {
  const listeners = new Map<string, (event: unknown) => void>()
  const root = { addEventListener: (type: string, fn: (event: unknown) => void) => listeners.set(type, fn), replaceChildren: () => {} }
  const iframe = { setAttribute: () => {}, addEventListener: () => {}, style: {} }, container = { ownerDocument: { createElement: () => iframe }, append: () => {} }
  const sent: unknown[] = []
  const renderer = paintedRenderer(container as unknown as HTMLElement, async action => { sent.push(action) }) as unknown as { attachShadow(host: unknown, record: Mirror2Record, fragment: unknown): void; ids: WeakMap<object, string>; documentId: string }
  renderer.documentId = 'd'
  renderer.attachShadow({ shadowRoot: null, attachShadow: () => root }, { id: 'n7', parent: 'n6', kind: 'shadow' }, { childNodes: [] })
  const scroller = { nodeType: 1, scrollLeft: 0, scrollTop: 300 }; renderer.ids.set(scroller, 'n9')
  const raf = globalThis.requestAnimationFrame
  globalThis.requestAnimationFrame = (fn: FrameRequestCallback) => setTimeout(() => fn(0), 0) as unknown as number
  try {
    listeners.get('scroll')?.({ target: scroller })
    await new Promise(resolve => setTimeout(resolve, 10))
  } finally { globalThis.requestAnimationFrame = raf }
  assert.deepEqual(sent, [{ kind: 'scroll_to', node_id: 'n9', x: 0, y: 300 }])
})

// A small fake DOM for apply(): text controls take Chrome's rules (assigning a value puts the caret at its end; focus keeps it).
type FakeNode = { nodeType: number; localName: string; childNodes: FakeNode[]; parentNode: FakeNode | null; [key: string]: unknown }
function fakeDocument(host: Record<string, unknown> = { getAnimations: () => [], addEventListener: () => {}, removeEventListener: () => {} }): { doc: FakeNode; container: unknown } {
  const doc = { nodeType: 9, localName: '#document', childNodes: [], parentNode: null, activeElement: null, adoptedStyleSheets: [], addEventListener: () => {}, getSelection: () => null } as unknown as FakeNode & { activeElement: FakeNode | null }
  const make = (tag: string, nodeType = 1): FakeNode => {
    let value = ''
    const node: FakeNode = { nodeType, localName: tag, childNodes: [], parentNode: null, ownerDocument: doc, attributes: new Map(), style: { setProperty: () => {} }, dataset: {}, shadowRoot: null, selection: null,
      setAttribute: (n: string, v: string) => (node.attributes as Map<string, string>).set(n, v), removeAttribute: (n: string) => (node.attributes as Map<string, string>).delete(n), addEventListener: (type: string, fn: () => void) => { if (type === 'load') setTimeout(fn, 0) },
      appendChild: (child: FakeNode) => { child.parentNode?.childNodes.splice(child.parentNode.childNodes.indexOf(child), 1); child.parentNode = node; node.childNodes.push(child); return child },
      insertBefore: (child: FakeNode, ref: FakeNode | null) => { child.parentNode?.childNodes.splice(child.parentNode.childNodes.indexOf(child), 1); child.parentNode = node; node.childNodes.splice(ref ? node.childNodes.indexOf(ref) : node.childNodes.length, 0, child); return child },
      removeChild: (child: FakeNode) => { node.childNodes.splice(node.childNodes.indexOf(child), 1); child.parentNode = null; return child },
      prepend: (child: FakeNode) => (node.insertBefore as (c: FakeNode, r: FakeNode | null) => FakeNode)(child, node.childNodes[0] ?? null),
      replaceWith: (other: FakeNode) => { (doc as unknown as { documentElement: FakeNode }).documentElement = other; other.parentNode = doc },
      getRootNode: () => doc, matches: () => false, closest: () => null,
      focus: () => { (doc as unknown as { activeElement: FakeNode }).activeElement = node },
      setSelectionRange: (start: number, end: number) => { node.selection = [start, end] } }
    if (tag === 'input') Object.defineProperty(node, 'value', { get: () => value, set: (next: string) => { value = next; node.selection = [next.length, next.length] } })
    return node
  }
  Object.assign(doc, { createElement: make, createElementNS: (_: string, tag: string) => make(tag), createTextNode: (text: string) => Object.assign(make('#text', 3), { data: text, textContent: text }), createDocumentFragment: () => make('#fragment', 11) })
  const html = make('html'), head = make('head'); html.childNodes.push(head); Object.assign(doc, { documentElement: html, head })
  const iframe = Object.assign(make('iframe'), { ownerDocument: { activeElement: null }, contentDocument: doc, contentWindow: { scrollTo: () => {}, scrollX: 0, scrollY: 0 }, remove: () => {} })
  return { doc, container: { ownerDocument: Object.assign(host, { createElement: () => iframe }), append: () => {} } } // the page hosting the mirror
}

test('MP-08/MP-10: the shared renderer leaves host animation policy to its client', async () => {
  const host = { getAnimations: () => { throw Error('host animations belong to the client') } }
  const renderer = paintedRenderer(fakeDocument(host).container as HTMLElement, async () => {})
  await renderer.ready(); renderer.close()
})

test('MP-08: review #320-1 a focused text control shows the kernel caret/range after focus (snapshot and a newly introduced field)', async () => {
  const { doc, container } = fakeDocument()
  const renderer = paintedRenderer(container as unknown as HTMLElement, async () => {})
  await renderer.ready()
  const form = (start: number, end: number) => ({ value: 'abc', checked: false, selected_index: -1, selection_start: start, selection_end: end })
  const nodes: Mirror2Record[] = [{ id: 'n1', parent: null, kind: 'document' }, { id: 'n2', parent: 'n1', kind: 'element', tag: 'html' }, { id: 'n3', parent: 'n2', kind: 'element', tag: 'body' },
    { id: 'n4', parent: 'n3', kind: 'element', tag: 'input', attrs: {}, form: form(0, 0) }]
  await renderer.apply(base(nodes, { focused: 'n4' }))
  const field = (id: string) => (doc as unknown as { documentElement: FakeNode }).documentElement.childNodes.flatMap(n => n.childNodes).find(n => (renderer as unknown as { ids: WeakMap<object, string> }).ids.get(n) === id)
  assert.deepEqual(field('n4')?.selection, [0, 0], 'MP-08: snapshot caret at the kernel position, not the value end')
  await renderer.apply({ ...base(nodes, { reset: false, base_sequence: 1, sequence: 2, focused: 'n5', ops: [{ op: 'children', id: 'n3', children: ['n4', 'n5'], nodes: [{ id: 'n5', parent: 'n3', kind: 'element', tag: 'input', attrs: {}, form: form(1, 2) }] }] }), nodes: undefined, root: undefined } as unknown as Mirror2Packet)
  assert.deepEqual(field('n5')?.selection, [1, 2], 'MP-08: a newly introduced focused field keeps the kernel range')
})

test('MP-10: review #320-2 a removed frame releases its nested records and a retired document its binding', async () => {
  const { doc, container } = fakeDocument()
  type Internals = { ready(): Promise<void>; bind(doc: unknown): void; op(op: unknown, scrolls: unknown[]): void; dom: Map<string, unknown>; records: Map<string, Mirror2Record>; ids: WeakMap<object, string> }
  const renderer = paintedRenderer(container as unknown as HTMLElement, async () => {}) as unknown as Internals
  await renderer.ready()
  const make = doc.createElement as (tag: string) => FakeNode
  const nested = fakeDocument().doc, body = make('body'), frame = make('iframe'), html = make('html'), text = make('#text')
  Object.defineProperty(frame, 'contentDocument', { get: () => frame.parentNode ? nested : null }) // a detached iframe has no document
  nested.childNodes.push(html); html.childNodes.push(text); (body.appendChild as (c: FakeNode) => FakeNode)(frame)
  const known: Array<[string, object, Mirror2Record]> = [['n3', body, { id: 'n3', parent: 'n2', kind: 'element', tag: 'body' }], ['n5', frame, { id: 'n5', parent: 'n3', kind: 'frame', tag: 'iframe' }],
    ['n6', nested, { id: 'n6', parent: 'n5', kind: 'document' }], ['n7', html, { id: 'n7', parent: 'n6', kind: 'element', tag: 'html' }], ['n8', text, { id: 'n8', parent: 'n7', kind: 'text', text: 'x' }]]
  for (const [id, node, record] of known) { renderer.dom.set(id, node); renderer.ids.set(node, id); renderer.records.set(id, record) }
  renderer.op({ op: 'children', id: 'n3', children: [], nodes: [] }, [])
  assert.deepEqual(['n5', 'n6', 'n7', 'n8'].filter(id => renderer.records.has(id) || renderer.dom.has(id)), [], 'MP-10: the frame and its document are forgotten')
  setFlagsFromString('--expose-gc'); const gc = runInNewContext('gc') as () => void
  let retired!: WeakRef<object>
  ;(() => { const old = { addEventListener: () => {}, getSelection: () => null }; renderer.bind(old); retired = new WeakRef(old) })()
  for (let i = 0; i < 3 && retired.deref(); i++) { await new Promise(resolve => setTimeout(resolve, 0)); gc() }
  assert.equal(retired.deref(), undefined, 'MP-10: a bound document no longer in use can be collected')
})

// Round 10 hosted drop drill: a dropped socket fails every credit in flight at once. That is one failure for the
// backoff (no 2 s wait), and a reset: only a reset ends the long polls the kernel still holds for the lost credits.
test('MP-10: credits that fail together (a dropped socket) count as one failure: the resend follows after one backoff step', async () => {
  let sequence = 0, dropped = false, released: Array<() => void> = []
  const applied: number[] = [], requests: Array<Record<string, unknown>> = []
  const renderer = () => ({ frame: {} as HTMLIFrameElement, ready: async () => {}, close: () => {}, apply: async (p: Mirror2Packet) => { applied.push(p.sequence) } })
  const transport = { protocolVersion: 550, request: async (request: unknown) => {
    const command = (request as { KernelBrowser: { command: Record<string, unknown> } }).KernelBrowser.command; requests.push({ ...command, at: Date.now() })
    if (command.op === 'mirror_subscribe') return { KernelBrowser: { result: { subscription_id: 's' } } }
    if (command.op === 'mirror_close') return { KernelBrowser: { result: { closed: true } } }
    if (command.after_sequence === 0) return { KernelBrowser: { result: packet(++sequence, true) } }
    // Before the drop, credits wait (long poll); the drop fails all of them together.
    if (!dropped) { await new Promise<void>(resolve => released.push(resolve)); throw Error('local_transport_error: connection closed') }
    return { KernelBrowser: { result: packet(++sequence, false) } }
  } }
  const view = await attachBrowserMirror2(transport, {} as HTMLElement, { tab_id: 't', generation: 1, device_scale_factor: 1 }, { failure: () => {} }, { credits: 4, waitMs: 0, renderer })
  await until(() => applied.length >= 1)
  await until(() => released.length >= 1, 1000)
  const droppedAt = Date.now(); dropped = true; for (const release of released.splice(0)) release()
  await until(() => applied.length >= 2, 3000)
  await view.close()
  const after = requests.filter(c => c.op === 'mirror_next' && (c.at as number) > droppedAt)
  assert.ok(after.length > 0 && (after[0]!.at as number) - droppedAt < 600, `MP-10: re-credit within one backoff step (${after.length ? (after[0]!.at as number) - droppedAt : 'none'} ms)`)
  assert.deepEqual([after[0]!.after_sequence, after[0]!.resend], [1, true], 'MP-10: the first credit after the drop is the resend that ends the lost long polls')
})

// r47 quiet-host GitHub drill (8 Mbit/s): small deltas overtook a 420 KB reset on the relay. Each one was buffered
// (its base not applied) yet released its credit, so the kernel answered credits past its 8-packet history while the
// reset was still arriving, and reset again: ~70 full 420 KB snapshots a minute and the page never left video.
test('MP-08/MP-10: packets that overtake a slow packet hold their credit until applied: the window stays bounded', async () => {
  let sequence = 0, slow = true, issuedBehind = 0
  const applied: number[] = [], requests: Array<Record<string, unknown>> = []
  const renderer = () => ({ frame: {} as HTMLIFrameElement, ready: async () => {}, close: () => {}, apply: async (p: Mirror2Packet) => { applied.push(p.sequence) } })
  const transport = { protocolVersion: 550, receiving: () => slow, request: async (request: unknown) => {
    const command = (request as { KernelBrowser: { command: Record<string, unknown> } }).KernelBrowser.command; requests.push(command)
    if (command.op === 'mirror_subscribe') return { KernelBrowser: { result: { subscription_id: 's' } } }
    if (command.op === 'mirror_close') return { KernelBrowser: { result: { closed: true } } }
    if (command.after_sequence === 0) return { KernelBrowser: { result: packet(++sequence, true) } }
    const own = ++sequence
    if ((command.after_sequence as number) < sequence - 8) issuedBehind++
    // Packet 2 is large: it crosses the link while the small ones behind it arrive at once.
    if (own === 2) await until(() => !slow, 5000); else await new Promise(resolve => setTimeout(resolve, 5))
    return { KernelBrowser: { result: packet(own, false) } }
  } }
  const view = await attachBrowserMirror2(transport, {} as HTMLElement, { tab_id: 't', generation: 1, device_scale_factor: 1 }, { failure: () => {} }, { credits: 4, waitMs: 0, renderer })
  await until(() => applied.length >= 1)
  await new Promise(resolve => setTimeout(resolve, 300))
  const during = sequence; slow = false
  await until(() => applied.length >= 6)
  await view.close()
  assert.ok(during <= 5, `MP-10: at most the credit window is answered past the applied base while packet 2 arrives (${during - 1})`)
  assert.equal(issuedBehind, 0, 'MP-10: no credit falls behind the kernel history')
  assert.deepEqual(applied.slice(0, 6), [1, 2, 3, 4, 5, 6], 'MP-08: packets apply in order without a reset')
})

// Round 11 hosted drop drill (b3, DPR2): a reply lost without a replay left a gap; the viewer waited 500 ms and reset
// (a full snapshot: 2.2 s after the drop). Protocol 501: the missing packet is asked for at once (resend).
test('MP-10: protocol 501 a gap (a reply lost with its socket) asks for the missing packet at once; it applies without a reset', async () => {
  let sequence = 0, gapAt = 0
  const applied: number[] = [], requests: Array<Record<string, unknown>> = []
  const renderer = () => ({ frame: {} as HTMLIFrameElement, ready: async () => {}, close: () => {}, apply: async (p: Mirror2Packet) => { applied.push(p.sequence) } })
  const transport = { protocolVersion: 550, request: async (request: unknown) => {
    const command = (request as { KernelBrowser: { command: Record<string, unknown> } }).KernelBrowser.command; requests.push({ ...command, at: Date.now() })
    if (command.op === 'mirror_subscribe') return { KernelBrowser: { result: { subscription_id: 's' } } }
    if (command.op === 'mirror_close') return { KernelBrowser: { result: { closed: true } } }
    if (command.after_sequence === 0) return { KernelBrowser: { result: packet(++sequence, true) } }
    if (command.resend) return { KernelBrowser: { result: packet(command.after_sequence as number + 1, false) } }
    if (sequence === 1) { ++sequence; return new Promise(() => {}) } // packet 2: its reply never arrives
    await new Promise(resolve => setTimeout(resolve, 30)); if (sequence === 2) gapAt = Date.now()
    return { KernelBrowser: { result: packet(++sequence, false) } }
  } }
  const view = await attachBrowserMirror2(transport, {} as HTMLElement, { tab_id: 't', generation: 1, device_scale_factor: 1 }, { failure: () => {} }, { credits: 2, waitMs: 0, renderer })
  await until(() => applied.includes(3), 3000)
  await view.close()
  const resend = requests.find(command => command.resend)
  assert.ok(resend, `MP-10: a resend credit: ${JSON.stringify(requests.map(c => [c.after_sequence, c.resend]))}`)
  assert.equal(resend.after_sequence, 1, 'MP-10: it names the base before the missing packet')
  assert.ok((resend.at as number) - gapAt < 200, `MP-10: at once (${(resend.at as number) - gapAt} ms after the gap)`)
  assert.deepEqual(applied.slice(0, 3), [1, 2, 3], 'MP-10: the missing packet applies, then the buffered one')
  assert.equal(requests.filter(command => command.after_sequence === 0).length, 1, 'MP-10: no second snapshot')
})

// r15 quiet-host drill (DPR1, typing every 1.2-1.7 s): the kernel starts a queued credit's wait when its predecessor is
// answered (bounded by arrival + 3 waits), but the viewer called the oldest credit lost waitMs + LOST_MS after it was SENT.
// Each spurious resend ended every waiting credit at once, so the next key's echo waited a round trip for a new credit.
test('MP-08: a queued credit answered within the kernel bound is not lost: no resend before its own wait has run', async () => {
  let sequence = 0, chain: Promise<unknown> = Promise.resolve()
  const requests: Array<Record<string, unknown>> = [], started = Date.now(), wait = 2000
  const renderer = () => ({ frame: {} as HTMLIFrameElement, ready: async () => {}, close: () => {}, apply: async () => {} })
  const transport = { protocolVersion: 550, request: async (request: unknown) => {
    const command = (request as { KernelBrowser: { command: Record<string, unknown> } }).KernelBrowser.command; requests.push({ ...command, at: Date.now() - started })
    if (command.op === 'mirror_subscribe') return { KernelBrowser: { result: { subscription_id: 's' } } }
    if (command.op === 'mirror_close') return { KernelBrowser: { result: { closed: true } } }
    if (command.after_sequence === 0) return { KernelBrowser: { result: packet(++sequence, true) } }
    // An idle page: each credit's wait starts when its predecessor is answered, never past arrival + 3 waits.
    const arrival = Date.now()
    const run = chain.then(() => new Promise(resolve => setTimeout(resolve, Math.max(0, Math.min(arrival + 3 * wait, Date.now() + wait) - Date.now())))).then(() => ({ KernelBrowser: { result: packet(++sequence, false) } }))
    chain = run; return run
  } }
  const view = await attachBrowserMirror2(transport, {} as HTMLElement, { tab_id: 't', generation: 1, device_scale_factor: 1 }, { failure: () => {} }, { waitMs: wait, renderer })
  await new Promise(resolve => setTimeout(resolve, 6500))
  await view.close()
  const resends = requests.filter(command => command.resend)
  assert.deepEqual(resends.map(command => command.at), [], `MP-08: no resend for credits the kernel still holds: ${JSON.stringify(requests.map(c => [c.op, c.after_sequence, c.resend, c.at]))}`)
})

// Round 11 final drop drill (b3, DPR2, 3.06 s): the transport replaced the dropped control socket without failing or
// replaying the credit in flight, so that credit (and the packet the kernel gave it) was orphaned until the 8 s request
// timeout; the gap only showed when the next heartbeat arrived 2.9 s later. The oldest outstanding credit is answered
// within its wait: one overdue by a second asks for a resend.
test('MP-10: protocol 501 an overdue oldest credit (orphaned by the transport) asks for a resend; its packet applies without waiting for a later one', async () => {
  let sequence = 0
  const applied: Array<[number, number]> = [], requests: Array<Record<string, unknown>> = [], started = Date.now()
  const renderer = () => ({ frame: {} as HTMLIFrameElement, ready: async () => {}, close: () => {}, apply: async (p: Mirror2Packet) => { applied.push([p.sequence, Date.now() - started]) } })
  const transport = { protocolVersion: 550, request: async (request: unknown) => {
    const command = (request as { KernelBrowser: { command: Record<string, unknown> } }).KernelBrowser.command; requests.push({ ...command, at: Date.now() - started })
    if (command.op === 'mirror_subscribe') return { KernelBrowser: { result: { subscription_id: 's' } } }
    if (command.op === 'mirror_close') return { KernelBrowser: { result: { closed: true } } }
    if (command.after_sequence === 0) return { KernelBrowser: { result: packet(++sequence, true) } }
    if (command.resend) return { KernelBrowser: { result: packet(command.after_sequence as number + 1, false) } }
    if (sequence === 1) { ++sequence; return new Promise(() => {}) } // its reply never arrives, and the request never fails
    await new Promise(resolve => setTimeout(resolve, 4000)); return { KernelBrowser: { result: packet(++sequence, false) } }
  } }
  const view = await attachBrowserMirror2(transport, {} as HTMLElement, { tab_id: 't', generation: 1, device_scale_factor: 1 }, { failure: () => {} }, { credits: 1, waitMs: 500, renderer })
  await until(() => applied.some(([s]) => s === 2), 3500)
  await view.close()
  const resend = requests.find(command => command.resend)
  assert.ok(resend, `MP-10: a resend credit: ${JSON.stringify(requests.map(c => [c.after_sequence, c.resend, c.at]))}`)
  assert.equal(resend.after_sequence, 1)
  const at = applied.find(([s]) => s === 2)?.[1]
  assert.ok(at !== undefined && at < 2500, `MP-10: the orphan's packet applied at ${at} ms (wait 500 ms)`)
})

// MP-11: protocol517 admits tile layout deltas through the same inert attribute/CSS guard.
test('MP-11: tile layout updates admit inert style/class and reject fetching CSS or handlers',()=>{
 const record: Mirror2Record={id:'n3',parent:'n2',kind:'tile',tag:'canvas',size:[80,40],attrs:{}};
 const known=new Map(tree(record).map(r=>[r.id,r]));
 const packet=(name:string,value:string)=>base(tree(record), {reset:false,base_sequence:1,sequence:2,ops:[{op:'attr',id:'n3',name,value}]});
 validateMirror2Packet(packet('class','moved'),known);validateMirror2Packet(packet('style','position:absolute;left:200px;display:block'),known);
 for(const [name,value]of [['onclick','alert(1)'],['src','https://example.com/x'],['style','background-image:url(https://example.com/x)']])assert.throws(()=>validateMirror2Packet(packet(name!,value!),known),/unsafe mirror/);
 const protectedKnown=new Map(known);protectedKnown.set('n3',{...record,kind:'mask'});assert.throws(()=>validateMirror2Packet(packet('class','moved'),protectedKnown),/unsafe mirror/);
});

// MP-08/MP-10/MP-11: host-safe causes retain terminal semantics after error sanitization.
test('specific retired-subscription replies retire the viewer immediately',async()=>{
 const f=flow(async c=>{if(c.op==='mirror_subscribe')return {subscription_id:'s'};if(c.op==='mirror_close')return {closed:true};throw Error('kernel_browser_failed: MD-2: host browser operation failed; refresh state or check host browser readiness (Error mirror_subscription_retired kernel-browser-mirror.mjs:31:4)')})
 const mirror=await f.start()
 try{await until(()=>f.failures.length>0,400);assert.equal(f.failures.length,1)}finally{await mirror.close()}
})

test('a refused input reports its reason without retiring paint or replaying the gesture',async()=>{
 let send:any,inputs=0,sequence=0;const refusals:unknown[]=[],failures:unknown[]=[]
 const error=Error('kernel_browser_failed: MD-2: browser operation failed: Error occluded_node')
 const mirror=await attachBrowserMirror2({protocolVersion:550,request:async(r:any)=>{
  const c=r.KernelBrowser.command
  if(c.op==='mirror_subscribe')return {KernelBrowser:{result:{subscription_id:'s'}}}
  if(c.op==='mirror_close')return {KernelBrowser:{result:{closed:true}}}
  if(c.op==='mirror_input'){inputs++;throw error}
  await new Promise(resolve=>setTimeout(resolve,10));return {KernelBrowser:{result:packet(++sequence,sequence===1)}}
 }},{} as HTMLElement,{tab_id:'t',generation:1,device_scale_factor:1},{failure:e=>failures.push(e),inputFailure:e=>refusals.push(e)},{credits:1,waitMs:0,renderer:(_c,s)=>{send=s;return {frame:{} as HTMLIFrameElement,ready:async()=>{},apply:async()=>{},close(){}}}})
 try{
  await until(()=>sequence>=1)
  await assert.rejects(send({kind:'click',node_id:'n2',x:1,y:1},{sequence:1,document_id:'d'}),error)
  const before=sequence;await until(()=>sequence>before)
  assert.equal(inputs,1);assert.deepEqual(refusals,[error]);assert.deepEqual(failures,[])
 }finally{await mirror.close()}
})

// MP-08/MP-10/MP-11: release the next input after the real write, not its
// round-trip reply; retain a bounded window and drop refusals without replay.
test('mirror input preserves write order with at most two unresolved replies',async()=>{
 const frame={style:{},dataset:{} as Record<string,string>,setAttribute(){},addEventListener(){},remove(){}},container={ownerDocument:{createElement:()=>frame},append(){}};
 const sent:Array<{action:any;written:(()=>void)|undefined;resolve:()=>void;reject:(e:Error)=>void}>=[];
 const renderer=paintedRenderer(container as unknown as HTMLElement,((action:any,_epoch:any,written?:()=>void)=>new Promise<void>((resolve,reject)=>sent.push({action,written,resolve,reject}))) as any) as any;
 renderer.documentId='d';renderer.sequence=1;
 for(const text of ['a','b','c'])renderer.enqueue({kind:'text',text});
 await until(()=>sent.length===1);assert.equal(sent.length,1);assert.equal(typeof sent[0]!.written,'function');
 sent[0]!.written!();await until(()=>sent.length===2);assert.equal(sent.length,2,'second input must not wait for the first reply');
 sent[1]!.written!();await new Promise(resolve=>setTimeout(resolve,15));assert.equal(sent.length,2,'two unacknowledged inputs is the bound');
 sent[1]!.resolve();await until(()=>sent.length===3);assert.deepEqual(sent.map(s=>s.action.text),['a','b','c']);
 sent[2]!.written!();sent[2]!.reject(Error('refused'));sent[0]!.resolve();await until(()=>renderer.pendingInputs===0);
 assert.equal(renderer.pendingInputs,0);assert.equal(frame.dataset.mirrorRefusals,'1');assert.equal(sent.length,3,'a refused input is never replayed');renderer.close();
})

// MP-08/MP-10/MP-11: an old transport that lacks write receipts stays serial.
test('mirror inputs without a write notification wait for replies and stop at disposal',async()=>{
 const frame={style:{},dataset:{},setAttribute(){},addEventListener(){},remove(){}},container={ownerDocument:{createElement:()=>frame},append(){}};const replies:Array<()=>void>=[],texts:string[]=[];
 const renderer=paintedRenderer(container as unknown as HTMLElement,((a:any)=>new Promise<void>(resolve=>{texts.push(a.text);replies.push(resolve)})) as any) as any;renderer.documentId='d';renderer.sequence=1;
 renderer.enqueue({kind:'text',text:'a'});renderer.enqueue({kind:'text',text:'b'});renderer.enqueue({kind:'text',text:'c'});await until(()=>texts.length===1);await new Promise(r=>setTimeout(r,15));assert.deepEqual(texts,['a']);replies[0]!();await until(()=>texts.length===2);renderer.close();replies[1]!();await until(()=>renderer.pendingInputs===0);assert.deepEqual(texts,['a','b']);assert.equal(renderer.pendingInputs,0);
})

// MP-08/MP-10/MP-11: the union's root-scroll hold lasts through its own reply.
test('overlapping input replies do not release another root scroll or hold a new document',async()=>{
 const frame={style:{},dataset:{},setAttribute(){},addEventListener(){},remove(){}},container={ownerDocument:{createElement:()=>frame},append(){}};
 const sent:Array<{written:(()=>void)|undefined;resolve:()=>void}>=[];
 const renderer=paintedRenderer(container as unknown as HTMLElement,((_a:any,_e:any,written?:()=>void)=>new Promise<void>(resolve=>sent.push({written,resolve}))) as any) as any;renderer.documentId='d';renderer.sequence=1;
 renderer.enqueue({kind:'scroll_to',node_id:null,x:0,y:200});await until(()=>sent.length===1);sent[0]!.written!();
 renderer.enqueue({kind:'text',text:'a'});await until(()=>sent.length===2);sent[1]!.written!();sent[1]!.resolve();await until(()=>renderer.pendingInputs===1);
 assert.equal(renderer.rootScrolling(),true,'a text reply cannot release the root scroll');
 renderer.documentId='next';assert.equal(renderer.rootScrolling(),false,'old-document replies do not hold the new document');
 renderer.documentId='d';sent[0]!.resolve();await until(()=>renderer.pendingInputs===0);assert.equal(renderer.rootScrolling(),false);renderer.close();
})

// MP-08/MP-10/MP-11: structural diagnostics reveal no IDs or page content.
test('MP-11: tree refusals distinguish missing delta targets from invalid parents without reflecting records', () => {
  const previous = new Map(tree().map(record => [record.id, record]))
  const delta = (target: string, parent: string): Mirror2Packet => { const { nodes, ...packet } = base(tree()); return { ...packet, reset: false, sequence: 2, base_sequence: 1, ops: [{ op: 'children', id: target, children: ['n7'], nodes: [{ id: 'n7', parent, kind: 'text', text: 'PRIVATE_PAGE_TEXT' }] }] } }
  assert.throws(() => validateMirror2Packet(delta('n6', 'n6'), previous), { message: 'MP-11: unsafe mirror tree (missing operation target)' })
  assert.throws(() => validateMirror2Packet(delta('n2', 'n8'), previous), { message: 'MP-11: unsafe mirror tree (missing or out-of-order parent)' })
  assert.throws(() => validateMirror2Packet(base(tree({ id: 'n7', parent: 'n8', kind: 'text', text: 'PRIVATE_PAGE_TEXT' })), previous), { message: 'MP-11: unsafe mirror tree (missing or out-of-order parent)' })
})

// MP-08/MP-10/MP-11: the renderer depends on the explicit completion bit.
test('MP-11 completion header is boolean; pre-550 kernels (#1016 review 5: 539/540 lack the wire extensions) are refused before subscription', async () => {
  for (const snapshot_complete of [true, false]) validateMirror2Packet(base(tree(element('p')), { snapshot_complete }), new Map())
  validateMirror2Packet(base(tree(element('p'))), new Map())
  for (const snapshot_complete of [null, 0, 'true', {}]) assert.throws(() => validateMirror2Packet(base(tree(element('p')), { snapshot_complete } as unknown as Partial<Mirror2Packet>), new Map()), /unsafe mirror header/)
  for (const protocolVersion of [517, 535, 538, 539, 540, NaN]) {
    let called = false
    await assert.rejects(attachBrowserMirror2({ protocolVersion, request: async () => { called = true } }, {} as HTMLElement, { tab_id: 't', generation: 1, device_scale_factor: 1 }, { failure() {} }), /requires protocol 550/)
    assert.equal(called, false)
  }
})


test('MP-10: a progressing 700 KB response over two seconds does not start another transfer', async () => {
  let nexts = 0, resends = 0
  const receiver = new RelayFrameReceiver(), applied: number[] = []
  const transport = { protocolVersion: 550, receiving: () => receiver.receiving(), request: async (request: any): Promise<any> => {
    const command = request.KernelBrowser.command
    if (command.op === 'mirror_subscribe') return { KernelBrowser: { result: { subscription_id: 's' } } }
    if (command.op === 'mirror_close') return { KernelBrowser: { result: { closed: true } } }
    const turn = ++nexts; if (command.resend) resends++
    if (turn > 2) return new Promise(() => {})
    if (turn === 1) return { KernelBrowser: { result: packet(1, true) } }
    const body = JSON.stringify({ KernelBrowser: { result: packet(2, false) }, padding: 'a'.repeat(700_000) })
    let complete: string | null = null
    for (let offset = 0; offset < body.length; offset += 16_384) {
      complete = receiver.receive(JSON.stringify({ kind: 'transport_chunk', transfer_id: 1, offset, total_bytes: body.length, payload: body.slice(offset, offset + 16_384) }), () => {})
      if (!complete) await new Promise(resolve => setTimeout(resolve, 50))
    }
    return JSON.parse(complete!)
  } }
  const renderer = () => ({ frame: {} as HTMLIFrameElement, ready: async () => {}, close: () => {}, apply: async (p: Mirror2Packet) => { applied.push(p.sequence) } })
  const mirror = await attachBrowserMirror2(transport, {} as HTMLElement, { tab_id: 't', generation: 1, device_scale_factor: 1 }, { failure() {} }, { credits: 1, waitMs: 0, renderer })
  try { await until(() => applied.length === 2, 2800); assert.equal(applied.length, 2); assert.equal(resends, 0) }
  finally { await mirror.close() }
})

test('MP-10: validated chunk progress suppresses recovery only while the transfer advances', async () => {
  const receiver = new RelayFrameReceiver()
  receiver.receive(JSON.stringify({kind:'transport_chunk',transfer_id:1,offset:0,total_bytes:700_000,payload:'x'}), () => {})
  assert.equal(receiver.receiving(), true)
  await new Promise(resolve => setTimeout(resolve, 1100))
  assert.equal(receiver.receiving(), false, 'a stalled response cannot keep recovery paused')
  assert.throws(() => receiver.receive(JSON.stringify({kind:'transport_chunk',transfer_id:1,offset:0,total_bytes:700_000,payload:'x'}), () => {}))
  assert.equal(receiver.receiving(), false, 'invalid bytes cannot refresh progress')
})


test('MP-11 a reset can reuse only immutable sheets actually retained by this viewer', () => {
  const digest = 'd'.repeat(24), known = new Map<string, string>(), retained = new Map([[digest, 'p{color:red}']])
  const reset = base(tree({id:'n3',parent:'n2',kind:'element',tag:'style',css_ref:digest}))
  assert.equal(resolveMirror2Sheets(reset, known, retained).nodes![2]!.css, 'p{color:red}')
  assert.throws(() => resolveMirror2Sheets({...reset,nodes:tree({id:'n3',parent:'n2',kind:'element',tag:'style',css_ref:'e'.repeat(24)})}, known, retained), /sheet reference/)
})

test('MP-08/MP-10/MP-11 native preview keeps credits alive and a handover flush applies after earlier packets',async()=>{
 const applied:number[]=[],commands:any[]=[],previews:string[]=[];let sequence=0,closed=false;
 const renderer=()=>({frame:{} as HTMLIFrameElement,ready:async()=>{},close:()=>{},apply:async(p:Mirror2Packet)=>{applied.push(p.sequence)}});
 const request=async(r:any)=>{const c=r.KernelBrowser.command;commands.push(c);let result:any;
  if(c.op==='mirror_subscribe')result={subscription_id:'s'};
  else if(c.op==='mirror_close'){closed=true;result={closed:true}}
  else if(c.op==='mirror_next'){
   const n=++sequence;if(n===1)result={...packet(1,true),fallback:'navigation_preview',snapshot_complete:false};
   else {if(!c.flush)await new Promise(resolve=>setTimeout(resolve,10));result=packet(n,n===2)}
  }
  return{KernelBrowser:{result}};
 };
 const view=await attachBrowserMirror2({protocolVersion:550,request},{}as HTMLElement,{tab_id:'t',generation:1,device_scale_factor:1},{failure:assert.fail,preparing:(p:Mirror2Packet)=>previews.push(p.document_id)} as any,{renderer,credits:1,waitMs:0});
 try{
  await until(()=>applied.length>0,1000);
  assert.deepEqual(previews,['d']);assert.equal(commands[0].navigation_preview,true);
  const fresh=await (view as any).synchronize();assert.equal(fresh.document_id,'d');assert(applied.includes(fresh.sequence));
  assert(commands.some(c=>c.flush&&c.wait_ms===0));
 }finally{await view.close();assert(closed)}
});

// MP-08/MP-10/MP-11: recovery over an existing video also has a staged reset.
test('MP-08/MP-10/MP-11 a recovery reset holds the renderer before any reset content applies',async()=>{
 const calls:string[]=[];let issued=0;
 const request=async(r:any)=>{const c=r.KernelBrowser.command;
  if(c.op==='mirror_subscribe')return{KernelBrowser:{result:{subscription_id:'s'}}};
  if(c.op==='mirror_close')return{KernelBrowser:{result:{closed:true}}};
  if(++issued>1)await new Promise(resolve=>setTimeout(resolve,10));
  return{KernelBrowser:{result:packet(issued,issued===1)}};
 };
 const renderer=()=>({frame:{}as HTMLIFrameElement,ready:async()=>{},close:()=>{},holdForHandover:()=>calls.push('hold'),apply:async()=>{calls.push('apply')}});
 const mirror=await attachBrowserMirror2({protocolVersion:550,request},{}as HTMLElement,{tab_id:'t',generation:1,device_scale_factor:1},{failure:assert.fail,prepareReset:()=>{calls.push('prepare');return true}}as any,{renderer,credits:1,waitMs:0});
 try{await until(()=>calls.includes('apply'),1000);assert.deepEqual(calls.slice(0,3),['prepare','hold','apply'])}finally{await mirror.close()}
});

test('MP-08/MP-10/MP-11 source font failure metadata is bounded and authenticated as part of the packet',()=>{
 const valid='a'.repeat(64)
 for(const font_failures of [[],[valid]])validateMirror2Packet(base(tree(element('p')),{font_failures} as Partial<Mirror2Packet>),new Map())
 for(const font_failures of [null,'x',[valid,valid],['not-a-digest'],Array(129).fill(valid)])assert.throws(()=>validateMirror2Packet(base(tree(element('p')),{font_failures} as unknown as Partial<Mirror2Packet>),new Map()),/unsafe mirror header/)
})

test('MP-08/MP-10/MP-11 native font requirements are bounded and target a known document only',()=>{
 const nodes:Mirror2Record[]=[{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'p'}]
 const hashes=Array.from({length:129},(_,i)=>i.toString(16).padStart(64,'0'))
 for(const required of [null,[],hashes.slice(0,128)])validateMirror2Packet(base(nodes,{ops:[{op:'fonts',id:'n1',required}]}),new Map())
 for(const required of [undefined,'x',[hashes[0],hashes[0]],hashes,['invalid']])assert.throws(()=>validateMirror2Packet(base(nodes,{ops:[{op:'fonts',id:'n1',required}]}as any),new Map()),/unsafe mirror fonts/)
 for(const id of ['n2','n99'])assert.throws(()=>validateMirror2Packet(base(nodes,{ops:[{op:'fonts',id,required:[]}]}),new Map()),/unsafe mirror fonts/)
 assert.throws(()=>validateMirror2Packet(base(nodes,{ops:[{op:'fonts',id:'n1',required:[],family:'unexpected'}]}as any),new Map()),/unsafe mirror fonts/)
 validateMirror2Packet(base(nodes,{ops:[{op:'fonts',id:'n1',required:[],absent:['-apple-system','segoe ui']}]}),new Map())
 for(const absent of ['x',[1],['a"b'],['a\\b'],['a{b'],[''],['x'.repeat(129)],['a','a'],Array.from({length:65},(_,i)=>`f${i}`)])assert.throws(()=>validateMirror2Packet(base(nodes,{ops:[{op:'fonts',id:'n1',required:[],absent}]}as any),new Map()),/unsafe mirror fonts/)
})
