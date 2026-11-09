// MP-08/MP-09/MP-10/MP-11: wait for text, a node or part of the URL on the host, so an agent does not poll snapshots.
// The kernel asks in short slices and joins them; one slice never holds the host longer than SLICE_LIMIT_MS.
// What is compared is what the model would be shown: protected values are redacted first, so a wait is no oracle
// for them.
import { assertNotCancelled } from './browser-controller-actions.mjs';
import { redactObservation, redactSnapshot } from './browser-controller-snapshot.mjs';
import { setTimeout as delay } from 'node:timers/promises';
import { AgentToolRefusal } from './kernel-browser-refusal.mjs';
import { topLevelNodeId } from './kernel-browser-node-ref.mjs';

export const WAIT_LIMIT_MS = 30_000;
const SNAPSHOT_GAP_MS = 400;
const URL_GAP_MS = 100;
const MAX_NEEDLE = 512;

function condition(command) {
  const keys = ['text', 'node_ref', 'url_contains'].filter(key => command[key] !== undefined);
  const timeout = command.timeout_ms ?? 0;
  if (keys.length !== 1 || !Number.isInteger(timeout) || timeout < 0 || timeout > WAIT_LIMIT_MS) throw new AgentToolRefusal('wait_condition_invalid');
  const [kind] = keys, value = command[kind];
  if (kind === 'node_ref') { topLevelNodeId(value); return { kind, value, timeout }; }
  if (typeof value !== 'string' || !value || value.length > MAX_NEEDLE) throw new AgentToolRefusal('wait_condition_invalid');
  return { kind, value, timeout };
}

const mentions = (haystack, needle) => typeof haystack === 'string' && haystack.toLowerCase().includes(needle);

function snapshotMatches(snapshot, { kind, value }) {
  const nodes = [...snapshot.accessibility_nodes ?? [], ...snapshot.dom_nodes ?? []];
  if (kind === 'node_ref') return nodes.some(node => node.node_ref === value);
  const needle = value.toLowerCase();
  return nodes.some(node => mentions(node.name, needle) || mentions(node.description, needle) || mentions(node.value, needle) || mentions(node.text, needle));
}

// The target's own URL follows same-document changes (pushState, hash) that the stored Tab list learns later.
async function liveUrl(host, tab) {
  try { return (await host.browser.connection.send('Target.getTargetInfo', { targetId: tab.target_id }))?.targetInfo?.url ?? tab.url; }
  catch { return tab.url; }
}

export async function waitForPage(host, command, scope, signal) {
  const wanted = condition(command);
  const started = Date.now(), deadline = started + wanted.timeout;
  const secrets = host.protection.values;
  for (;;) {
    assertNotCancelled(signal);
    const tab = await host.displayTarget({ generation: command.generation, tab_id: command.tab_id });
    let matched;
    if (wanted.kind === 'url_contains') matched = mentions(redactObservation(await liveUrl(host, tab), secrets), wanted.value.toLowerCase());
    else matched = snapshotMatches(redactSnapshot(await host.browser.snapshot({ target_id: tab.target_id, document_id: tab.document_id }), secrets), wanted);
    const last = Date.now() >= deadline;
    if (matched || last) {
      return host.observe({ ...host.tabState(), matched, waited_ms: Date.now() - started }, tab, scope);
    }
    await delay(Math.min(wanted.kind === 'url_contains' ? URL_GAP_MS : SNAPSHOT_GAP_MS, Math.max(1, deadline - Date.now())));
  }
}
