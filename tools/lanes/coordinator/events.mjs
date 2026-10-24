// MP-08 / MP-09 / MP-10 / MP-11: one inbox, durable batch archival before acknowledgement.
import { readFile, readdir } from 'node:fs/promises';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';
import { LaneError, save } from '../lanes.mjs';

const read = async path => JSON.parse(await readFile(path, 'utf8'));
export async function openInbox(home, cli) {
  const { subscription } = await cli(['events', 'subscription']);
  if (!subscription || subscription.ended) throw new LaneError('subscription_unavailable');
  const statePath = join(home, 'subscription.json');
  let previous; try { previous = await read(statePath); } catch (error) { if (error.code !== 'ENOENT') throw error; }
  if (previous && previous.subscription_id !== subscription.subscription_id) throw new LaneError('subscription_changed', 'The access grant changed; restore the original grant before resuming its inbox.');
  const records = await Promise.all((await readdir(home)).filter(f => /^[A-Za-z0-9][A-Za-z0-9_.-]*\.json$/.test(f) && !f.endsWith('.launch.json')).map(f => read(join(home, f))));
  for (const r of records.filter(r => r.launch && !r.stopped)) {
    if (!subscription.targets?.some(t => t.kernel === r.launch.kernel && t.session_id === r.launch.sessionId && (!t.agent_id || t.agent_id === r.launch.agentId))) throw new LaneError('subscription_target_missing', `Launch target ${r.lane} is missing from the inbox; subscribe before waiting.`);
  }
  await save(statePath, { subscription_id: subscription.subscription_id });
  return subscription;
}

export async function waitBatch(home, cli, now, seconds) {
  const batch = await cli(['events', 'wait', '--timeout', String(seconds)]);
  if (!Array.isArray(batch.events) || !Number.isSafeInteger(batch.next_cursor)) throw new LaneError('invalid_event_batch');
  const archive = join(home, 'events', `${now()}-${randomUUID()}.json`);
  await save(archive, batch);
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
    archives.push(archive);
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
  throw new LaneError('stop_confirmation_timeout', 'The worker stopped, but no matching removal event arrived in 120 seconds; resume stop to confirm its retained receipt.');
}
