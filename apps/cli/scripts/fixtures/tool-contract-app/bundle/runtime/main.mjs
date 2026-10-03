// Tool-contract drill (V-SDK-03): each tool breaks the call contract in one way,
// so the kernel's answer to the caller can be observed from the App's view.
export default function register(chariox) {
  chariox.tools.register('echo', async ({ text }) => ({ text }));
  chariox.tools.register('bad_output', async () => ({ count: 'three' }));
  chariox.tools.register('refuse', async () => {
    throw new chariox.AppError('INVALID_ARGUMENT', 'refused on purpose');
  });
  // Logs whether the kernel cancelled the call (`app logs` shows it).
  chariox.tools.register('slow', async ({ ms }, { signal }) => {
    const started = Date.now();
    await new Promise((resolve) => {
      const timer = setTimeout(resolve, ms);
      signal.addEventListener('abort', () => { clearTimeout(timer); resolve(); }, { once: true });
    });
    const afterMs = Date.now() - started;
    if (signal.aborted) {
      await chariox.log.write('info', 'slow call cancelled', { ms, afterMs }).catch(() => {});
      throw new chariox.AppError('CANCELLED', 'cancelled by the caller');
    }
    return { waited: ms };
  });
}
