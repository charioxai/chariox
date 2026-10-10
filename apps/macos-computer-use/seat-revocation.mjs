// Runner boundary: observe a native effect before revoking an outstanding input.
// Neither a completed input nor an input that failed before revoke proves this.
export async function revokeDuringInput({ dispatch, observe, changed, revoke,
  wait = ms => new Promise(resolve => setTimeout(resolve, ms)), now = () => performance.now() }) {
  let settled = false;
  const input = dispatch().then(() => { settled = true; return { cancelled: false }; },
    error => { settled = true; return { cancelled: error?.code === "browser_action_cancelled", error }; });
  const deadline = now() + 2000;
  let observed = false;
  while (now() < deadline) {
    await wait(1);
    if (settled) throw new Error('input completed before revoke');
    if (changed(observe())) { observed = true; break; }
  }
  if (!observed) throw new Error('native mutation not observed during input');
  const started = now();
  await revoke();
  const ackMs = now() - started;
  const outcome = await input;
  if (!outcome.cancelled) throw new Error('input lacks explicit cancellation outcome');
  if (ackMs >= 2500) throw new Error('revoke acknowledgment exceeded lease');
  return { cancelled: true, revoke_ack_ms: ackMs };
}
