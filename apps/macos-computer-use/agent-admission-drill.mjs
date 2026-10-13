// Supplementary regression over owner IPC and the private provider MCP fixture.
// The provider holds its generated binding. This client never reads its token.
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { lstatSync, readFileSync, writeFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { createRequire } from 'node:module';
const WebSocket = createRequire(import.meta.url)('ws');

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
export function connectKernel(socketPath) {
  // Ordinary terminal admission uses this run's generated local token. Unix
  // external-agent admission is a different, passkey-gated product route.
  const tokenPath = process.env.CUMAC_LOCAL_TOKEN_FILE;
  assert(tokenPath?.startsWith(process.env.CHARIOX_HOME + '/') && !tokenPath.includes('..'), 'Only this disposable kernel token is allowed');
  assert(/^\d+$/.test(process.env.CHARIOX_KERNEL_PORT), 'Local kernel port required');
  const token = lstatSync(tokenPath), parent = lstatSync(tokenPath.slice(0, tokenPath.lastIndexOf('/')));
  assert(token.isFile() && token.uid === process.getuid() && (token.mode & 0o777) === 0o600);
  assert(parent.isDirectory() && parent.uid === process.getuid() && (parent.mode & 0o777) === 0o700);
  const ws = new WebSocket(`ws://127.0.0.1:${process.env.CHARIOX_KERNEL_PORT}/kernel`, { headers: { authorization: `Bearer ${readFileSync(tokenPath, 'utf8').trim()}` } });
  const pending = new Map();
  const listeners = new Set();
  const ready = new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = () => reject(Error('Owner IPC failed')); });
  ws.onmessage = event => {
    const frame = JSON.parse(String(event.data));
    if (frame.type === 'event') { for (const listen of listeners) listen(frame.event); return; }
    const entry = pending.get(frame.request_id);
    if (!entry) return;
    pending.delete(frame.request_id); clearTimeout(entry.timer);
    if (frame.error || frame.response?.Error) entry.reject(Error(JSON.stringify(frame.error ?? frame.response.Error)));
    else entry.resolve(frame.response);
  };
  ws.onclose = () => { for (const entry of pending.values()) { clearTimeout(entry.timer); entry.reject(Error('Owner IPC closed')); } pending.clear(); };
  async function frameSend(frame) {
    await ready;
    const id = randomUUID();
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { pending.delete(id); reject(Error('Owner IPC timeout')); }, 24000);
      pending.set(id, { resolve, reject, timer });
      ws.send(JSON.stringify({ request_id: id, command_id: id, ...frame }));
    });
  }
  return { ready, close: () => ws.close(), onEvent: listen => { listeners.add(listen); return () => listeners.delete(listen); },
    subscribe: (session, attachment) => frameSend({ type: 'subscribe', session_id: session, attachment_id: attachment, resume_from_event_id: null }),
    send: request => frameSend({ type: 'request', request }) };
}

