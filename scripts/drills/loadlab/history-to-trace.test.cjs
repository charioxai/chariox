const assert = require('node:assert/strict')
const test = require('node:test')
const { convertHistory, markdownOf, rowsFromOutline, toolName } = require('./history-to-trace.cjs')
const { parseTrace } = require('../../../apps/kernel/src/provider/registry/dev_stub_adapter/trace_format.cjs')

// A lab session where a Vault-filled value (a fixture secret) reached the prompt text, a tool call, the model's text and a terminal echo.
const secret = 'lab-vault-fixture-9f3c2a71b6'
const row = (kind, text, timestamp_ms, extra = {}) => ({
  session_id: 'session-aa11', agent_id: 'agent-id-orchestrator-7731', provider_run_id: 'run-5521', source_attachment_id: 'attachment-77',
  prompt_author: 'miguel@example.invalid', attachments: [{ name: `${secret}.pem` }], kind, text, timestamp_ms, ...extra,
})
const session = [
  row('user_prompt', `Use the deploy key ${secret} to log in, then list the releases`, 1_000),
  row('provider_reasoning', `I will paste ${secret} into the login form`, 1_400),
  row('provider_tool', `${secret} --login`, 2_000),
  row('provider_tool', `Bash {"command":"export DEPLOY_KEY=${secret}"} output: exported`, 2_600),
  row('provider_output', `- first release\n- second release\nThe key ${secret} worked`, 3_000),
  row('provider_status', 'status text', 3_100),
  row('user_prompt', 'Now show the table', 10_000),
  row('provider_output', '| name | size |\n|---|---|\n| a | 1 |', 10_500),
  row('provider_error', `denied for ${secret}`, 11_000),
  row('provider_output', 'the other agent speaking', 1_500, { agent_id: 'agent-id-child-9902' }),
  row('user_prompt', 'child prompt', 1_200, { agent_id: 'agent-id-child-9902' }),
]

test('R331: a session with a Vault-filled value gives a trace with no value', () => {
  const traces = convertHistory(session)
  assert.deepEqual([...traces.keys()], ['agent-1.jsonl', 'agent-2.jsonl'])
  const everything = [...traces.values()].join('\n')
  assert.ok(!everything.includes(secret), 'the secret is in a trace')
  for (const piece of secret.split(/[-_]/).filter(piece => piece.length >= 4)) assert.ok(!everything.includes(piece), `a piece of the secret is in a trace: ${piece}`)
  for (const leaked of ['session-aa11', 'agent-id', 'run-5521', 'attachment-77', 'miguel', 'example.invalid', '.pem', 'deploy', 'release', 'login']) {
    assert.ok(!everything.toLowerCase().includes(leaked), `${leaked} is in a trace`)
  }
})

test('every field of a trace is a number or one of a closed set of words', () => {
  const allowed = { v: [1], source: ['recorded'], kind: ['prompt', 'reasoning', 'text', 'tool', 'end'], markdown: ['list', 'code', 'table', 'plain'], outcome: ['ok', 'completed', 'error'] }
  for (const [filename, contents] of convertHistory(session)) {
    for (const [index, line] of contents.trim().split('\n').entries()) {
      for (const [key, value] of Object.entries(JSON.parse(line))) {
        if (typeof value === 'number') continue
        if (index === 0 && key === 'agent') { assert.match(value, /^agent-\d+$/, filename); continue }
        if (index === 0 && key === 'notes') { assert.equal(value, 'kinds, sizes and timings of a recorded session'); continue }
        if (key === 'name') { assert.match(value, /^(Bash|Tool)$/, filename); continue }
        assert.ok(allowed[key]?.includes(value), `${filename} line ${index + 1}: ${key}=${value}`)
      }
    }
  }
})

test('the sizes, order and timings of the session are kept', () => {
  const [first] = [...convertHistory(session).values()].map(contents => parseTrace(contents))
  assert.equal(first.turns.length, 2)
  const [one, two] = first.turns
  assert.deepEqual(one.map(step => step.kind), ['prompt', 'reasoning', 'tool', 'tool', 'text', 'end'])
  assert.equal(one[0].chars, session[0].text.length)
  assert.deepEqual(one.map(step => step.t), [0, 400, 1_000, 1_600, 2_000, 2_000])
  assert.equal(one[1].stream_ms, 600)
  assert.equal(one[2].name, 'Tool')
  assert.equal(one[3].name, 'Bash')
  assert.equal(one[3].output_bytes, session[3].text.length)
  assert.equal(one[4].markdown, 'list')
  assert.equal(one.at(-1).outcome, 'completed')
  assert.deepEqual(two.map(step => step.kind), ['prompt', 'text', 'end'])
  assert.equal(two[1].markdown, 'table')
})

