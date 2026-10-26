import test from 'node:test'
import assert from 'node:assert/strict'
import http from 'node:http'
import { SecretHandoffHarness } from '../experiments/secret-handoff/src/secret-harness.mjs'

test('MP-11 F15 explicitly excluded reserved environment names stay excluded', () => {
  const harness = new SecretHandoffHarness({ credentials: { secret: { source: 'env', env: 'PATH' }, home: { source: 'env', env: 'HOME' }, temp: { source: 'env', env: 'TMPDIR' } }, kernelEnv: { PATH: 'synthetic', HOME: 'synthetic', TMPDIR: 'synthetic' } })
  assert.deepEqual(harness.scrubProviderEnv(), {})
})
test('MP-11 F15 prototype HTTP echoes exclude injected and encoded values', async () => {
  const secret = 'mp11 synthetic/+secret'
  const server = http.createServer((request,response) => {
    const body = JSON.stringify({ raw: secret, encoded: encodeURIComponent(secret), base64: Buffer.from(secret).toString('base64'), auth: request.headers.authorization })
    response.writeHead(403, {'content-type':'application/json'}); response.end(body)
  })
  await new Promise(resolve => server.listen(0,'127.0.0.1',resolve))
  try {
    const harness = new SecretHandoffHarness({ kernelEnv: {}, credentials: { secret: { source: 'vault', value: secret, allowed_uses:['http'], injection: {kind:'basic', username:'fixture'} } } })
    const result = await harness.httpRequestWithCredential({ credential_id:'secret', url:`http://127.0.0.1:${server.address().port}/` })
    const text = JSON.stringify(result)
    for (const value of [secret,encodeURIComponent(secret),Buffer.from(secret).toString('base64'),Buffer.from(`fixture:${secret}`).toString('base64')]) assert.equal(text.includes(value),false)
    assert.equal(result.status,403)
  } finally { await new Promise(resolve => server.close(resolve)) }
})
