const assert = require('node:assert/strict')
const { readFileSync } = require('node:fs')
const { join } = require('node:path')
const test = require('node:test')
const { generate } = require('./generate.cjs')
const { parseTrace } = require('../../../apps/kernel/src/provider/registry/dev_stub_adapter/trace_format.cjs')

test('T3: deterministic S1 fixture inventory and history sizes', () => {
  const files = generate('s1', 42)
  assert.deepEqual(files, generate('s1', 42))
  assert.notDeepEqual(files, generate('s1', 43))

  const census = { running: 0, waiting: 0, done: 0 }
  const visited = new Set()
  let deepest = 0
  function visit(filename, depth) {
    assert.ok(!visited.has(filename), filename)
    visited.add(filename)
    deepest = Math.max(deepest, depth)
    const { turns } = parseTrace(files.get(filename), filename)
    const steps = turns.flat()
    const kinds = steps.map(step => step.kind)
    for (const step of steps.filter(step => step.kind === 'spawn')) {
      assert.equal(step.trace, `${step.agent}.jsonl`)
      visit(step.trace, depth + 1)
    }
    if (depth === 0) return
    if (kinds.includes('wait')) census.running++
    else if (kinds.includes('permission')) census.waiting++
    else {
      census.done++
      const rows = steps.reduce((total, step) => total + (step.kind === 'end' ? 0 : step.kind === 'tool' ? 2 : 1), 0)
      assert.ok(rows >= 200 && rows <= 2_000, `${filename} has ${rows} entries`)
    }
  }
  visit('orch.jsonl', 0)
  assert.deepEqual(census, { running: 12, waiting: 6, done: 22 })
  assert.equal(deepest, 2)
  assert.equal(visited.size, files.size)
})

test('S2 and S3 preserve their workload sizes', () => {
  const plans = preset => [...generate(preset, 42)].map(([filename, contents]) => parseTrace(contents, filename))
  const steps = preset => plans(preset).flatMap(plan => plan.turns.flat())
  const s2 = steps('s2')
  assert.equal(plans('s2').length, 50)
  assert.equal(s2.filter(step => step.kind === 'text' && step.stream_ms).length, 10)
  assert.equal(s2.filter(step => step.kind === 'terminal' && step.rate_lps === 4000).length, 6)
  assert.equal(s2.filter(step => step.kind === 'permission').length, 3)
  assert.equal(plans('s3').length, 40)
  assert.equal(new Set(plans('s3').map(plan => plan.header.notes)).size, 8)
  assert.equal(steps('s3').filter(step => step.kind === 'notify').reduce((sum, step) => sum + step.count, 0), 300)
})

test('T4: malformed input names its file and line', () => {
  const fixture = readFileSync(join(__dirname, 'fixtures/orch.jsonl'), 'utf8')
  assert.equal(parseTrace(fixture).turns.length, 1)
  for (const [contents, error] of [
    [fixture.replace('"generator"', '"live"'), /fixture.jsonl: line 1: invalid or missing source/],
    [fixture.replace('"chars":80', '"chars":'), /fixture.jsonl: line 3: invalid JSON/],
    [fixture.replace('"t":40', '"t":1'), /fixture.jsonl: line 5: step time moved backwards/],
    [fixture.replace('"outcome":"completed"', '"outcome":"ok"'), /fixture.jsonl: line 6: invalid or missing outcome/],
    [fixture.split('\n').slice(0, 5).join('\n'), /fixture.jsonl: line 6: expected a turn end/],
  ]) {
    assert.throws(() => parseTrace(contents, 'fixture.jsonl'), error)
  }
})
