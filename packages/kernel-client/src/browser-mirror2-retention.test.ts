// MIRROR-P2: sheets and fonts a view received stay available to the next subscription of the same view, within bounds.
import test from 'node:test'
import assert from 'node:assert/strict'
import { Mirror2Retention } from './browser-mirror2-retention.js'
import { attachBrowserMirror2 } from './browser-mirror2.js'
const digest = (n: number) => n.toString(16).padStart(24, '0')
const id = (n: number) => digest(n).padEnd(64, '0')
test('MIRROR-P2 a new subscription frees the oldest half of a full view cache and keeps the newest', () => {
  const retention = new Mirror2Retention()
  for (let n = 0; n < 128; n++) retention.sheets.remember({ [digest(n)]: 'p{color:red}' })
  for (let n = 0; n < 64; n++) retention.fonts.remember(id(n), new Blob([new Uint8Array(10)], { type: 'font/woff2' }))
  retention.begin()
  assert.equal(retention.sheets.digests().length, 64)
  assert.equal(retention.fonts.digests().length, 32)
  assert.deepEqual(retention.sheets.digests()[0], digest(64), 'the oldest go first')
  const kept = retention.sheets.digests(); retention.begin()
  assert.deepEqual(retention.sheets.digests(), kept, 'a cache already within half its bounds loses nothing')
  retention.sheets.remember({ [digest(500)]: 'p{color:blue}' })
  assert(retention.sheets.digests().includes(digest(500)), 'the freed room takes what the next page teaches')
})
test('MIRROR-P2 the bytes bound the freed room too', () => {
  const retention = new Mirror2Retention(), text = 'é'.repeat(512 * 1024)
  for (let n = 0; n < 8; n++) retention.sheets.remember({ [digest(n)]: text })
  retention.begin()
  assert.equal(retention.sheets.digests().length, 4)
  const fonts = new Mirror2Retention()
  for (let n = 0; n < 4; n++) fonts.fonts.remember(id(n), new Blob([new Uint8Array(4 * 1024 * 1024)], { type: 'font/woff2' }))
  fonts.begin()
  assert.equal(fonts.fonts.digests().length, 2)
})

test('MIRROR-P2 a subscription of a view lists what the view kept from its earlier pages on its reset credit', async () => {
  const retention = new Mirror2Retention()
  for (let n = 0; n < 4; n++) { retention.fonts.remember(id(n), new Blob([new Uint8Array(10)], { type: 'font/woff2' })); retention.sheets.remember({ [digest(n)]: 'p{color:red}' }) }
  const credits: Array<{ command: any; answer: () => void }> = []
  const packet = { wire: 2, subscription_id: 's', tab_id: 't', generation: 1, document_id: 'd', sequence: 1, base_sequence: null, reset: true, snapshot_complete: true, css_width: 1280, css_height: 800, device_scale_factor: 1, scroll: [0, 0], focused: null, selection: null, resources: [], tiles: [] }
  let received: Mirror2Retention | undefined
  const attached = attachBrowserMirror2({ protocolVersion: 629, request: async (request: any) => {
    const command = request.KernelBrowser.command
    if (command.op === 'mirror_subscribe') return { KernelBrowser: { result: { subscription_id: 's' } } }
    if (command.op === 'mirror_next') return new Promise(resolve => credits.push({ command, answer: () => resolve({ KernelBrowser: { result: packet } }) }))
    return { KernelBrowser: { result: {} } }
  } }, {} as HTMLElement, { tab_id: 't', generation: 1, device_scale_factor: 1 }, { failure() {} }, { waitMs: 2000, retention, renderer: ((_container: unknown, _send: unknown, retained?: Mirror2Retention) => { received = retained; return { ready: async () => {}, apply: async () => {}, close: () => {}, frame: {} as HTMLIFrameElement, knownSheets: () => retained!.sheets.digests(), knownFonts: () => retained!.fonts.digests() } }) as never })
  for (let n = 0; n < 40 && !credits.length; n++) await new Promise(resolve => setTimeout(resolve, 20))
  assert.equal(received, retention, 'the renderer works on the view\'s caches')
  assert.deepEqual(credits[0]!.command.known_fonts, retention.fonts.digests())
  assert.deepEqual(credits[0]!.command.known_sheets, retention.sheets.digests())
  assert.equal(credits[0]!.command.known_fonts.length, 4)
  credits[0]!.answer()
  const mirror = await attached
  await mirror.close()
})
