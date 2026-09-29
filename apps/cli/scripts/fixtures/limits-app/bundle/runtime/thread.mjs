// One pool thread for the `threads` tool: hold a touched Buffer, report that
// it is held, and stay alive.
import { parentPort, workerData } from 'node:worker_threads';
const held = Buffer.alloc(workerData.mb * 1024 * 1024, 1);
parentPort.postMessage('held');
setInterval(() => { held[0] ^= 1; }, 1000);
