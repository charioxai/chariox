// Tries to build the async probe with componentize-js; prints the outcome.
import { componentize } from '@bytecodealliance/componentize-js';
import { readFile, writeFile } from 'node:fs/promises';
const root = process.env.SPIKE_ROOT;
try {
  const { component } = await componentize({
    sourcePath: `${root}async-probe/js/probe.js`,
    witWorld: await readFile(`${root}async-probe/wit/probe.wit`, 'utf8'),
    worldName: 'probe',
    disableFeatures: ['http', 'fetch-event'],
  });
  await writeFile(process.argv[2], component);
  console.log(`built ${component.length} bytes`);
} catch (error) {
  console.log(`componentize-js refused: ${String(error?.message ?? error).split('\n').slice(0, 3).join(' | ')}`);
}
