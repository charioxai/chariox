// MP-08 / MP-10: lane-owned fault operations and safe client projections.
// Process spawning remains with the existing drill, which owns cleanup.
import assert from 'node:assert/strict'
import { automationNoticeEntries } from './room-tui-notices.mjs'

export function roomWebFaultHandlers(runtime) {
  const { docker, containerName, requests } = runtime
  const local = () => runtime.getLocalAutomation()
  const remote = () => runtime.getRemoteAutomation()
  let controllerEventCursor = 0
  const projection = async automation => {
    const value = await automation.send('snapshot')
    return { screen: value.screen, daemonDisconnected: value.daemonDisconnected,
      statusLine: value.statusLine, sessionId: value.session?.id,
      notices: automationNoticeEntries(value).filter(item => /^Room |screen|disconnect|reconnect|error/i.test(item.text)).slice(-18) }
  }
  const response = async (request, variant) => {
    const value = await runtime.getClient().send(request)
    assert.ok(value && Object.hasOwn(value, variant), `expected ${variant}`)
    return value[variant]
  }
  return {
    relay_stop: async () => { await runtime.terminateChild(runtime.getRelay()); return { stopped: true } },
    relay_start: () => runtime.startRelay(),
    relay_queue: async () => { await runtime.terminateChild(runtime.getRelay()); return runtime.startRelay(2) },
    kernel_stop: async () => { await runtime.stopKernel(); return { stopped: true } },
    kernel_start: () => runtime.restartKernel(),
    streamer_stop: async () => {
      await docker(['exec', '-u', 'slice', containerName, '/opt/chariox-selkies/bin/python', '/opt/chariox-slice/slice-selkies.py', 'stop', '--allow-forced'])
      return { stopped: true }
    },
    streamer_start: async () => {
      // Restore only the injected stream seam; desktop start may open another Tab.
      await docker(['exec', '-u', 'slice', containerName, '/opt/chariox-selkies/bin/python', '/opt/chariox-slice/slice-selkies.py', 'start'])
      return { restarted: true }
    },
    controller_stop: async () => {
      const before = await response(requests.getRoomEnvironmentStateRequest(runtime.getSessionId()), 'RoomEnvironmentState')
      controllerEventCursor = before.environment.event_cursor
      const value = await docker(['exec', containerName, 'python3', '-c', "import os,signal; matches=[int(p) for p in os.listdir('/proc') if p.isdigit() and int(p)!=os.getpid() and os.path.exists('/proc/'+p+'/cmdline') and b'/opt/chariox-slice/browser-controller.mjs' in open('/proc/'+p+'/cmdline','rb').read().split(b'\\0')]; assert len(matches)==1, len(matches); os.kill(matches[0],signal.SIGKILL); print('controller_stopped')"])
      return { stopped: value.stdout.trim() === 'controller_stopped' }
    },
    controller_events: async () => response(requests.getRoomEnvironmentEventsRequest(runtime.getSessionId(), controllerEventCursor), 'RoomEnvironmentEvents'),
    clients: async () => ({ local: await projection(local()), remote: await projection(remote()) }),
    room_status: async () => {
      await Promise.all([local().send('submit_prompt', { prompt: '/room status' }), remote().send('submit_prompt', { prompt: '/room status' })])
      return { submitted: true }
    },
    state: async () => ({
      environment: (await response(requests.getRoomEnvironmentStateRequest(runtime.getSessionId()), 'RoomEnvironmentState')).environment,
      page: (await response(requests.listRoomEnvironmentActionHistoryRequest(runtime.getSessionId(), undefined, 100), 'RoomEnvironmentActionHistoryListed')).page,
    }),
  }
}
