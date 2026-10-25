// MP-08 / MP-09 / MP-10 / MP-11: one inbox, durable batch archival before acknowledgement.
import { readFile, readdir } from 'node:fs/promises';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';
import { LaneError, save } from '../lanes.mjs';

const read = async path => JSON.parse(await readFile(path, 'utf8'));
export async function openInbox(home, cli, settleMs = 1000) {
  let { subscription } = await cli(['events', 'subscription']);
  if (!subscription || subscription.ended) throw new LaneError('subscription_unavailable');
  const statePath = join(home, 'subscription.json');
  let previous; try { previous = await read(statePath); } catch (error) { if (error.code !== 'ENOENT') throw error; }
  if (previous && previous.subscription_id !== subscription.subscription_id) throw new LaneError('subscription_changed', 'The access grant changed; restore the original grant before resuming its inbox.');
  const records = await Promise.all((await readdir(home)).filter(f => /^[A-Za-z0-9][A-Za-z0-9_.-]*\.json$/.test(f) && !f.endsWith('.launch.json')).map(f => read(join(home, f))));
  let repaired = false;
  for (const r of records.filter(r => r.launch && !r.stopped)) {
    const wanted = [
      { session_id: r.launch.sessionId },
      { session_id: r.launch.sessionId, agent_id: r.launch.agentId },
      ...(r.launch.operationsSessionId ? [{ session_id: r.launch.operationsSessionId }] : []),
    ];
    if (wanted.some(w => !subscription.targets?.some(t => t.kernel === r.launch.kernel && t.session_id === w.session_id && t.agent_id === w.agent_id))) {
      await followLaunch(cli, r.launch);
      repaired = true;
    }
  }
  if (repaired) {
    subscription = (await cli(['events', 'subscription'])).subscription;
    for (const r of records.filter(r => r.launch && !r.stopped)) {
      if (!subscription.targets?.some(t => t.kernel === r.launch.kernel && t.session_id === r.launch.sessionId && t.agent_id === r.launch.agentId)) {
        throw new LaneError('subscription_target_missing', `Worker ${r.lane} is still missing after subscribing; retry after the kernel is ready.`);
      }
    }
  }
  // A destroyed agent that stays a target makes the kernel refuse the whole stream (every lane goes silent).
  // Stopped workers lose their agent target; their sessions go once the removal event has been seen.
  const targets = new Set();
  for (const r of records.filter(r => r.launch && r.stopped)) {
    const wanted = [[r.launch.sessionId, r.launch.agentId], ...(r.stop?.removal ? [[r.launch.sessionId], [r.launch.operationsSessionId]] : [])];
    for (const [session, agent] of wanted) {
      if (session && subscription.targets?.some(t => t.kernel === r.launch.kernel && t.session_id === session && (t.agent_id ?? undefined) === agent)) targets.add(JSON.stringify([r.launch.kernel, session, agent]));
    }
  }
  for (const target of targets) {
    const [kernel, session, agent] = JSON.parse(target);
    await cli(['events', 'unsubscribe', '--kernel', kernel, '--session', session, ...(agent ? ['--agent', agent] : [])]);
  }
  // The kernel clears its refusal a moment after the dead target goes, so look again before giving up.
  let current = targets.size ? (await cli(['events', 'subscription'])).subscription : subscription;
  const refusal = () => current.kernels?.find(k => k.error?.code === 'events_refused');
  for (let look = 0; (targets.size || repaired) && refusal() && look < 8; look++) {
    await new Promise(resolve => setTimeout(resolve, settleMs));
    current = (await cli(['events', 'subscription'])).subscription;
  }
  const refused = refusal();
  if (refused) throw new LaneError('subscription_unhealthy', `Kernel ${refused.kernel} refuses the event stream (${refused.error.message}); unsubscribe the removed target it names, then resume.`);
  await save(statePath, { subscription_id: subscription.subscription_id });
  return current;
}

// A new grant needs both Room and agent targets; the routed agent stream is admitted separately.
export async function followLaunch(cli, launch) {
  for (const session of [launch.sessionId, launch.operationsSessionId].filter(Boolean)) await cli(['events', 'subscribe', '--kernel', launch.kernel, '--session', session]);
  try { await cli(['events', 'subscribe', '--kernel', launch.kernel, '--session', launch.sessionId, '--agent', launch.agentId]); }
  catch (error) {
    if (!['agent_not_found', 'agent_not_in_session', 'session_not_found'].includes(error.code)) throw error;
    const lost = new LaneError('worker_lost', `Worker ${launch.agentId} no longer exists in ${launch.sessionId}; collect its work without calling it finished.`);
    lost.launch = launch;
    throw lost;
  }
}

