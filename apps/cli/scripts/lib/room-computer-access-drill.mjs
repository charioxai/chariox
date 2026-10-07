// MP-08 / MP-10 / MP-11: drive the real TUI, observe the kernel's shared grants.
// The caller owns binaries, real accounts, evidence and process cleanup.
import assert from 'node:assert/strict';

export async function verifyRoomComputerBulkAccess({ submitCommand, snapshot, readTuiText, captureEvidence, waitFor }) {
  const before = await snapshot();
  const ids = before.room_computer.map(row => row.agent_id).sort();
  assert(ids.length > 0, 'MP-11 bulk access drill requires owned agents');
  await submitCommand('/access revoke all');
  const revoked = await waitFor(async () => {
    const result = await snapshot();
    return result.cursor > before.cursor && result.room_computer.length === ids.length
      && result.room_computer.every(row => !row.allowed) && result;
  }, 'MP-11 bulk revoke');
  await waitFor(async () => (await readTuiText()).includes('Revoked access and Room Computer control for all agents.'), 'MP-08 bulk revoke notice');
  await captureEvidence('bulk-revoke', revoked);

  await submitCommand('/access grant all');
  const restored = await waitFor(async () => {
    const result = await snapshot();
    return result.cursor > revoked.cursor && result.room_computer.length === ids.length
      && result.room_computer.every(row => row.allowed) && result;
  }, 'MP-11 bulk restore');
  assert.deepEqual(restored.room_computer.map(row => row.agent_id).sort(), ids, 'MP-11 restore preserves agents');
  await waitFor(async () => (await readTuiText()).includes('Restored Room Computer control for all agents.'), 'MP-08 bulk restore notice');
  await captureEvidence('bulk-restore', restored);
  return { items: ['MP-08', 'MP-10', 'MP-11'], agents: ids.length, restored: true, notices: true };
}
