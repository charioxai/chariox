import assert from 'node:assert/strict';

// Exact libc probe output. Observer assertions and negative controls are
// separate receipts and must not inflate this native check count.
export const NATIVE_CHECKS = Object.freeze([
  'constructor_already_confined', 'environment_filtered', 'ambient_descriptor_closed', 'constructor_host_read_denied',
  'trusted_bootstrap', 'supplementary_groups_empty', 'unprivileged_identity',
  'inherited_sdk_bidirectional', 'native_thread_create_join',
  'private_data_write', 'private_tmp_write', 'package_write_denied', 'parent_traversal_denied',
  'unrelated_host_read_denied', 'package_read', 'package_executable_mapping_denied',
  'package_mapping_cannot_become_executable', 'anonymous_memory', 'raw_network_denied',
  'fork_denied', 'foreign_signal_denied', 'native_limit_query', 'native_limit_mutation_denied',
  'foreign_limit_query_denied', 'private_descriptor_copy', 'private_file_watch', 'private_file_watch_event',
  'escaped_file_watch_denied', 'private_file_watch_remove', 'exec_denied',
]);

export function parseNativeChecks(stdout) {
  const entries = stdout.trim().split('\n').map(line => line.split(':'));
  assert.equal(entries.length, NATIVE_CHECKS.length, 'native probe check count');
  assert.equal(new Set(entries.map(([name]) => name)).size, NATIVE_CHECKS.length, 'duplicate native check');
  assert.deepEqual(entries.map(([name]) => name).sort(), [...NATIVE_CHECKS].sort(), 'native probe check names');
  for (const entry of entries) {
    assert.equal(entry.length, 2, 'native probe record shape');
    assert.equal(entry[1], 'ok', `native probe ${entry[0]}`);
  }
  return Object.fromEntries(entries);
}
