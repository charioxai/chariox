// chariox-loadlab-trace v1. Each prompt opens a turn and resets its clock.
const isInteger = value => Number.isSafeInteger(value) && value >= 0
const isPositive = value => isInteger(value) && value > 0
const oneOf = (...values) => value => values.includes(value)
const agentName = value => typeof value === 'string' && /^[a-zA-Z0-9_-]{1,64}$/.test(value)
const traceName = value => typeof value === 'string' && /^[a-zA-Z0-9_-]{1,64}\.jsonl$/.test(value)
const name = value => typeof value === 'string' && /^[a-zA-Z0-9_.:-]{1,128}$/.test(value)
const bounded = max => value => isPositive(value) && value <= max

const bytes = bounded(4 * 1024 * 1024)
const steps = {
  prompt: { chars: bytes },
  reasoning: { chars: bytes, stream_ms: isInteger },
  text: { chars: bytes, stream_ms: isInteger, markdown: oneOf('list', 'code', 'table', 'plain') },
  tool: { name, input_bytes: bytes, output_bytes: bytes, ms: isInteger, outcome: oneOf('ok', 'error') },
  terminal: { lines: bounded(100_000), line_bytes: bounded(4096), rate_lps: bounded(100_000) },
  spawn: { agent: agentName, trace: traceName },
  notify: { source: name, count: bounded(10_000), interval_ms: isInteger },
  permission: { tool: name },
  wait: { ms: isPositive },
  end: { outcome: oneOf('completed', 'cancelled', 'error') },
}

function fields(record, checks, fail) {
  for (const [key, check] of Object.entries(checks)) {
    if (!check(record[key])) fail(`invalid or missing ${key}`)
  }
  for (const key of Object.keys(record)) {
    if (!Object.hasOwn(checks, key)) fail(`unexpected field ${key}`)
  }
}

function parseTrace(input, filename = 'trace') {
  const lines = input.split(/\r?\n/)
  if (lines.at(-1) === '') lines.pop()
  const turns = []
  let header
  let turn
  let lastTime
  for (const [index, line] of lines.entries()) {
    const fail = reason => { throw new Error(`${filename}: line ${index + 1}: ${reason}`) }
    let record
    try {
      record = JSON.parse(line)
    } catch {
      fail('invalid JSON')
    }
    if (!record || typeof record !== 'object' || Array.isArray(record)) fail('expected a JSON object')
    if (index === 0) {
      fields(record, {
        v: oneOf(1), source: oneOf('generator'), seed: isInteger, agent: agentName,
        notes: value => typeof value === 'string' && value.length <= 1024,
      }, fail)
      header = record
      continue
    }
    const { kind, t } = record
    if (!Object.hasOwn(steps, kind)) fail('unknown step kind')
    fields(record, { kind: oneOf(kind), t: isInteger, ...steps[kind] }, fail)
    if (kind === 'prompt') {
      if (turn || t !== 0) fail('prompt must open a new turn at t=0')
      turn = []
      lastTime = 0
    } else if (!turn) {
      fail('expected a prompt to open the turn')
    }
    if (t < lastTime) fail('step time moved backwards')
    lastTime = t
    turn.push(record)
    if (kind === 'end') {
      turns.push(turn)
      turn = undefined
    }
  }
  if (!header) throw new Error(`${filename}: line 1: missing header`)
  if (turn || turns.length === 0) throw new Error(`${filename}: line ${lines.length + 1}: expected a turn end`)
  return { header, turns }
}

module.exports = { parseTrace }
