// One pool thread for the `threads` tool: hold a touched Buffer and stay alive.
import { workerData } from 'node:worker_threads';
const held = Buffer.alloc(workerData.mb * 1024 * 1024, 1);
setInterval(() => { held[0] ^= 1; }, 1000);
