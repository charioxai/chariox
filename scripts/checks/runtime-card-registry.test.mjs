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
