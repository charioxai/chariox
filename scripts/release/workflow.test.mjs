import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

test('MP-08 / MP-09 / MP-10 / MP-11: release notification is manual and main-only', async () => {
  const workflow = await readFile(new URL('../../.github/workflows/release-main.yml', import.meta.url), 'utf8')
  assert.match(workflow, /^on:\n  workflow_dispatch:\s*\n\npermissions:/m)
  assert.match(workflow, /if:.*github\.ref == 'refs\/heads\/main'/)
  assert.match(workflow, /actions\/workflows\/release-staging\.yml\/dispatches/)
  assert.doesNotMatch(workflow, /repository_dispatch|client_payload|head_commit/)
})
