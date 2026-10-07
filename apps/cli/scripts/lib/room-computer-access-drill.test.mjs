// MP-08 / MP-10 / MP-11: bulk Room Computer drill flow against a fake TUI and kernel snapshot.
import assert from 'node:assert/strict';
import test from 'node:test';
import { verifyRoomComputerBulkAccess } from './room-computer-access-drill.mjs';

function fakeKernelTui({ restore = ids => ids, restoreNotice = 'Restored Room Computer control for all agents.' } = {}) {
  let cursor = 1, tui = '';
  let rows = ['b', 'a'].map(agent_id => ({ agent_id, session_id: 's', allowed: true }));
  const evidence = [];
  return {
    evidence,
    submitCommand: async command => {
      cursor += 1;
      if (command === '/access revoke all') {
        rows = rows.map(row => ({ ...row, allowed: false }));
        tui += 'Revoked access and Room Computer control for all agents.\n';
      } else if (command === '/access grant all') {
        const allowed = restore(rows.map(row => row.agent_id));
        rows = rows.map(row => ({ ...row, allowed: allowed.includes(row.agent_id) }));
        tui += restoreNotice + '\n';
      } else throw new Error('unexpected command ' + command);
    },
    snapshot: async () => ({ cursor, room_computer: rows.map(row => ({ ...row })) }),
    readTuiText: async () => tui,
    captureEvidence: async (label, result) => { evidence.push([label, result.cursor]); },
    waitFor: async (work, label) => {
      for (let attempt = 0; attempt < 3; attempt++) { const value = await work(); if (value) return value; }
      throw new Error('timeout: ' + label);
    },
  };
}

test('MP-11 bulk revoke then bulk restore passes with both notices and evidence', async () => {
  const fake = fakeKernelTui();
  assert.deepEqual(await verifyRoomComputerBulkAccess(fake), { items: ['MP-08', 'MP-10', 'MP-11'], agents: 2, restored: true, notices: true });
  assert.deepEqual(fake.evidence, [['bulk-revoke', 2], ['bulk-restore', 3]]);
});

test('MP-11 a partial bulk restore fails the drill', async () => {
  await assert.rejects(verifyRoomComputerBulkAccess(fakeKernelTui({ restore: ids => ids.slice(1) })), /timeout: MP-11 bulk restore/);
});

test('MP-08 a missing Room Computer restore notice fails the drill', async () => {
  await assert.rejects(verifyRoomComputerBulkAccess(fakeKernelTui({ restoreNotice: 'Restored access.' })), /timeout: MP-08 bulk restore notice/);
});

test('MP-11 the drill refuses to run without owned agents', async () => {
  const fake = fakeKernelTui();
  fake.snapshot = async () => ({ cursor: 1, room_computer: [] });
  await assert.rejects(verifyRoomComputerBulkAccess(fake), /requires owned agents/);
});
