import assert from 'node:assert/strict'
import { createHash, generateKeyPairSync, randomBytes, sign } from 'node:crypto'
import { createServer } from 'node:http'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { startVerifierServer } from './web-bot-auth-verifier.mjs'

const b64url = value => Buffer.from(value).toString('base64url')
const thumbprint = x => createHash('sha256').update(JSON.stringify({ crv: 'Ed25519', kty: 'OKP', x })).digest('base64url')
const keyPair = generateKeyPairSync('ed25519')
const jwk = keyPair.publicKey.export({ format: 'jwk' })
const key = { kty: 'OKP', crv: 'Ed25519', kid: thumbprint(jwk.x), x: jwk.x, use: 'sig', nbf: Math.floor(Date.now() / 1000) - 60, exp: Math.floor(Date.now() / 1000) + 3600 }
let publishedKeys = [key]

const directoryServer = createServer((request, response) => {
  if (request.url !== '/.well-known/http-message-signatures-directory') { response.writeHead(404).end(); return }
  response.writeHead(200, { 'content-type': 'application/http-message-signatures-directory+json' })
  response.end(JSON.stringify({ keys: publishedKeys }))
})

async function listen(server) {
  await new Promise((resolve, reject) => server.listen(0, '127.0.0.1', error => error ? reject(error) : resolve()))
  return `http://127.0.0.1:${server.address().port}`
}

function signedHeaders(url, directoryUrl, { method = 'GET', body, alterBase = base => base, created: suppliedCreated, expires: suppliedExpires } = {}) {
  const created = suppliedCreated ?? Math.floor(Date.now() / 1000)
  const expires = suppliedExpires ?? created + 60
  const nonce = b64url(randomBytes(24))
  const components = ['"@method"', '"@target-uri"', '"@authority"', '"signature-agent";key="sig1"']
  const rows = [
    `"@method": ${method}`,
    `"@target-uri": ${url}`,
    `"@authority": ${new URL(url).host}`,
    `"signature-agent";key="sig1": "${directoryUrl}"`,
  ]
  const headers = { 'signature-agent': `sig1="${directoryUrl}"` }
  if (body !== undefined) {
    const digest = `sha-256=:${createHash('sha256').update(body).digest('base64')}:`
    headers['content-digest'] = digest
    components.push('"content-digest"')
    rows.push(`"content-digest": ${digest}`)
  }
  const params = `(${components.join(' ')});created=${created};expires=${expires};keyid="${key.kid}";alg="ed25519";nonce="${nonce}";tag="web-bot-auth"`
  const signatureBase = `${rows.join('\n')}\n"@signature-params": ${params}`
  headers['signature-input'] = `sig1=${params}`
  headers.signature = `sig1=:${sign(null, Buffer.from(alterBase(signatureBase)), keyPair.privateKey).toString('base64')}:`
  return headers
}