// Called before a worker is destroyed: the agent target would be dead the moment it is gone.
export async function dropAgentTarget(cli, launch) {
  await cli(['events', 'unsubscribe', '--kernel', launch.kernel, '--session', launch.sessionId, '--agent', launch.agentId]);
}

export async function waitBatch(home, cli, now, seconds) {
  const batch = await cli(['events', 'wait', '--timeout', String(seconds)]);
  if (!Array.isArray(batch.events) || !Number.isSafeInteger(batch.next_cursor)) throw new LaneError('invalid_event_batch');
  // A wake that timed out empty has nothing to lose, so a standing service does not write one file per wake.
  const worthKeeping = batch.events.length > 0;
  const archive = join(home, 'events', `${now()}-${randomUUID()}.json`);
  if (worthKeeping) await save(archive, batch);
  if (batch.events.some(e => ['gap', 'subscription_ended'].includes(e.type))) throw new LaneError('event_stream_incomplete', 'Inbox gap or end archived; inspect before acknowledging.');
  return { batch, archive };
}

export async function confirmSteer({ home, cli, now, target, steerId, confirmed }) {
  const deadline = Date.now() + 120_000;
  const archives = [];
  let remaining = 120;
  while (remaining > 0) {
    const { batch, archive } = await waitBatch(home, cli, now, remaining);
    archives.push(archive);
    const event = batch.events.find(e => e.type === 'steer' && e.value?.steer_id === steerId && e.kernel === target.kernel && e.session_id === target.sessionId && e.agent_id === target.agentId);
    if (event) {
      const confirmation = { mode: 'event', event, archives };
      // A retry can reuse this receipt even if acknowledgement is interrupted.
      await confirmed(confirmation);
      await cli(['events', 'ack', String(batch.next_cursor)]);
      return confirmation;
    }
    if (batch.events.length) await cli(['events', 'ack', String(batch.next_cursor)]);
    if (batch.timed_out) break;
    remaining = Math.ceil((deadline - Date.now()) / 1000);
  }
  throw new LaneError('steer_confirmation_timeout', 'Steer accepted but no matching event arrived in 120 seconds; resume this delivery to confirm it.');
}

const isRemoval = target => e => e.type === 'session' && e.value?.state === 'agent_removed' && e.kernel === target.kernel && e.session_id === target.sessionId && e.agent_id === target.agentId;

// Another consumer may have archived and acknowledged the removal after an earlier stop timed out.
async function archivedRemoval(home, target) {
  let names; try { names = (await readdir(join(home, 'events'))).filter(f => f.endsWith('.json')).sort(); } catch (error) { if (error.code === 'ENOENT') return undefined; throw error; }
  for (const name of names) {
    const archive = join(home, 'events', name);
    const event = (await read(archive)).events?.find(isRemoval(target));
    if (event) return { event, archives: [archive] };
  }
}

export async function confirmRemoval({ home, cli, now, target, confirmed }) {
  const archived = await archivedRemoval(home, target);
  if (archived) { await confirmed(archived); return; }
  const deadline = Date.now() + 120_000;
  const archives = [];
  let remaining = 120;
  while (remaining > 0) {
    const { batch, archive } = await waitBatch(home, cli, now, remaining);
    if (batch.events.length) archives.push(archive); // an empty wake is not written
    const event = batch.events.find(isRemoval(target));
    if (event) {
      await confirmed({ event, archives });
      await cli(['events', 'ack', String(batch.next_cursor)]);
      return;
    }
    if (batch.events.length) await cli(['events', 'ack', String(batch.next_cursor)]);
    if (batch.timed_out) break;
    remaining = Math.ceil((deadline - Date.now()) / 1000);
  }
  // Batches consumed while waiting belong to other lanes too: the caller still has to handle them.
  throw Object.assign(new LaneError('stop_confirmation_timeout', 'The worker stopped, but no matching removal event arrived in 120 seconds; resume stop to confirm its retained receipt.'), { archives });
}