test('a turn that met an error ends with the error outcome', () => {
  const [first] = [...convertHistory(session).values()].map(contents => parseTrace(contents))
  assert.equal(first.turns[1].at(-1).outcome, 'error')
})

test('shape helpers read only the shape of the text', () => {
  assert.equal(markdownOf('```js\nx\n```'), 'code')
  assert.equal(markdownOf('plain words'), 'plain')
  assert.equal(toolName('Read /etc/shadow'), 'Read')
  assert.equal(toolName('mcp__chariox__send'), 'mcp__chariox__send')
  assert.equal(toolName('secret-value --flag'), 'Tool')
  assert.equal(toolName('rm -rf'), 'Tool')
})

test('a session with no prompt gives no trace', () => {
  assert.equal(convertHistory([row('provider_output', 'orphan', 1)]).size, 0)
})

test('an outline from the kernel is read by size, and its text fragment never reaches the trace', () => {
  const entry = (kind, text, timestamp_ms, total_chars) => ({ total_chars, entry: { kind, text, timestamp_ms } })
  const outline = { agents: [{ agent_id: 'agent-id-1', turns: [{
    user_prompt: entry('user_prompt', `use ${secret}`, 100, 4_000),
    entries: [entry('provider_tool', `Bash ${secret}`, 200, 90_000), entry('provider_output', `the value ${secret}`, 700, 12_000)],
    summary: null,
    blobs: [{ kind: 'provider_output', title: `title ${secret}`, summary: `summary ${secret}`, preview: `preview ${secret}`, total_chars: 33_000, timestamp_ms: 900 }],
  }] }] }
  const traces = convertHistory(rowsFromOutline(outline))
  assert.ok(![...traces.values()].join('').includes(secret))
  const steps = parseTrace(traces.get('agent-1.jsonl')).turns[0]
  assert.deepEqual(steps.map(step => [step.kind, step.chars ?? step.output_bytes]), [['prompt', 4_000], ['tool', 90_000], ['text', 12_000], ['text', 33_000], ['end', undefined]])
})

// The kernel forces a long final reply into blobs and keeps a truncated summary of the same events: one reply, recorded once.
test('an outlined final reply is counted once, not as its summary and its blob', () => {
  const page = (kind, text, timestamp_ms, total_chars, extra = {}) => ({ entry_index: 12, fragment_start: 0, fragment_end: text.length, total_chars, entry: { kind, text, timestamp_ms }, ...extra })
  const outline = { agents: [{ agent_id: 'agent-id-1', turns: [{
    user_prompt: page('user_prompt', 'go', 100, 2),
    entries: [],
    summary: page('provider_output', 'x'.repeat(16_384), 900, 20_000, { entry_index: 12 }),
    blobs: [{ kind: 'provider_output', sequence_start: 12, sequence_end: 13, total_chars: 20_000, timestamp_ms: 900 }],
  }] }] }
  const steps = parseTrace(convertHistory(rowsFromOutline(outline)).get('agent-1.jsonl')).turns[0]
  assert.deepEqual(steps.map(step => [step.kind, step.chars]), [['prompt', 2], ['text', 20_000], ['end', undefined]])
})

// A steering prompt inside a turn is part of that turn: replay must not stop at a synthetic end and wait for another prompt.
test('a steering prompt between two outputs keeps one turn', () => {
  const page = (kind, text, timestamp_ms) => ({ entry_index: timestamp_ms, fragment_start: 0, fragment_end: text.length, total_chars: text.length, entry: { kind, text, timestamp_ms } })
  const outline = { agents: [{ agent_id: 'agent-id-1', turns: [{
    user_prompt: page('user_prompt', 'start', 100),
    entries: [page('provider_output', 'first part', 200), page('user_prompt', 'steer now', 300), page('provider_output', 'second part', 400)],
    summary: null,
    blobs: [{ kind: 'user_prompt', steering: true, sequence_start: 7, sequence_end: 7, total_chars: 9, timestamp_ms: 350 }],
  }] }] }
  const turns = parseTrace(convertHistory(rowsFromOutline(outline)).get('agent-1.jsonl')).turns
  assert.equal(turns.length, 1, 'the steering prompt must not open a second turn')
  assert.deepEqual(turns[0].map(step => step.kind), ['prompt', 'text', 'text', 'end'])
})
