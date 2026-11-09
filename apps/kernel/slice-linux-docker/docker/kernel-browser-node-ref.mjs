// MP-08/MP-10/MP-11: agent reads that take a snapshot node_ref accept only a top-level page node.
import { AgentToolRefusal } from './kernel-browser-refusal.mjs';

export function topLevelNodeId(nodeRef) {
  const match = typeof nodeRef === 'string' ? /^backend:([1-9][0-9]{0,15})$/.exec(nodeRef) : null;
  if (!match || !Number.isSafeInteger(Number(match[1]))) throw new AgentToolRefusal('node_ref_invalid');
  return Number(match[1]);
}

// A CDP refusal for a node the page no longer holds is the agent's cue to snapshot again.
export async function withLiveNode(operation) {
  try { return await operation(); }
  catch (error) { throw error?.code === 'browser_cdp_command_failed' ? new AgentToolRefusal('node_unavailable') : error; }
}
