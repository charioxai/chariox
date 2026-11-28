import assert from 'node:assert/strict'
import test from 'node:test'
import check from './runtime-card-registry.mjs'

test('rejects a raw card push outside the registry', () => {
  const file = 'apps/kernel/src/runtime/rogue_card.rs'
  const source = [
    '// active_interactions.push(a_comment);',
    '#[cfg(test)]',
    'fn fixture() { session.add_active_interaction(a_test_card); }',
    'fn raise() { session.active_interactions.push(raw); }',
  ].join('\n')
  assert.deepEqual(check.scan({ files: [file], read: () => source }), [
    { file, line: 4, what: 'adds a card outside its registry' },
  ])
})

// Decisions in these fixtures are synthetic; they are never production approvals.
const typesFile = 'docs/runtime-card-types.json'
const decisionsFile = 'docs/card-decisions.json'
const fixture = (types, decisions = [], before = []) => ({
  files: [typesFile, decisionsFile], base: 'fixture-base',
  read: file => JSON.stringify(file === typesFile ? types : decisions),
  readAtBase: file => JSON.stringify(file === typesFile ? before : []),
})
const syntheticDecision = (type, classification) => ({ id: `synthetic-${type}-${classification}`, types: [{ type, class: classification }], approved_by: 'Miguel', source: 'synthetic test fixture, not an owner approval' })

test('an unregistered test-PR type fails closed without its individual decision', () => {
  const entries = [{ type: 'fixture_new_card', class: 'person action or answer' }]
  assert.ok(check.scan(fixture(entries)).some(f => f.what.includes('fixture_new_card')))
})

test('an individually registered type with its exact class decision passes', () => {
  const entry = { type: 'fixture_cloud_only', class: 'asked-for notification' }
  assert.deepEqual(check.scan(fixture([entry], [syntheticDecision(entry.type, entry.class)])), [])
})

test('a class change needs a decision for that type and new class', () => {
  const before = { type: 'fixture_card', class: 'person action or answer' }
  const after = { ...before, class: 'live browser card' }
  assert.ok(check.scan(fixture([after], [syntheticDecision(before.type, before.class)], [before])).length)
  assert.deepEqual(check.scan(fixture([after], [syntheticDecision(after.type, after.class)], [before])), [])
})

test('a category policy is not an exact type approval', () => {
  assert.ok(check.scan(fixture([{ type: 'fixture_card', class: 'person action or answer' }], [syntheticDecision('*', 'person action or answer')])).length)
})

test('duplicate types and unknown classes fail closed', () => {
  const entry = { type: 'fixture_card', class: 'status' }
  assert.ok(check.scan(fixture([entry, entry])).length)
})


test('a test PR cannot emit a type it did not individually register', () => {
  const file = 'apps/kernel/src/runtime/new_card.rs'
  const ctx = fixture([])
  ctx.files.push(file)
  const read = ctx.read
  ctx.read = name => name === file ? 'fn raise() { interaction.with_card_type("fixture_unregistered"); }' : read(name)
  assert.ok(check.scan(ctx).some(f => f.file === file && f.what.includes('fixture_unregistered')))
})

test('an old approval cannot be rewritten to authorize a changed class', () => {
  const old = { type: 'fixture_immutable', class: 'person action or answer' }
  const next = { ...old, class: 'live browser card' }
  const approved = syntheticDecision(old.type, old.class)
  const edited = { ...approved, types: [next] }
  const ctx = fixture([next], [edited], [old])
  ctx.readAtBase = file => JSON.stringify(file === typesFile ? [old] : [approved])
  assert.ok(check.scan(ctx).some(finding => finding.what.includes('approval record')))
  const added = { ...syntheticDecision(next.type, next.class), id: 'synthetic-new-class-decision' }
  ctx.read = file => JSON.stringify(file === typesFile ? [next] : [approved, added])
  assert.deepEqual(check.scan(ctx), [])
})

test('direct card constructors and the native classifier also require registration', () => {
  for (const [file, code] of [
    ['apps/kernel/src/runtime/new_card.rs', 'fn raise() { InteractionSurface::Card("fixture_unregistered"); }'],
    ['apps/kernel/src/runtime/state/runtime_interaction_owned_state/card_registry.rs', 'fn native() { Self::named("fixture_unregistered"); }'],
  ]) {
    const ctx = fixture([])
    ctx.files.push(file)
    const read = ctx.read
    ctx.read = name => name === file ? code : read(name)
    assert.ok(check.scan(ctx).some(finding => finding.file === file && finding.what.includes('fixture_unregistered')))
  }
})
