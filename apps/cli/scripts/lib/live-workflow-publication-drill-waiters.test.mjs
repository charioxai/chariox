import assert from 'node:assert/strict'
import test from 'node:test'

import { waitForScheduledWorkflowRun } from './live-workflow-publication-drill-waiters.mjs'

test('scheduled publication waiter returns a terminal run with final output', async () => {
  const workflowRun = {
    id: 'run-1',
    status: 'completed',
    final_output: { message: { value: 1842 } },
  }
  let requestCount = 0
  const client = {
    async send() {
      requestCount += 1
      if (requestCount === 1) {
        return { workflow_runs: [{ id: workflowRun.id, status: 'running' }] }
      }
      return { workflow_run: workflowRun }
    },
  }

  const result = await waitForScheduledWorkflowRun(client, 'session-1', 'workflow-1', {
    requireOutput: true,
    timeoutMs: 100,
  })

  assert.deepEqual(result, workflowRun)
  assert.equal(requestCount, 2)
})
