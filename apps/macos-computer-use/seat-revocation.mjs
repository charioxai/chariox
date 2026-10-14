// Runner boundary: observe a native effect before revoking an outstanding input.
// Neither a completed input nor an input that failed before revoke proves this;
// `cancelled` must prove the refused input's own Action was cancelled.
export async function revokeDuringInput({ dispatch, observe, changed, revoke, cancelled,
  wait = ms => new Promise(resolve => setTimeout(resolve, ms)), now = () => performance.now() }) {
  let settled = false;
  const input = dispatch().then(() => { settled = true; return null; },
    error => { settled = true; return error ?? new Error('input refused'); });
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
  const refusal = await input;
  if (!refusal || !(await cancelled(refusal))) throw new Error('input lacks explicit cancellation outcome');
  if (ackMs >= 2500) throw new Error('revoke acknowledgment exceeded lease');
  return { cancelled: true, revoke_ack_ms: ackMs };
}

// The paired seat posts session events and cannot observe app completion, so
// only this exact receipt is valid; the fixture's own log proves the effect.
const PHYSICAL_CLICK_RECEIPT = 'dispatched; path=CGEventSession; application completion unproven';
export function requirePhysicalClick(reply) {
  if (reply?.dispatched !== true || reply.receipts?.length !== 1 || reply.receipts[0] !== PHYSICAL_CLICK_RECEIPT)
    throw new Error('unexpected paired click receipt');
}
