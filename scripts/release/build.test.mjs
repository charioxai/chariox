import assert from 'node:assert/strict'
import test from 'node:test'
import { build, buildCommand, boundedDockerfile } from './build.mjs'

const input = { component: 'cloud-web', source: '/source', output: '/evidence/build', version: '0.1.0', revision: 'a'.repeat(40) }
test('MP-07 / MP-11: default build is a side-effect-free plan', async () => {
  const plan = await build(input)
  assert.equal(plan.mode, 'plan')
  assert.equal(plan.command, 'docker')
  assert.ok(!plan.args.includes('--push'))
  assert.ok(!plan.args.some(arg => arg.startsWith('type=registry')))
})
test('MP-07 / MP-11: publishing uses separate registry cache and immutable revision tags', () => {
  const plan = buildCommand({ ...input, mode: 'push' })
  assert.ok(plan.args.includes('--push'))
  assert.ok(!plan.args.includes('--load'))
  assert.ok(plan.args.includes('type=registry,ref=ghcr.io/charioxai/chariox-cloud-web:buildcache,mode=max'))
  assert.ok(plan.tag.endsWith(`:0.1.0-${input.revision}`))
  for (const component of ['cloud-services', 'oss', 'relay']) {
    const other = buildCommand({ ...input, component, mode: 'push' })
    assert.notEqual(other.tag, plan.tag)
  }
})
test('MP-07 / MP-11: bad revisions/modes cannot become shell or Docker inputs', () => {
  for (const change of [{ revision: 'main' }, { component: '../../other' }, { version: '0.1.0; id' }, { version: '0.1.0-01' }, { mode: 'publish' }]) {
    assert.throws(() => buildCommand({ ...input, ...change }))
  }
})
test('MP-08 / MP-09 / MP-10 / MP-11: a build memory cap does not alter the deployed runtime environment', () => {
  const source = 'FROM node:fixture AS build\nRUN node build.mjs\nFROM node:fixture AS runtime\nCMD ["node", "server.mjs"]\n'
  const result = boundedDockerfile(source)
  assert.ok(result.split('AS runtime\n')[0].includes('ENV NODE_OPTIONS=--max-old-space-size=4096'))
  assert.equal(result.split('AS runtime\n')[1], source.split('AS runtime\n')[1])
})
