import assert from 'node:assert/strict'
import { test } from 'node:test'
import { makeRepo } from '../lib/check-fixture.mjs'
import check from './hosted-wss.mjs'

test('wss and loopback ws pass', () => {
  const r = makeRepo({ 'deploy/hosted/compose.yaml': 'env:\n  A: wss://relay.example.test/kernel\n  B: ws://127.0.0.1:43150\n  C: ws://localhost:1\n  D: ws://[::1]:2\n' })
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('a plain ws:// endpoint in deploy config is red and names the root fix', () => {
  const r = makeRepo({ 'deploy/hosted/compose.yaml': 'env:\n  A: wss://relay.example.test/kernel\n' })
  r.write('deploy/hosted/compose.yaml', 'env:\n  A: wss://relay.example.test/kernel\n  B: ws://relay.example.test:43150\n')
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, /deploy\/hosted\/compose\.yaml:3: plain ws:\/\/relay\.example\.test:43150 in deploy config\. Root fix: put the endpoint behind Caddy/)
  r.dispose()
})

test('a ws:// URL in prose or code outside deploy config is not this check', () => {
  const r = makeRepo({ 'docs/a.md': 'x\n' })
  r.write('docs/a.md', 'use ws://relay:1\n')
  r.write('apps/cli/src/a.ts', 'const url = "ws://host:1"\n')
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})
