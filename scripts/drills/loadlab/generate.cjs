#!/usr/bin/env node
const { mkdirSync, writeFileSync } = require('node:fs')
const { join } = require('node:path')
const { parseTrace } = require('../../../apps/kernel/src/provider/registry/dev_stub_adapter/trace_format.cjs')

const prompt = { t: 0, kind: 'prompt', chars: 420 }
const end = { t: 0, kind: 'end', outcome: 'completed' }
const wait = { t: 0, kind: 'wait', ms: 300_000 }
const text = (chars, stream_ms = 0, markdown = 'plain') => ({ t: 0, kind: 'text', chars, stream_ms, markdown })
const terminal = { t: 0, kind: 'terminal', lines: 20_000, line_bytes: 120, rate_lps: 4_000 }

function randomFor(seed) {
  // Seed the 32-bit generator with both halves of the safe integer seed.
  let state = (seed ^ Math.trunc(seed / 2 ** 32)) >>> 0
  return max => {
    state = (Math.imul(1664525, state) + 1013904223) >>> 0
    return Math.floor(state * max / 2 ** 32)
  }
}

function history(random, turns) {
  return Array.from({ length: turns }, (_, index) => [
    prompt,
    { t: 0, kind: 'reasoning', chars: 200 + random(1700), stream_ms: 0 },
    text(300 + random(3500), 0, ['plain', 'list', 'table', 'code'][random(4)]),
    ...(index % 8 === 0 ? [{
      t: 0, kind: 'tool', name: ['Bash', 'Read', 'Grep'][random(3)],
      input_bytes: 120 + random(400), output_bytes: 500 + random(39_500),
      ms: 0, outcome: index % 16 === 0 ? 'error' : 'ok',
    }] : []),
    end,
  ]).flat()
}

function childName(index) {
  return `child-${String(index + 1).padStart(2, '0')}`
}

function spawn(index) {
  const agent = childName(index)
  return { t: 0, kind: 'spawn', agent, trace: `${agent}.jsonl` }
}

function s1(random, add) {
  // Four direct children each spawn two grandchildren. All 40 count toward S1.
  add('orch', 'coordinator; 32 direct children', [prompt, ...Array.from({ length: 32 }, (_, i) => spawn(i)), wait, end])
  for (let i = 0; i < 40; i++) {
    const name = childName(i)
    if (i < 12) {
      const grandchildren = i < 4 ? [spawn(32 + 2 * i), spawn(33 + 2 * i)] : []
      add(name, 'running', [prompt, ...grandchildren, text(12_000, 120_000), wait, end])
    } else if (i < 18) {
      add(name, 'waiting', [prompt, { t: 0, kind: 'permission', tool: 'Bash' }, end])
    } else {
      // Three ordinary history entries per turn, with a tool row every eighth
      // turn. Leave enough headroom for both tool call and result rows.
      add(name, 'done', history(random, 67 + random(540)))
    }
  }
}

function s2(random, add) {
  for (let i = 0; i < 50; i++) {
    const steps = i < 10 ? [text(12_000 + random(2000), 60_000)] : []
    if (i >= 10 && i < 16) steps.push(terminal)
    if (i >= 16 && i < 19) steps.push({ t: 0, kind: 'permission', tool: 'Bash' })
    add(`room-agent-${i + 1}`, 'one of 50 Room agents', [prompt, ...steps, end])
  }
}

function s3(random, add) {
  for (let i = 0; i < 40; i++) {
    const burst = i % 5 === 0 ? [{ t: 0, kind: 'notify', source: 'lab-source', count: i < 20 ? 38 : 37, interval_ms: 250 }] : []
    add(`room-${i + 1}`, `project-${1 + Math.floor(i / 5)}`, [
      prompt, text(700 + random(1500), 800, 'list'), ...burst, end,
    ])
  }
}

function generate(preset, seed) {
  if (!Number.isSafeInteger(seed) || seed < 0) throw new Error('seed must be a non-negative safe integer')
  const scenarios = { s1, s2, s3 }
  if (!Object.hasOwn(scenarios, preset)) throw new Error('preset must be s1, s2 or s3')
  const files = new Map()
  const add = (agent, notes, steps) => {
    const filename = `${agent}.jsonl`
    const contents = [{ v: 1, source: 'generator', seed, agent, notes }, ...steps].map(row => JSON.stringify(row)).join('\n') + '\n'
    parseTrace(contents, filename)
    files.set(filename, contents)
  }
  scenarios[preset](randomFor(seed), add)
  return files
}

if (require.main === module) {
  try {
    const [preset, seed, output, ...extra] = process.argv.slice(2)
    if (!output || !/^\d+$/.test(seed ?? '') || extra.length) {
      throw new Error('usage: node scripts/drills/loadlab/generate.cjs <s1|s2|s3> <seed> <output-directory>')
    }
    const files = generate(preset, Number(seed))
    mkdirSync(output, { recursive: true })
    for (const [filename, contents] of files) writeFileSync(join(output, filename), contents, { flag: 'wx' })
  } catch (error) {
    console.error(error.message)
    process.exitCode = 1
  }
}

module.exports = { generate }
