// MP-08/MP-10: completed history may precede the final provider output projection.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { runPromptAttachments } from './wp07p-prompt-attachments.mjs';
test('MP-08/MP-10 attachment verifier waits for the completed turn reply', async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'wp07p-attachment-check-'));
  const previous = process.env.WP07_ATTACHMENTS_CLIENTS;
  process.env.WP07_ATTACHMENTS_CLIENTS = 'localTui';
  let submitted = false, reads = 0;
  const requests = { focusAgentRequest: () => 'focus', getSessionHistoryOutlineRequest: () => 'history' };
  const client = { async send(request) {
    if (request === 'focus') return {};
    if (!submitted) return { SessionHistoryOutline: { agents: [{ agent_id: 'agent', turns: [] }] } };
    reads++;
    const entries = reads > 1 ? [{ entry: { kind: 'provider_output', text: (await readFile(path.join(root, 'wp07p-localTui-attachment.txt'), 'utf8')).trim() } }] : [];
    return { SessionHistoryOutline: { agents: [{ agent_id: 'agent', turns: [{ prompt_id: 'prompt', lifecycle: 'completed', entries: [] }, { prompt_id: 'prompt', lifecycle: 'completed', entries }] }] } };
  } };
  const terminal = { async send(action) { if (action === 'snapshot') return { session: { focusedAgentId: 'agent' } }; submitted = true; return {}; } };
  const waitFor = async callback => { for (let i = 0; i < 4; i++) { const result = await callback(); if (result) return result; } throw new Error('fixture never reached ready output'); };
  try {
    const result = await runPromptAttachments({ client, requests, sessionId: 'session', provider: { agentId: 'agent' }, options: { provider: 'opencode' }, fixtureWorkspace: root, publicEvidenceRoot: root, localAutomation: terminal, remoteAutomation: null, web: null, waitFor });
    assert.equal(result.clients[0].status, 'PASS');
  } finally {
    if (previous === undefined) delete process.env.WP07_ATTACHMENTS_CLIENTS; else process.env.WP07_ATTACHMENTS_CLIENTS = previous;
    await rm(root, { recursive: true });
  }
});
