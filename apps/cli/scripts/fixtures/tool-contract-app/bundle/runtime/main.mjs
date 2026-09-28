// Tool-contract drill (V-SDK-03): each tool breaks the call contract in one way,
// so the kernel's answer to the caller can be observed from the App's view.
export default function register(chariox) {
  chariox.tools.register('echo', async ({ text }) => ({ text }));
  chariox.tools.register('bad_output', async () => ({ count: 'three' }));
  chariox.tools.register('refuse', async () => {
    throw new chariox.AppError('INVALID_ARGUMENT', 'refused on purpose');
  });
  chariox.tools.register('slow', async ({ ms }) => {
    await new Promise((resolve) => setTimeout(resolve, ms));
    return { waited: ms };
  });
}
