// MP-08/MP-10: actual capture helper; admission proof must not alter frozen judge evidence.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import vm from 'node:vm'
import { readFile } from 'node:fs/promises'
test('MP-08/MP-10 admission screenshot stays separate from solver/judge screenshot history', async () => {
  const source = await readFile(new URL('./webvoyager-task.mjs', import.meta.url), 'utf8')
  const start = source.indexOf('  async function capture('), end = source.indexOf('  const started = Date.now()', start)
  assert(start >= 0 && end > start)
  const row = { screenshots: [], screenshotMetadata: [], captureErrors: [] }, calls = []
  const context = vm.createContext({ row, container: 'owned-fixture', directory: '/owned-evidence', seam: 'capture_preflight',
    checkpoint: async () => {}, docker: async args => { calls.push(args); return { stdout: JSON.stringify({ width: 1024, height: 768 }) } } })
  const admissionStart = source.indexOf("    seam = 'capture_preflight'"), admissionEnd = source.indexOf("    seam = 'agent_spawn'", admissionStart)
  assert(admissionStart >= 0 && admissionEnd > admissionStart)
  await vm.runInContext(source.slice(start, end) + '\n(async () => {\n' + source.slice(admissionStart, admissionEnd) + '\n})()', context)
  assert.equal(row.screenshots.length, 0, 'blank admission image changed official last-15 judge evidence')
  assert.equal(row.screenshotMetadata.length, 0)
  assert.equal(row.admissionScreenshot.name, 'admission.png')
  assert(calls.some(args => args[0] === 'cp' && args[1].endsWith('/tmp/benchwv-admission.png')))
  await vm.runInContext('capture()', context)
  assert.deepEqual(row.screenshots, ['screenshot1.png'])
  assert.equal(row.screenshotMetadata.length, 1)
})
