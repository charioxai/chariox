// MP-08 / MP-09 / MP-10 / MP-11: check round identity and pickup after a runtime swap.
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';

export async function verifyPickup(agents, snapshot, logPath) {
  const before = JSON.parse(await readFile(snapshot, 'utf8'));
  // Retained logs can predate stderr separation, and a writer may still be appending its last line.
  const log = (await readFile(logPath, 'utf8')).split('\n').flatMap(line => {
    try { const event = JSON.parse(line); return event && typeof event.kind === 'string' ? [event] : []; }
    catch { return []; }
  });
  const last = log.map(event => event.kind).lastIndexOf('started');
  const picked = new Set(log.slice(last).filter(event => event.kind === 'picked_up').map(event => event.key));
  const rows = [];
  for (const [key, id] of Object.entries(before)) {
    const lane = key.replace(/-r\d+\w*$/, '');
    let run;
    try { run = JSON.parse(await readFile(join(agents, lane, 'LOOP_RUN.json'), 'utf8')); }
    catch (error) { if (error.code !== 'ENOENT') throw error; run = {}; }
    const same = run.launch?.sessionId === id.sessionId && run.launch?.agentId === id.agentId && run.launchedAtMs === id.launchedAtMs;
    rows.push({ key, same, pickedUp: picked.has(key), finishReason: run.finishReason,
      ok: Boolean(same && (picked.has(key) || run.finishReason)) });
  }
  return rows;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const rows = await verifyPickup(...process.argv.slice(2));
  for (const row of rows) console.log(`${row.ok ? 'ok   ' : 'FAIL '}${row.key} same-ids=${row.same} picked_up=${row.pickedUp}${row.finishReason ? ` finished=${row.finishReason}` : ''}`);
  if (rows.some(row => !row.ok)) process.exitCode = 3;
}
