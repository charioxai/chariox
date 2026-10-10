import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdir, mkdtemp, writeFile, readFile, rm } from 'node:fs/promises';
import { openSync, closeSync } from 'node:fs';
import net from 'node:net';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { LocalIpcClient } from '../../../packages/kernel-client/dist/ipc.js';
import * as request from '../../../packages/kernel-client/dist/ipc-requests.js';
import { createAccessCommandController } from '../dist/access-command-controller.js';
const oss = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const evidence = path.resolve(process.argv[2] ?? '/w/evidence/mdgrants');
assert.ok(!evidence.startsWith(oss + path.sep));
await mkdir(evidence + '/state', { recursive: true });
const root = await mkdtemp(evidence + '/state/live-');
const home = root + '/kernel';
await mkdir(home, { mode: 0o700 });
const freePort = async () => { const s = net.createServer(); await new Promise(r => s.listen(0, '127.0.0.1', r)); const p = s.address().port; await new Promise(r => s.close(r)); return p; };
const port = await freePort(), mcpPort = await freePort();
const log = openSync(evidence + '/live-kernel.log', 'w', 0o600);
const kernel = spawn(oss + '/target/debug/chariox-kernel', [], { cwd: oss, detached: true, stdio: ['ignore', log, log], env: { ...process.env,
  CHARIOX_RELAY_URL: `ws://127.0.0.1:${await freePort()}`, CHARIOX_RELAY_TOKEN: 'disposable-mdgrants', CHARIOX_HOME: home, CHARIOX_LOG_DIR: root + '/logs', CHARIOX_DAEMON_SOCKET: root + '/kernel.sock',
  CHARIOX_KERNEL_PORT: String(port), CHARIOX_MCP_PORT: String(mcpPort), CHARIOX_CODEX_PORT: String(await freePort()), CHARIOX_OPENCODE_PORT: String(await freePort()),
  CHARIOX_DAEMON_ID: 'mdgrants-drill', CHARIOX_DAEMON_ALIAS: 'mdgrants-drill', CHARIOX_DEV_STUB_FIRST_OUTPUT_DELAY_MS: '120000',
} });
const clients = [], results = [];
let control, sessionId, tui;
const tuiLines = [];
const sleep = ms => new Promise(r => setTimeout(r, ms));
async function until(work, label) { const end = Date.now() + 30000; let error; while (Date.now() < end) { try { const r = await work(); if (r) return r; } catch (e) { error = e; } await sleep(100); } throw new Error(label + ': ' + (error?.message ?? 'timeout')); }
const pass = label => { results.push(label); console.log('PASS ' + label); };
const send = async value => { const r = await control.send(value); if (r.Error) throw new Error(r.Error.message); return r; };
const grants = async command => (await send({ KernelBrowser: { command } })).KernelBrowser.result;
async function tool(run, name, args) {
  const r = await fetch(run.runtime_mcp_server_url, { method: 'POST', headers: { 'content-type': 'application/json', authorization: 'Bearer ' + run.runtime_mcp_auth_token }, body: JSON.stringify({ jsonrpc: '2.0', id: Date.now(), method: 'tools/call', params: { name, arguments: args } }) });
  const body = await r.text();
  // Some MCP transports use an SSE response. No token is included in evidence.
  return JSON.parse(body.includes('data:') ? body.split('\n').find(line => line.startsWith('data:')).slice(5).trim() : body);
}
try {
  await until(async () => { const s = net.createConnection(port, '127.0.0.1'); return new Promise(resolve => { s.on('connect', () => { s.destroy(); resolve(true); }); s.on('error', () => resolve(false)); }); }, 'kernel listener');
  control = new LocalIpcClient(`ws://127.0.0.1:${port}/kernel`, { localAuthEnvironment: { ...process.env, CHARIOX_HOME: home } }); clients.push(control);
  const presence = await until(async () => JSON.parse(await readFile(home + "/kernels/active/mdgrants-drill.json", "utf8")), "kernel protocol presence");
  assert.equal(presence.local_daemon_protocol_version, 443);
  const created = (await send(request.createSessionRequest(oss, oss, 'mdgrants', { provider: 'dev-stub', model: 'slow-first-output-drill' }))).SessionCreated;
  assert.ok(created); sessionId = created.session.id; const first = created.agent.id;
  const attachment = (await send(request.attachToSessionRequest(sessionId, 'mdgrants-drill'))).SessionAttached.attachment.id;
  const second = (await send(request.spawnAgentRequest(sessionId, 'dev-stub', 'second', 'slow-first-output-drill'))).AgentSpawned.agent.id;
  await send(request.focusAgentRequest(sessionId, first));
  const focused = await grants({ op: 'list_grants' });
  assert.ok(focused.grants.some(g => g.agent_id === first && g.focused)); pass('focus creates owner-visible grant');
  const launch = await send(request.launchProviderRunRequest(sessionId, 'dev-stub', 'default', 'slow-first-output-drill', 'low', first));
  const run = (launch.ProviderRunLaunchAccepted ?? launch.ProviderRunLaunched).provider_run;
  assert.ok(run.runtime_mcp_auth_token && run.runtime_mcp_server_url, 'provider-native runtime binding');
  const loaded = await tool(run, 'chariox.load_notes', {}); assert.ok(!loaded.error && !loaded.result?.isError, JSON.stringify(loaded));
  const selection = (await send({ Notes: { command: { op: 'report_selection', anchor: { window: { kind: 'panel', window_id: 'drill-panel' }, url: null, document_id: null, hint: 'drill', quote: { exact: 'ordinary drill selection', prefix: '', suffix: '' } }, box_css: null } } })).Notes.result.selection;
  const note = (await send({ Notes: { command: { op: 'create', selection_id: selection.selection_id, comment: 'ordinary drill note' } } })).Notes.result.note;
  const read = await tool(run, 'chariox.read_note', { note_id: note.note_id }); assert.ok(!read.error && !read.result?.isError, JSON.stringify(read));
  await send(request.submitPromptRequest(sessionId, attachment, first, 'Hold this turn during the access check.', []));
  await send(request.focusAgentRequest(sessionId, second));
  const retained = await until(async () => { const snapshot = await grants({ op: 'list_grants' }); const g = snapshot.grants.find(g => g.agent_id === first); return g && !g.focused && g.idle_since_ms === null && g.resources.some(r => r.kind === 'note' && r.note_id === note.note_id) ? snapshot : null; }, 'working retained holder');
  pass('grant survives focus switch during an active dev-stub turn, scoped to claimed note');
  const feed = new LocalIpcClient(`ws://127.0.0.1:${port}/kernel`, { localAuthEnvironment: { ...process.env, CHARIOX_HOME: home } }); clients.push(feed);
  const subscribed = feed.send({ KernelBrowser: { command: { op: 'subscribe_grants', after: retained.cursor, wait_ms: 25000 } } });
  const retainedRead = await tool(run, 'chariox.read_note', { note_id: note.note_id }); assert.ok(!retainedRead.error && !retainedRead.result?.isError, JSON.stringify(retainedRead));
  const notice = (await subscribed).KernelBrowser.result;
  assert.ok(notice.cursor > retained.cursor); assert.equal(notice.notice.agent_id, first); pass('non-focused use advances live cursor and publishes notice');
  tui = createAccessCommandController({ client: { localDaemonProtocolVersion: 443, send: value => control.send(value) }, appendNotice: line => tuiLines.push(line) });
  await tui.handle([]);
  assert.ok(tuiLines.some(line => line.includes(first) && line.includes(note.note_id)));
  await tui.handle(['revoke', first]);
  const revoked = await grants({ op: 'list_grants' });
  assert.ok(!revoked.grants.some(g => g.agent_id === first));
  const refused = await tool(run, 'chariox.read_note', { note_id: note.note_id });
  assert.ok(refused.error || refused.result?.isError || refused.result?.structuredContent?.ok === false, 'next MCP use must be refused');
  pass('revoke removes holder and refuses the next provider-bound use');
  await tui.handle(['revoke', 'all']);
  assert.equal((await grants({ op: 'list_grants' })).grants.length, 0); pass('revoke all clears owner grants');
  await writeFile(evidence + '/live-tui-access.txt', tuiLines.join('\n') + '\n', { mode: 0o600 });
  await writeFile(evidence + '/live-results.json', JSON.stringify({ protocol: 443, relay: 86, scope: 'Separate production kernel process, local owner WebSocket controls, real provider-bound MCP HTTP calls, dev-stub held turn and panel note. No official-provider or native-browser acceptance claim.', results, cleanup: 'Owned process group terminated; disposable state removed.' }, null, 2));
} finally {
  tui?.stop();
  if (control && sessionId) await control.send(request.teardownProviderProcessesRequest('dev-stub', true)).catch(() => {});
  for (const client of clients) client.destroy();
  try { process.kill(-kernel.pid, 'SIGTERM'); } catch {}
  await Promise.race([new Promise(r => kernel.once('exit', r)), sleep(2000)]);
  try { process.kill(-kernel.pid, 'SIGKILL'); } catch {}
  closeSync(log);
  await rm(root, { recursive: true, force: true });
}
