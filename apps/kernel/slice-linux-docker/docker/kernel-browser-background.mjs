// MP-08/MP-10/MP-11: focus emulation exposes visibility but does not keep a hidden
// RenderWidget scheduling frames. A tiny capture holds it awake. Its pixels are
// discarded here; the viewer and agent screenshots retain their protected paths.
export async function captureBackgroundRenderer(connection, targetId) {
  const { sessionId } = await connection.send('Target.attachToTarget', { targetId, flatten: true });
  let closed = false;
  const off = connection.subscribe(message => {
    if (closed || message.method !== 'Page.screencastFrame' || message.sessionId !== sessionId) return;
    const frame = message.params?.sessionId;
    if (Number.isSafeInteger(frame)) void connection.send('Page.screencastFrameAck', { sessionId: frame }, sessionId).catch(() => {});
  });
  const close = async () => {
    if (closed) return;
    closed = true; off();
    try { await connection.send('Page.stopScreencast', {}, sessionId); }
    finally { await connection.send('Target.detachFromTarget', { sessionId }); }
  };
  try {
    await connection.send('Page.startScreencast', { format: 'jpeg', quality: 1, maxWidth: 16, maxHeight: 16, everyNthFrame: 1 }, sessionId);
    return close;
  } catch (error) {
    await close().catch(() => {});
    throw error;
  }
}
