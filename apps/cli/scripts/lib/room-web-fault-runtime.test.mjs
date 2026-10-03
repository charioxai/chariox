// MP-08 / MP-10: stream recovery must preserve the running desktop and its Tabs.
import assert from 'node:assert/strict'
import test from 'node:test'
import { roomWebFaultHandlers } from './room-web-fault-runtime.mjs'
test('display recovery starts only the Selkies stream seam', async () => {
  const calls = []
  const handlers = roomWebFaultHandlers({
    containerName: 'owned-slice', requests: {},
    docker: async args => { calls.push(args); return { stdout: '' } },
    sliceScreen: async () => { throw new Error('desktop restart would replace Tabs') },
  })
  await handlers.streamer_start()
  assert.deepEqual(calls, [['exec', '-u', 'slice', 'owned-slice', '/opt/chariox-selkies/bin/python', '/opt/chariox-slice/slice-selkies.py', 'start']])
})
