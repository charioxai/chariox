// MP-08 / MP-10 / MP-11: keep the coordinator inventory tied to its frozen API.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

test('lane file-transfer inventory names implemented frozen requests', () => {
  const base = '1144cfc5a1345666b64d17949f6633992550aaba';
  // Exact request enum from the frozen API; available in shallow checkouts.
  const requests = readFileSync(new URL(`./fixtures/lanes-request-api-${base.slice(0, 8)}.rs`, import.meta.url), 'utf8');
  const runbook = readFileSync(new URL('../docs/LANES_THROUGH_CHARIOX.md', import.meta.url), 'utf8');
  const names = [...new Set(runbook.match(/\b(?:Stat|Read|Write)(?:Machine|Kernel)File\b/g))];
  assert.equal(names.length, 3, 'inventory must cover stat, download and upload');
  for (const name of names) assert.match(requests, new RegExp(`\\b${name}\\(`), `${name} is absent from the frozen request enum`);
});
