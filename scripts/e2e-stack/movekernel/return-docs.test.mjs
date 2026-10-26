// MP-08 / MP-09 / MP-10 / MP-11: user-facing Return instructions match the supported workflow.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
test('Return instructions use the original account and actionable conflict advice', async () => {
  const docs = await readFile(new URL('../../../docs/NATIVE_PROVIDER_CHECKPOINTS.md', import.meta.url), 'utf8');
  assert.match(docs, /reuses the original home account\s+profile/);
  const errors = await readFile(new URL('../../../apps/kernel/src/provider/checkpoint.rs', import.meta.url), 'utf8');
  assert.ok(!errors.includes('choose a fresh destination profile'), 'Return must not offer an unavailable separate-profile workflow');
});
