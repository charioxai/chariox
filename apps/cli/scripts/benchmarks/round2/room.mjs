// MP-08 / MP-10 / MP-11. One owned Room; product lifecycle and dynamic ports.
import { unwrap } from './kernel.mjs'

export class Round2Room {
  owned = { sessionId: null, slices: [], attachmentId: null, agentId: null, portRetries: 0 }

  constructor(api, { workspace, runId, checkpoint = async () => {} }) {
    if (!workspace || !/^[a-z0-9][a-z0-9_-]*$/i.test(runId)) throw new Error('MP-10 Room identity required')
    this.api = api; this.workspace = workspace; this.runId = runId; this.checkpoint = checkpoint
  }

  async setup({ viewport, verifySlice = async () => {}, guard = async () => {} } = {}) {
    const { client, requests: r } = this.api
    await guard()
    const session = unwrap(await client.send(r.createSessionRequest(this.workspace, this.workspace, this.runId)), 'SessionCreated').session
    this.owned.sessionId = session.id; await this.checkpoint(this.owned)
    for (let attempt = 0; attempt < 3; attempt++) {
      await guard()
      const name = `${this.runId}-${attempt}`
      const slice = unwrap(await client.send(r.createSliceRequest({ name, displayMode: 'headed', displayBackend: 'selkies', workspaceId: this.workspace, worktreeId: this.workspace, base: 'clean' })), 'SliceCreated').slice
      this.owned.slices.push({ id: slice.id, name, deleted: false }); await this.checkpoint(this.owned)
      await client.send(r.bindRoomEnvironmentSliceRequest(session.id, slice.id))
      try {
        await client.send(r.startSliceRequest(slice.id))
      } catch (error) {
        if (!/host port(?:\(s\)|s)?.*already in use/i.test(error.message) || attempt === 2) throw error
        await client.send(r.deleteSliceRequest(slice.id))
        this.owned.slices.at(-1).deleted = true; this.owned.portRetries++
        await this.checkpoint(this.owned); continue
      }
      // verifySlice should compare exact runtime/image/labels/limits and return
      // only public provenance; never export a raw Docker inspect/config.
      await verifySlice({ sessionId: session.id, sliceId: slice.id, name })
      await client.send(r.startRoomEnvironmentRequest(session.id, viewport ?? {
        css_width: 1280, css_height: 800, device_scale_factor: 1,
        desktop_pixel_width: 1280, desktop_pixel_height: 800,
      }))
      return { sessionId: session.id, sliceId: slice.id }
    }
  }

  async spawn({ provider, model, effort = 'high', accountProfile, sliceRef }) {
    const { client, requests: r } = this.api, sessionId = this.owned.sessionId
    const agent = unwrap(await client.send(r.spawnAgentRequest(sessionId, provider, `${this.runId}-agent`, model,
      this.workspace, effort, 'build', 'yolo', undefined, undefined, sliceRef, accountProfile)), 'AgentSpawned').agent
    this.owned.agentId = agent.id; await this.checkpoint(this.owned)
    const attachment = unwrap(await client.send(r.attachToSessionRequest(sessionId, this.runId)), 'SessionAttached').attachment
    this.owned.attachmentId = attachment.id; await this.checkpoint(this.owned)
    return agent.id
  }

  async submit(prompt) {
    const { client, requests: r } = this.api, o = this.owned
    const submitted = unwrap(await client.send(r.submitPromptRequest(o.sessionId, o.attachmentId, o.agentId, prompt, [])), 'PromptSubmitted')
    const promptId = (submitted.outcome?.Started ?? submitted.outcome?.Queued)?.prompt?.id
    if (!promptId) throw new Error('MP-10 submitted prompt identity missing; never replay')
    return { sessionId: o.sessionId, agentId: o.agentId, promptId }
  }

  async cancel() {
    const { client, requests: r } = this.api, o = this.owned
    await client.send(r.cancelActivePromptRequest(o.sessionId, o.attachmentId, o.agentId))
  }

  async cleanup() {
    const { client, requests: r } = this.api, o = this.owned
    const receipt = { sessionGone: !o.sessionId, attachmentDetached: !o.attachmentId, slices: [], errors: [] }
    const attempt = async (seam, action) => {
      try { await action(); return true } catch (error) { receipt.errors.push({ seam, errorClass: error.name, errorCode: error.code ?? null }); return false }
    }
    if (o.attachmentId) receipt.attachmentDetached = await attempt('attachment_detach', () => client.send(r.detachFromSessionRequest(o.attachmentId)))
    if (o.sessionId) {
      await attempt('room_delete', () => client.send(r.deleteSessionRequest(o.sessionId, this.workspace)))
      await attempt('room_absence', async () => {
        const response = unwrap(await client.send(r.listSessionsRequest()), 'SessionsListed')
        receipt.sessionGone = !response.sessions.some(session => session.id === o.sessionId)
      })
    }
    // Do not remove an execution environment while provider ownership remains.
    for (const slice of [...o.slices].reverse()) {
      if (!slice.name.startsWith(`${this.runId}-`)) throw new Error('MP-10 cleanup ownership mismatch')
      if (receipt.sessionGone && !slice.deleted) await attempt('slice_delete', () => client.send(r.deleteSliceRequest(slice.id)))
      let gone = false
      await attempt('slice_absence', async () => {
        const response = unwrap(await client.send(r.listSlicesRequest()), 'SlicesListed')
        gone = !response.slices.some(item => item.id === slice.id)
      })
      receipt.slices.push({ sliceId: slice.id, gone })
    }
    receipt.complete = receipt.sessionGone && receipt.attachmentDetached && receipt.slices.every(slice => slice.gone) && !receipt.errors.length
    return receipt
  }
}
