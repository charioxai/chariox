// MP-08 / MP-10 / MP-11: official turn.complete receipt, no prompt dispatch.
export function register(on, options = {}) {
  const context = options.context ?? "__CHARIOX_CONTEXT__"
  let bound
  on('turn.start', async ($, e, next) => {
    bound = undefined
    try {
      const prompt = (await $.fs.read(context + '.approval-origin')).trim()
      const batch = JSON.parse(await $.fs.read(context + '.hook-batch.json'))
      if (prompt && prompt.length <= 256 && /^[a-zA-Z0-9_-]{1,128}$/.test(batch.id))
        bound = {turn_id:e.turnId, prompt_id:prompt, hook_context_request_id:batch.id}
    } catch {}
    return next(e)
  })
  on('turn.complete', async ($, e, next) => {
    if (!e.agentId && bound?.turn_id === e.turnId && e.reason === 'aborted' && e.isAborted === true) {
      await $.fs.write(context + '.native-abort.json', JSON.stringify({...bound, reason:'aborted', at_ms:Date.now()}))
      bound = undefined
    }
    return next(e)
  })
}
