import assert from 'node:assert/strict'
import test from 'node:test'

import { providerHistoryTextForPrompt } from './live-relay-freeform-multi-user-history.mjs'

const requests = {
  getSessionHistoryOutlineRequest: (sessionId, agentIds) => ({ outline: { sessionId, agentIds } }),
  getSessionHistoryBlobContentRequest: (sessionId, agentId, blobId) => ({ blob: { sessionId, agentId, blobId } }),
}

function pageEntry(entryIndex, kind, text) {
  return { entry_index: entryIndex, entry: { kind, text } }
}

test('provider history ignores markers present only in prompts or another agent and prompt', async () => {
  const client = {
    async send(request) {
      assert.ok(request.outline)
      return {
        SessionHistoryOutline: {
          agents: [
            {
              agent_id: 'other-agent',
              turns: [{
                prompt_id: 'target-prompt',
                user_prompt: pageEntry(1, 'user_prompt', 'MARKER'),
                entries: [pageEntry(2, 'provider_output', 'MARKER')],
                blobs: [],
              }],
            },
            {
              agent_id: 'target-agent',
              turns: [
                {
                  prompt_id: 'other-prompt',
                  user_prompt: pageEntry(3, 'user_prompt', 'MARKER'),
                  entries: [pageEntry(4, 'provider_output', 'MARKER')],
                  blobs: [],
                },
                {
                  prompt_id: 'target-prompt',
                  user_prompt: pageEntry(5, 'user_prompt', 'Reply with MARKER'),
                  entries: [pageEntry(6, 'provider_reasoning', 'MARKER')],
                  blobs: [],
                },
              ],
            },
          ],
        },
      }
    },
  }

  assert.equal(
    await providerHistoryTextForPrompt(client, requests, 'session-1', 'target-agent', 'target-prompt'),
    '',
  )
})

test('provider history reconstructs fragmented output across inline and blob entries', async () => {
  const client = {
    async send(request) {
      if (request.outline) {
        return {
          SessionHistoryOutline: {
            agents: [{
              agent_id: 'target-agent',
              turns: [{
                prompt_id: 'target-prompt',
                user_prompt: pageEntry(1, 'user_prompt', 'Reply with MARKER'),
                entries: [pageEntry(2, 'provider_output', 'MAR')],
                blobs: [{ blob_id: 'blob-1', sequence_start: 3 }],
                summary: pageEntry(4, 'provider_output', 'ER'),
              }],
            }],
          },
        }
      }
      assert.deepEqual(request.blob, {
        sessionId: 'session-1',
        agentId: 'target-agent',
        blobId: 'blob-1',
      })
      return {
        SessionHistoryBlobContent: {
          entries: [
            pageEntry(3, 'provider_output', 'K'),
            pageEntry(3, 'provider_status', 'ignored'),
          ],
        },
      }
    },
  }

  assert.equal(
    await providerHistoryTextForPrompt(client, requests, 'session-1', 'target-agent', 'target-prompt'),
    'MARKER',
  )
})