export async function createAgentCallback(client, workspace) {
  const created = (await client.send({ CreateSession: { workspace_id: workspace, worktree_id: workspace, alias: 'Mac admission regression' } })).SessionCreated;
  const session = created.session.id, agent = created.agent.id;
  const attachment = (await client.send({ AttachToSession: { session_id: session, client_id: 'cumac-agent-drill', capability_level: 'FullTerminal' } })).SessionAttached.attachment.id;
  let output = '';
  const counts = { records: 0, bytes: 0, events: {} };
  const seen = new Set();
  function receive(records) { for (const record of records ?? []) {
    counts.records++; counts.bytes += record.bytes?.length ?? 0;
    if (record.record_id !== undefined && seen.has(record.record_id)) continue;
    if (record.record_id !== undefined) seen.add(record.record_id);
    output += Buffer.from(record.bytes).toString('utf8');
  }}
  const unsubscribe = client.onEvent(event => { counts.events[event.event] = (counts.events[event.event] ?? 0) + 1; if (event.event === 'terminal_output') receive(event.records); });
  const launched = await client.send({ LaunchProviderRun: { session_id: session, agent_id: agent, adapter_key: 'dev-stub', provider: 'dev-stub', account_profile: 'default', model: 'runtime-mcp-fixture', variant: null, native_tui: true } });
  const run = (launched.ProviderRunLaunched ?? launched.ProviderRunLaunchAccepted).provider_run.id;
  const readyDeadline = Date.now() + 25000;
  let runState;
  do {
    const current = (await client.send({ GetProviderRun: { provider_run_id: run } })).ProviderRun.provider_run;
    assert.equal(current.model, 'runtime-mcp-fixture', 'Provider must retain the requested fixture model');
    runState = current.state;
    if (runState === 'Ended') throw Error('Private provider ended before MCP callback');
    if (runState === 'Running') break;
    await sleep(30);
  } while (Date.now() < readyDeadline);
  assert.equal(runState, 'Running', 'Private provider must be running');
  await client.send({ FocusAgent: { session_id: session, agent_id: agent } });
  async function pump() {
    const records = (await client.send({ PumpTerminalOutput: { session_id: session, attachment_id: attachment } })).TerminalOutput.records;
    receive(records);
  }
  async function until(predicate) {
    const deadline = Date.now() + 25000;
    while (Date.now() < deadline) { await pump(); const value = predicate(output); if (value) return value; await sleep(30); }
    const current = (await client.send({ GetProviderRun: { provider_run_id: run } })).ProviderRun.provider_run;
    throw Error('Private provider callback timeout: ' + JSON.stringify({ ...counts, state: current.state, interface: current.client_interface, model: current.model }) + ' ' + output.replace(/chx_[A-Za-z0-9_-]+/g, '[REDACTED]').slice(-2000));
  }
  // Ready output may have been pumped during launch. The request itself waits
  // for the provider, and its newline also clears any startup terminal input.
  async function call(method, params) {
    const id = randomUUID();
    const message = `\nMP11_MCP_REQUEST ${id} ${Buffer.from(JSON.stringify({ method, params })).toString('base64')}\n`;
    await pump();
    // A real provider turn keeps its output in the kernel transcript. Idle
    // dev-stub PTY chatter is deliberately discarded by the output pump.
    await client.send({ SubmitPrompt: { session_id: session, attachment_id: attachment, target_agent_id: agent, prompt: message, attachments: [] } });
    return until(text => { const match = text.match(new RegExp(`MP11_MCP_RESULT ${id} ([A-Za-z0-9+/=]+)[\\r\\n]`)); return match && JSON.parse(Buffer.from(match[1], 'base64').toString()); });
  }
  // The owner socket can precede the MCP listener during normal startup.
  // Wait for discovery before testing Computer admission; never retry effects.
  let discovery;
  for (let attempt = 0; attempt < 5; attempt++) {
    discovery = await call('tools/list', {});
    if (discovery.result?.tools) break;
    await sleep(100);
  }
  assert(discovery.result?.tools, 'Private provider MCP must be ready');
  return { session, agent, run, call, input: command => call('tools/call', { name: 'chariox.kernel_computer', arguments: { command } }),
    stop: async () => { unsubscribe(); return client.send({ EndSession: { session_id: session } }); } };
}

export async function checkAgentAdmission(client, callback) {
  const result = {};
  // Dispatch independently of discovery, so the old Linux-only error is retained.
  result.loader = await callback.call('tools/call', { name: 'chariox.load_kernel_computer', arguments: {} });
  result.catalog = await callback.call('tools/list', {});
  assert(!JSON.stringify(result.loader).includes('require Linux'), 'Mac agent dispatch must reach provider admission');
  assert(result.catalog.result.tools.some(tool => tool.name === 'chariox.load_kernel_computer'), 'Mac provider must discover Computer loader');
  assert(result.catalog.result.tools.some(tool => tool.name === 'chariox.kernel_computer'), 'Loaded Mac provider must discover Computer');
  assert(!result.loader.error && !result.loader.result?.isError, 'Computer loader must admit the focused provider: ' + JSON.stringify(result.loader));
  result.agentStart = await callback.input({ op: 'start' });
  assert(JSON.stringify(result.agentStart).includes('owner starts this Computer seat'), 'Personal Mac seat still requires owner Start');
  result.agentState = await callback.input({ op: 'state' });
  assert(JSON.stringify(result.agentState).includes('explicitly start the owned desktop'), 'Agent state must reach configured Mac backend without starting it');
  result.agentRelease = await callback.input({ op: 'release', target: { surface_id: 'unstarted', generation: 'unstarted' } });
  assert(JSON.stringify(result.agentRelease).includes('human desktop channel required'), 'Agent cannot use owner Release');
  await client.send({ KernelBrowser: { command: { op: 'revoke_grants', agent_id: callback.agent } } });
  result.revoked = await callback.input({ op: 'state' });
  assert(JSON.stringify(result.revoked).includes('not_granted'), 'Revoked Mac agent must remain fenced');
  await client.send({ FocusAgent: { session_id: callback.session, agent_id: callback.agent } });
  await callback.call('tools/call', { name: 'chariox.load_kernel_computer', arguments: {} });
  return result;
}

async function main() {
  const [socket, workspace, receipt] = process.argv.slice(2);
  const client = connectKernel(socket);
  let callback;
  const report = { status: 'FAIL', scope: 'supplementary private provider, real kernel IPC/MCP; no native input' };
  try {
    callback = await createAgentCallback(client, workspace);
    report.binding = { session: callback.session, agent: callback.agent, run: callback.run };
    report.loader = await callback.call('tools/call', { name: 'chariox.load_kernel_computer', arguments: {} });
    report.checks = await checkAgentAdmission(client, callback);
    report.status = 'PASS';
  } catch (error) { report.error = error.message; process.exitCode = 1; }
  finally { if (callback) await callback.stop(); client.close(); writeFileSync(receipt, JSON.stringify(report, null, 2) + '\n', { mode: 0o600 }); }
  console.log(JSON.stringify({ status: report.status, error: report.error }));
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) await main();
