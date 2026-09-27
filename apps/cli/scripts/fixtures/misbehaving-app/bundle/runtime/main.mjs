// Runtime-limit drill (V-RUN-01/02/03/05/08): each `misbehave` occurrence makes
// the worker misbehave in one way, so the kernel's limits can be observed.
//   ok     log and acknowledge (control)
//   spin   a synchronous infinite loop (CPU)
//   grow   keep allocating memory
//   flood  write log entries far past the rate limit
//   hang   never acknowledge the event
//   crash  throw outside any handler
export default function register(chariox) {
  chariox.events.register('misbehave', async ({ payload }) => {
    if (payload.mode === 'ok') {
      await chariox.log.write('info', 'ok');
    } else if (payload.mode === 'spin') {
      for (;;) {}
    } else if (payload.mode === 'grow') {
      const held = [];
      for (;;) {
        held.push(Buffer.alloc(16 * 1024 * 1024, 1));
        await new Promise((resolve) => setTimeout(resolve, 5));
      }
    } else if (payload.mode === 'flood') {
      const writes = [];
      for (let i = 0; i < 5000; i += 1) {
        writes.push(chariox.log.write('info', `flood ${i}`, { i }).catch((error) => error?.code ?? 'failed'));
      }
      const results = await Promise.all(writes);
      const refused = results.filter((value) => value !== undefined).length;
      await chariox.log.write('warn', 'flood finished', { attempted: 5000, refused });
    } else if (payload.mode === 'hang') {
      await new Promise(() => {});
    } else if (payload.mode === 'crash') {
      setTimeout(() => { throw new Error('drill crash'); }, 0);
    }
  });
}
