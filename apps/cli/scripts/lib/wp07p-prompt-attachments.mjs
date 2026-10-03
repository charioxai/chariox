// MP-08/MP-10: real terminal attachment paths; only lane-owned fixture bytes.
import assert from 'node:assert/strict';
import { randomUUID, createHash } from 'node:crypto';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
const mp_items = ['MP-08', 'MP-10'];
const unwrap = (response, key) => { assert.ok(response?.[key], key); return response[key]; };
export async function runPromptAttachments(input) {
  const { client, requests, sessionId, provider, fixtureWorkspace, publicEvidenceRoot,
    localAutomation, remoteAutomation, web, waitFor } = input;
  const result = { mp_items, provider: input.options.provider, clients: [] };
  await client.send(requests.focusAgentRequest(sessionId, provider.agentId));
  for (const [name, terminal] of [['localTui', localAutomation], ['remoteTui', remoteAutomation], ['Web', web]]) {
    if (process.env.WP07_ATTACHMENTS_CLIENTS && !process.env.WP07_ATTACHMENTS_CLIENTS.split(',').includes(name)) continue;
    if (!terminal) { result.clients.push({ client: name, status: 'blocked', reason: 'Web observer unavailable' }); continue; }
    const marker = `WP07P_ATTACHMENT_${randomUUID()}`;
    const bytes = Buffer.from(marker + '\n');
    const filename = name === 'Web' ? 'wp07p-web-attachment.txt' : `wp07p-${name}-attachment.txt`;
    const file = path.join(fixtureWorkspace, filename);
    await writeFile(file, bytes, { mode: 0o600 });
    const row = { client: name, status: 'RED', filename, sizeBytes: bytes.length,
      sha256: createHash('sha256').update(bytes).digest('hex') };
    try {
      const before = unwrap(await client.send(requests.getSessionHistoryOutlineRequest(sessionId, [provider.agentId], 4)), 'SessionHistoryOutline');
      const old = new Set(before.agents?.find(a => a.agent_id === provider.agentId)?.turns?.map(t => t.prompt_id) ?? []);
      if (name === 'Web') await web.observe('web-attachment');
      else {
        await waitFor(async () => (await terminal.send('snapshot')).session?.focusedAgentId === provider.agentId,
          15000, 'MP-08/MP-10 attachment TUI focus');
        await terminal.send('submit_prompt', { prompt: 'Read the attached text file. Reply exactly its single line. No other work.',
          attachments: [{ url: pathToFileURL(file).href, mime: 'text/plain', filename }] }, 20000);
      }
      const turn = await waitFor(async () => {
        const history = unwrap(await client.send(requests.getSessionHistoryOutlineRequest(sessionId, [provider.agentId], 5)), 'SessionHistoryOutline');
        const candidates = history.agents?.find(a => a.agent_id === provider.agentId)?.turns?.filter(t => !old.has(t.prompt_id) && t.lifecycle === 'completed') ?? [];
        for (const candidate of candidates) {
          const entries = [...(candidate.entries ?? []), ...(candidate.summary ? [candidate.summary] : [])];
          for (const blob of candidate.blobs ?? []) {
            if (blob.total_chars > 250000) continue;
            const content = unwrap(await client.send(requests.getSessionHistoryBlobContentRequest(sessionId, provider.agentId, blob.blob_id)), 'SessionHistoryBlobContent');
            entries.push(...(content.entries ?? []));
          }
          if (entries.some(i => i.entry?.kind === 'provider_output' && i.entry.text.includes(marker))) return candidate;
        }
        return false;
      }, 90000, 'MP-08/MP-10 completed attachment turn did not expose its exact fixture reply');
      row.promptId = turn.prompt_id;
      row.status = 'PASS'; row.source = name === 'Web' ? 'production terminal file input' : 'actual TUI submit_prompt attachment path';
    } catch (error) { row.reason = error.message.split('; last snapshot')[0]; }
    result.clients.push(row);
    await writeFile(path.join(publicEvidenceRoot, 'prompt-attachments.json'), JSON.stringify(result, null, 2) + '\n', { mode: 0o600 });
  }
  return result;
}
