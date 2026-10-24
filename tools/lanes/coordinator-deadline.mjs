// MP-08 / MP-09 / MP-10 / MP-11: best-effort policy when a kernel lacks deadlines.
import { readFile, writeFile } from 'node:fs/promises';
import { run } from './lanes.mjs';

export async function enforceCoordinatorDeadline(stateFile, dependencies = {}) {
  const state = JSON.parse(await readFile(stateFile, 'utf8'));
  if (state.deadline?.mode !== 'coordinator' || !Number.isSafeInteger(state.deadlineMs) || state.deadlineMs <= 0) throw Error('invalid coordinator deadline receipt');
  await writeFile(`${stateFile}.deadline.json`, JSON.stringify({status:'waiting',mode:'coordinator',deadlineMs:state.deadlineMs,pid:process.pid}) + '\n', {mode:0o600});
  const now = dependencies.now || Date.now;
  const wait = dependencies.wait || (ms => new Promise(ok => setTimeout(ok, ms)));
  while (now() < state.deadlineMs) await wait(Math.min(state.deadlineMs - now(), 2_147_483_647));
  let receipt;
  try {
    const result = await (dependencies.stop || (input => run('stop', input)))({kernel:state.kernel,taskId:state.taskId,sessionId:state.sessionId,agentId:state.agentId,attachmentId:state.attachmentId});
    receipt = {status:'stopped',mode:'coordinator',deadlineMs:state.deadlineMs,result};
  } catch (error) {
    const code = typeof error.code === 'string' && /^[a-z][a-z0-9_]{0,79}$/.test(error.code) ? error.code : 'coordinator_timer_failed';
    receipt = {status:'failed',mode:'coordinator',deadlineMs:state.deadlineMs,code,message:'No kernel deadline: coordinator cleanup failed. Restore the coordinator connection/access grant and stop the lane.'};
  }
  await writeFile(`${stateFile}.deadline.json`, JSON.stringify(receipt) + '\n', {mode:0o600});
  return receipt;
}
