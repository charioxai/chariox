// MP-08/MP-10 (plan 1.1): protocol 491 ACK gate under a stalled client.
// The viewer keeps receiving and presenting but stops acknowledging while
// the canvas fixture animates (its click toggles motion, as in the workload); the kernel pump must stop sending
// (Selkies backpressure, reprobe keys only), then resume on the first
// acknowledgements without a resubscribe or decode failure.
// control=true keeps acknowledging (no stall): the drill must then fail,
// proving it detects a pump that ignores the client.
export async function measureAckGate({ page, pause, stallMs = 6000, control = false }) {
  const result = await page.evaluate(async ({ stallMs, control }) => {
    if (!mdStream.push) return { skipped: 'credit-mode stream (no ACK gate)' };
    const stamp = () => performance.timeOrigin + performance.now(), request = mdTransport.request;
    const sent = { acks: 0, held: 0 }, held = [];
    mdTransport.request = async value => {
      // A stalled link delays acknowledgements; they arrive when it recovers.
      if (value?.KernelBrowser?.command?.op === 'display_ack' && window.mdAckStalled && !control) { sent.held++; return new Promise(resolve => held.push(() => resolve(request(value)))); }
      if (value?.KernelBrowser?.command?.op === 'display_ack') sent.acks++;
      return request(value);
    };
    const wait = ms => new Promise(r => setTimeout(r, ms));
    await mdStream.input({ kind: 'click', x: 60, y: 88 });
    const before = stamp(); await wait(1000);
    const normal = mdFrames.filter(f => f.arrived_ms >= before).length;
    window.mdAckStalled = true; const stalled = stamp(); await wait(stallMs);
    window.mdAckStalled = false; const lifted = stamp(); for (const release of held.splice(0)) release();
    const during = mdFrames.filter(f => f.arrived_ms >= stalled + 500 && f.arrived_ms < lifted);
    await wait(1500);
    await mdStream.input({ kind: 'click', x: 60, y: 88 });
    mdTransport.request = request;
    const after = mdFrames.filter(f => f.arrived_ms >= lifted);
    const resumed = mdPresentations.find(p => p.drawn_ms >= lifted);
    return { stall_ms: stallMs, normal_frames_per_s: normal, stalled_frames: during.length, stalled_keys: during.filter(f => f.key).length,
      resumed_ms: resumed ? resumed.drawn_ms - lifted : null, frames_after_lift: after.length, acks_held: sent.held, acks_sent: sent.acks,
      stream_error: mdStream.error ? String(mdStream.error.message) : null };
  }, { stallMs, control });
  if (result.skipped) return result;
  // Without the gate a 6 s stall would carry every source frame (~30/s).
  result.passed = result.stream_error === null && result.normal_frames_per_s >= 10 && result.stalled_frames <= Math.max(6, result.stall_ms / 1000 * 2) && result.resumed_ms !== null && result.resumed_ms <= 500 && result.frames_after_lift >= 5;
  if (!result.passed) throw Error('MP-10: ACK gate drill ' + JSON.stringify(result));
  await pause(500);
  return result;
}
