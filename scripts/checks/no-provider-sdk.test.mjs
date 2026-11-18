import assert from 'node:assert/strict'
import { test } from 'node:test'
import { makeRepo } from '../lib/check-fixture.mjs'
import check from './no-provider-sdk.mjs'

const manifest = deps => `${JSON.stringify({ name: 'x', dependencies: deps })}\n`
const planted = (r, label) => {
  const result = r.run(check.name)
  assert.equal(result.status, 1, label)
  assert.ok(result.text.includes(`Root fix: ${check.rootFix}`), `${label}: names the root fix`)
  return result.text
}

test('the official provider CLIs are allowed', () => {
  const r = makeRepo({ 'apps/x/package.json': manifest({ '@anthropic-ai/claude-code': '1', '@openai/codex': '1' }) })
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('a provider SDK dependency in package.json is red', () => {
  const r = makeRepo({ 'apps/x/package.json': manifest({ zod: '1' }) })
  r.write('apps/x/package.json', manifest({ zod: '1', '@anthropic-ai/sdk': '0.1' }))
  assert.match(planted(r, 'package.json'), /apps\/x\/package\.json:\d+: dependencies lists the provider SDK @anthropic-ai\/sdk/)
  r.dispose()
})

test('a provider SDK import in source is red', () => {
  const r = makeRepo({ 'apps/x/a.ts': 'export const a = 1\n' })
  r.write('apps/x/a.ts', 'import OpenAI from "openai"\nexport const a = 1\n')
  assert.match(planted(r, 'import'), /apps\/x\/a\.ts:1: imports the provider SDK openai/)
  r.dispose()
})

test('a provider SDK crate in a Cargo manifest is red', () => {
  const r = makeRepo({ 'apps/k/Cargo.toml': '[dependencies]\nserde = "1"\n' })
  r.write('apps/k/Cargo.toml', '[dependencies]\nserde = "1"\nasync-openai = "0.2"\n')
  assert.match(planted(r, 'cargo'), /apps\/k\/Cargo\.toml:3: Cargo manifest lists the provider SDK crate async-openai/)
  r.dispose()
})
