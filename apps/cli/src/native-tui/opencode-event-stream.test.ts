// MP-08 / MP-10 / MP-11: native source reviews must not corrupt JSON at hidden tags.
import assert from 'node:assert/strict'
import test from 'node:test'
import { createSseHiddenInstructionRedactor } from './opencode-event-stream.js'
import { hiddenInstructionsStart, hiddenInstructionsEnd } from './hidden-instructions.js'

async function through(bytes: Buffer[]) {
  const stream = createSseHiddenInstructionRedactor()
  const chunks: Buffer[] = []
  stream.on('data', chunk => chunks.push(Buffer.from(chunk)))
  for (const bytesPart of bytes) stream.write(bytesPart)
  stream.end()
  await new Promise<void>((resolve, reject) => { stream.on('end', resolve); stream.on('error', reject) })
  return Buffer.concat(chunks).toString('utf8')
}

test('MP-08 / MP-10: workflow tag in a real source-shaped tool body preserves SSE JSON', async () => {
  const event = { type: 'message.part.updated', properties: { part: { type: 'tool', state: { output: 'public code\n<workflow-runtime-instructions>\nprivate injected context\n</workflow-runtime-instructions>' } } } }
  const output = await through([Buffer.from('data: '+JSON.stringify(event)+'\n\n')])
  const decoded = JSON.parse(output.slice(6).trim())
  assert.equal(decoded.type, event.type)
  assert.equal(decoded.properties.part.state.output, 'public code')
  assert.equal(output.includes('private injected context'), false)
})

test('MP-08 / MP-10 / MP-11: fragmented UTF-8 and explicit hidden blocks remain protected', async () => {
  const event = { type: 'message.part.updated', properties: { part: { type: 'text', text: `公開 🌍 ${hiddenInstructionsStart}private${hiddenInstructionsEnd} visible` } } }
  const input = Buffer.from('id: stable\r\ndata: '+JSON.stringify(event)+'\r\n\r\n')
  const output = await through([...input].map(byte => Buffer.from([byte])))
  const data = output.split(/\r?\n/).find(line => line.startsWith('data: '))!
  assert.equal(JSON.parse(data.slice(6)).properties.part.text, '公開 🌍  visible')
  assert.equal(output.includes('private'), false)
})
