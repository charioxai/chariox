// Sends one probe prompt to each agent through the kernel's local IPC and
// records when it was sent. Runs inside the drill kernel's namespaces.
// node probe.mjs <ipc.js> <chariox home> <port> <session> <phase> <out tsv> <agent>...
import { appendFileSync } from 'node:fs';
const [ipc, home, port, sessionId, phase, out, ...agents] = process.argv.slice(2);
const { LocalIpcClient } = await import(ipc);
const client = new LocalIpcClient(`ws://127.0.0.1:${port}/kernel`, {
  localAuthEnvironment: { CHARIOX_HOME: home },
  controlRequestRetryDeadlineMs: 0,
  controlResponseStallMs: 60000,
});
const variant = (response, key) => {
  if (!response || !(key in response)) throw new Error(`expected ${key}, got ${Object.keys(response || {}).join(',')}`);
  return response[key];
};
try {
  const attachment = variant(await client.send({ AttachToSession: { session_id: sessionId, client_id: 'restart-stall-drill', capability_level: 'FullTerminal' } }), 'SessionAttached').attachment.id;
  for (const agent of agents) {
    const marker = `drill-probe-${phase}-${agent}`;
    const sent = Date.now();
    let result;
    try {
      result = Object.keys(await client.send({ SubmitPrompt: { session_id: sessionId, attachment_id: attachment, target_agent_id: agent, prompt: `${marker}: reply with one line.`, attachments: [] } }) || {})[0];
    } catch (error) {
      result = `failed: ${String(error?.message || error).slice(0, 120)}`;
    }
    appendFileSync(out, `${marker}\t${sent}\t${result}\n`);
  }
} finally {
  await client.close().catch(() => {});
}
