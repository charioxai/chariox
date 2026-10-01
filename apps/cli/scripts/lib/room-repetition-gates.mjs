// MP-08/MP-10: reject owned leaks; shared-host changes are retained separately.
import assert from 'node:assert/strict'
import {createHash} from 'node:crypto'
export function compareCycle(before, after) {
  const errors = []
  for (const key of ['containers', 'volumes', 'images', 'ownedPids', 'listeners']) {
    const extra = after[key].filter(value => !before[key].includes(value))
    if (extra.length) errors.push(`${key}: ${extra.join(', ')}`)
  }
  for (const key of ['kernel', 'relay']) {
    if (after[key].fds > before[key].fds + 8) errors.push(`${key} FD growth: ${after[key].fds - before[key].fds}`)
    if (after[key].rss > before[key].rss + 128 * 1024 ** 2) errors.push(`${key} RSS growth: ${after[key].rss - before[key].rss}`)
  }
  if (after.diskBytes > before.diskBytes + 64 * 1024 ** 2) errors.push(`owned disk growth: ${after.diskBytes - before.diskBytes}`)
  return errors
}
export function assertSameRoom(before, after) {
  for (const key of Object.keys(before)) assert.deepEqual(after[key], before[key], `MP-08: ${key} changed`)
}
export async function detachOwnedAttachment(send, request, attachmentId) {
  try { await send(request) } catch (error) {
    // A disconnected TUI's normal kernel cleanup may win this race.
    if (!String(error.message).includes(`attachment \`${attachmentId}\` was not found`)) throw error
  }
}

export function classifyOwnedListeners(listeners, historicalPorts, containers, runId, servicePids, observedOwnedPids=new Set(), foreignHostPids=new Set()) {
 const foreignPorts=new Map()
 for(const row of containers) {
  const [containerId,containerName,,published='']=row.split('|')
  if(containerName.includes(runId))continue
  for(const match of published.matchAll(/(?:127\.0\.0\.1|0\.0\.0\.0|\[::\]):(\d+)(?:-(\d+))?->/g)) {
   for(let port=Number(match[1]);port<=Number(match[2]??match[1]);port++)foreignPorts.set(port,{containerId,containerName})
  }
 }
 const owned=[],reused=[]
 for(const listener of listeners) {
  const port=Number(listener.trim().split(/\s+/)[3]?.match(/:(\d+)$/)?.[1])
  if(!historicalPorts.has(port))continue
  const listenerPids=[...listener.matchAll(/pid=(\d+),/g)].map(m=>Number(m[1]))
  const ownService=listenerPids.some(pid=>servicePids.includes(pid)||observedOwnedPids.has(pid))
  const foreign=foreignPorts.get(port)
  if(foreign&&!ownService)reused.push({listener,port,...foreign})
  else if(!ownService&&!listener.includes('docker-proxy')&&listenerPids.length&&listenerPids.every(pid=>foreignHostPids.has(pid)))reused.push({listener,port,foreignHostPids:listenerPids})
  else owned.push(listener)
 }
 return {owned,reused}
}

export function assertRestoredMachineIdentity(before, after, oldSliceId, newSliceId) {
 if(oldSliceId===newSliceId)assert.equal(after,before,'MP-08: same-slice machine identity changed')
 else assert.equal(after,createHash('sha256').update(`slice:${newSliceId}`).digest('hex').slice(0,32),'MP-10: replacement machine identity does not match new slice')
}

export function assertStableDecodedViews(samples, requireFrameChange=false) {
 assert.ok(samples.length>=2,'MP-08: stable display needs repeated samples')
 for(let viewer=0;viewer<samples[0].length;viewer++) {
  const first=samples[0][viewer]
  for(const row of samples) {
   const view=row[viewer]
   assert.ok(view?.connected&&view.ready&&view.canvasCount===1&&view.width>0&&view.height>0,`MP-08: viewer ${viewer} lost decoded display`)
   assert.equal(view.streamId,first.streamId,`MP-08: viewer ${viewer} stream changed during stability window`)
  }
  if(requireFrameChange)assert.ok(samples.some(row=>row[viewer].frameHash!==first.frameHash),`MP-10: viewer ${viewer} active browser frame did not advance`)
 }
}
