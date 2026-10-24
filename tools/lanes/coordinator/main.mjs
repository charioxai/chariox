// MP-08 / MP-09 / MP-10 / MP-11: bounded CLI output; child diagnostics stay private.
import { coordinate } from './loop.mjs';
export async function main(operation) {
  try {
    const args = process.argv.slice(2);
    const result = operation === 'lanefin' && args.length > 1
      ? await Promise.all(args.map(lane => coordinate(operation, [lane])))
      : await coordinate(operation, args);
    console.log(JSON.stringify(result));
  }
  catch (error) { console.error(JSON.stringify({ error: { code: error.code || 'coordinator_failed', message: error.code ? error.message : 'Coordinator step failed; inspect retained receipts.' } })); process.exitCode = 1; }
}
