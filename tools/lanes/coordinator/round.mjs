// MP-08 / MP-09 / MP-10 / MP-11: run a fixed set of lanes to the end (`round.mjs <spec>`), or serve the queue (`round.mjs serve`).
import { readFile } from 'node:fs/promises';
import { createRound } from './engine.mjs';
export { createRound, newSections } from './engine.mjs';

const read = async path => JSON.parse(await readFile(path, 'utf8'));

export async function runRound(spec, options = {}) {
  const round = createRound({ ...options, home: options.home || spec.home });
  for (const entry of spec.lanes) await round.add(entry);
  try { while (round.pending().length) await round.step(spec.wakeMinutes ?? 5); }
  catch (error) { await round.failed(error); throw error; }
  return round.lanes.map(lane => lane.run);
}

if (import.meta.url === `file://${process.argv[1]}`) {
  try {
    if (process.argv[2] === 'stop') console.log(JSON.stringify(await (await import('./round-stop.mjs')).stopRound(process.argv[3], { expectedRound: process.argv[4] })));
    else if (process.argv[2] === 'serve') await (await import('./loopd.mjs')).main();
    else {
      const spec = await read(process.argv[2]);
      spec.home ??= process.env.CHARIOX_COORDINATOR_HOME;
      const runs = await runRound(spec);
      console.log(JSON.stringify(runs.map(run => ({ lane: run.lane, finishReason: run.finishReason, head: run.head }))));
    }
  } catch (error) {
    console.error(JSON.stringify({ error: { code: error.code || 'round_failed', message: error.code ? error.message : 'Round failed; inspect retained receipts.' } }));
    process.exitCode = 1;
  }
}