test('MP-08/09/10/11 local Web Bot Auth verifier accepts signed requests and rejects invalid proof', async t => {
  const directoryOrigin = await listen(directoryServer)
  const directoryUrl = `${directoryOrigin}/.well-known/http-message-signatures-directory`
  const evidence = await mkdtemp(path.join(tmpdir(), 'wba-verifier-'))
  const eventLog = path.join(evidence, 'verification.ndjson')
  const verifier = await startVerifierServer({ host: '127.0.0.1', port: 0, allowedDirectoryUrls: [directoryUrl], eventLog, expectedCredential: 'synthetic-secret' })
  const origin = verifier.url
  try {
    const validUrl = `${origin}/api?source=agent`
    const validHeaders = signedHeaders(validUrl, directoryUrl)
    const accepted = await fetch(validUrl, { headers: validHeaders })
    assert.equal(accepted.status, 200)
    assert.deepEqual(await accepted.json(), { accepted: true })
    const replayed = await fetch(validUrl, { headers: validHeaders })
    assert.equal(replayed.status, 403)

    const unsigned = await fetch(`${origin}/api`)
    assert.equal(unsigned.status, 403)

    const original = `${origin}/api?value=one`
    const tampered = await fetch(`${origin}/api?value=two`, { headers: signedHeaders(original, directoryUrl) })
    assert.equal(tampered.status, 403)

    const body = Buffer.from('credential=synthetic-secret')
    const bodyUrl = `${origin}/api`
    const bodyTampered = await fetch(bodyUrl, { method: 'POST', body: 'credential=changed', headers: signedHeaders(bodyUrl, directoryUrl, { method: 'POST', body }) })
    assert.equal(bodyTampered.status, 403)
    const acceptedBody = await fetch(bodyUrl, { method: 'POST', body, headers: signedHeaders(bodyUrl, directoryUrl, { method: 'POST', body }) })
    assert.equal(acceptedBody.status, 200)
    const badSignature = await fetch(validUrl, { headers: signedHeaders(validUrl, directoryUrl, { alterBase: base => `${base}x` }) })
    assert.equal(badSignature.status, 403)
    const now = Math.floor(Date.now() / 1000)
    const expiredSignature = await fetch(validUrl, { headers: signedHeaders(validUrl, directoryUrl, { created: now - 120, expires: now - 60 }) })
    assert.equal(expiredSignature.status, 403)

    publishedKeys = [{ ...key, exp: now - 1 }]
    const expiredKey = await fetch(validUrl, { headers: signedHeaders(validUrl, directoryUrl) })
    assert.equal(expiredKey.status, 403)
    publishedKeys = [{ ...key, nbf: now + 3600, exp: now + 7200 }]
    const futureKey = await fetch(validUrl, { headers: signedHeaders(validUrl, directoryUrl) })
    assert.equal(futureKey.status, 403)
    publishedKeys = [key]

    const untrustedDirectory = 'http://127.0.0.1:1/.well-known/http-message-signatures-directory'
    const rejectedDirectory = await fetch(validUrl, { headers: signedHeaders(validUrl, untrustedDirectory) })
    assert.equal(rejectedDirectory.status, 403)

    publishedKeys = []
    const removedKey = await fetch(validUrl, { headers: signedHeaders(validUrl, directoryUrl) })
    assert.equal(removedKey.status, 403)
    publishedKeys = [key]

    for (const [credential, status] of [['synthetic-secret', 200], ['wrong', 403]]) {
      const url = `${origin}/api?kind=credential&probe=${credential}`
      assert.equal((await fetch(url, { headers: signedHeaders(url, directoryUrl) })).status, status)
    }

    const page = await fetch(`${origin}/`, { headers: signedHeaders(`${origin}/`, directoryUrl) })
    assert.equal(page.status, 200)
    const html = await page.text()
    assert.match(html, /<img[^>]+src="\/assets\/pixel\.png"/)
    assert.match(html, /<script[^>]+src="\/assets\/agent\.js"/)

    for (const route of ['/assets/pixel.png', '/assets/agent.js']) {
      const url = `${origin}${route}`
      const response = await fetch(url, { headers: signedHeaders(url, directoryUrl) })
      assert.equal(response.status, 200)
    }
    const apiEvents = await verifier.events()
    assert.equal(apiEvents.verified, 6)
    assert.equal(apiEvents.denied, 11)
    assert.deepEqual(apiEvents.routes, {
      '/': { verified: 1, denied: 0 },
      '/api': { verified: 3, denied: 11 },
      '/assets/*': { verified: 2, denied: 0 },
    })
    const logs = await readFile(eventLog, 'utf8')
    assert.doesNotMatch(logs, /synthetic-secret|credential=changed|Signature-Agent|signature-input/i)
    assert.equal((await verifier.close()).verified, 6)
  } finally {
    await verifier.close().catch(() => undefined)
    await new Promise(resolve => directoryServer.close(resolve))
    await rm(evidence, { recursive: true, force: true })
  }
})
