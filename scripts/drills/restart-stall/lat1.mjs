// LAT-1: with the longest-history agent mid-turn (busy) or idle, spawn a fresh
// Claude Haiku agent and prompt "hi there". Records when each step was sent;
// run.sh reads when the kernel recorded the reply. Runs in the kernel's namespaces.
// node lat1.mjs <ipc.js> <chariox home> <port> <room> <big agent> <busy|idle|seed|outline> <run> <out tsv> [second big agent | agents...]
// LAT-1b: busy turns stream on the two longest-history agents at once; "outline" reads the
// kernel's completed_at_ms of every marked turn (what the UI's Writing timer stops on).
import { appendFileSync } from 'node:fs';
const [ipc, home, port, room, bigAgent, mode, run, out, ...rest] = process.argv.slice(2);
const { LocalIpcClient } = await import(ipc);
const client = new LocalIpcClient(`ws://127.0.0.1:${port}/kernel`, {
  localAuthEnvironment: { CHARIOX_HOME: home },
  controlRequestRetryDeadlineMs: 0,
  controlResponseStallMs: 120000,
});
const variant = (response, key) => {
  if (!response || !(key in response)) throw new Error(`expected ${key}, got ${JSON.stringify(response).slice(0, 200)}`);
  return response[key];
};
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
try {
  const attachment = variant(await client.send({ AttachToSession: { session_id: room, client_id: 'restart-stall-lat1', capability_level: 'FullTerminal' } }), 'SessionAttached').attachment.id;
  if (mode === 'seed') {
    const created = await client.send({ CreateCredentialVault: { origin: { kind: 'passphrase', passphrase: 'drill-copy-vault-passphrase' } } }).catch((error) => ({ error: String(error?.message || error) }));
    await client.send({ SubmitPrompt: { session_id: room, attachment_id: attachment, target_agent_id: bigAgent, prompt: `drill-seed-${run}: store values.`, attachments: [] } });
    appendFileSync(out, `seed\t${JSON.stringify(created).slice(0, 200)}\n`);
    process.exit(0);
  }
  if (mode === 'outline') {
    for (const agent of rest) {
      const outline = await client.send({ GetSessionHistoryOutline: { session_id: room, agent_ids: [agent], latest_prompt_count: 20 } });
      const turns = [];
      const walk = (node) => {
        if (Array.isArray(node)) node.forEach(walk);
        else if (node && typeof node === 'object') {
          if ('completed_at_ms' in node && 'user_prompt' in node) turns.push(node);
          Object.values(node).forEach(walk);
        }
      };
      walk(outline);
      for (const turn of turns) {
        const marker = JSON.stringify(turn.user_prompt).match(/drill-(?:stream|turn)-[a-z0-9-]+/)?.[0];
        if (marker) appendFileSync(out, `${marker}\t${agent}\t${turn.completed_at_ms ?? ''}\t${turn.lifecycle}\n`);
      }
    }
    process.exit(0);
  }
  if (mode === 'busy') {
    await client.send({ SubmitPrompt: { session_id: room, attachment_id: attachment, target_agent_id: bigAgent, prompt: `drill-stream-${run}-a: keep working.`, attachments: [] } });
    if (rest[0]) await client.send({ SubmitPrompt: { session_id: room, attachment_id: attachment, target_agent_id: rest[0], prompt: `drill-stream-${run}-b: keep working.`, attachments: [] } });
    // The big agent streams for 15 s; the fresh agent's prompt lands as that turn ends.
    await sleep(12000);
  }
  const spawnSent = Date.now();
  const spawned = await client.send({ SpawnAgent: { session_id: room, provider: 'claude', model: 'haiku', alias: `lat1-${mode}-${run}`, effort: null, worktree_id: null, kernel_ref: null, slice_ref: null, worktree_placement: null, execution_mode: null, permission_level: null } });
  const spawnDone = Date.now();
  const agent = JSON.stringify(spawned).match(/"(agent-[0-9a-f]{32})"/)?.[1];
  if (!agent) throw new Error(`no agent in ${JSON.stringify(spawned).slice(0, 300)}`);
  const promptSent = Date.now();
  const submitted = Object.keys(await client.send({ SubmitPrompt: { session_id: room, attachment_id: attachment, target_agent_id: agent, prompt: `hi there (drill-turn-${mode}-${run})`, attachments: [] } }) || {})[0];
  appendFileSync(out, `${run}\t${mode}\t${agent}\t${spawnSent}\t${spawnDone}\t${promptSent}\t${submitted}\n`);
} catch (error) {
  appendFileSync(out, `${run}\t${mode}\tfailed\t0\t0\t0\t${String(error?.message || error).slice(0, 200)}\n`);
} finally {
  await client.close().catch(() => {});
}
