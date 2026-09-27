// Critical-action drill (P1.15): each `drill_step` occurrence runs one step of
// the kernel-owned validation lifecycle and logs its outcome to `app logs`.
//   request    ask for approval of send_greeting {message}
//   spend      spend the last approval: POST it to the declared route, once
//   change     ask again with the same operation id and another message
//   unapproved open the route with a fresh, undecided operation
const ROUTE = 'https://httpbin.org/post';

export default function register(chariox) {
  const log = (message, fields) => chariox.log.write('info', message, fields);
  const last = async () => (await chariox.state.get('last'))?.value ?? null;
  const remember = async (operationId, message) => {
    const record = await chariox.state.get('last');
    await chariox.state.transaction({ schemaVersion: 0, checks: [{ key: 'last', version: record?.version ?? null }],
      writes: [{ key: 'last', value: { operationId, message } }] });
  };
  const spend = async (operationId) => {
    try {
      const { streamId } = await chariox.http.open({ url: ROUTE, method: 'POST', hasBody: false,
        headers: [['accept', 'application/json']], operationId });
      for (let i = 0; i < 50; i += 1) {
        const headers = await chariox.http.headers(streamId);
        if (!headers.pending) return { status: headers.status };
        await new Promise((resolve) => setTimeout(resolve, 200));
      }
      return { status: 'no headers' };
    } catch (error) {
      return { error: error?.code ?? String(error) };
    }
  };

  chariox.events.register('drill_step', async ({ payload }) => {
    const message = payload.message ?? 'hello from the Chariox critical-action drill';
    if (payload.step === 'request') {
      const operation = await chariox.validation.request({ action: 'send_greeting', parameters: { message } });
      await remember(operation.operationId, message);
      await log('validation requested', { operationId: operation.operationId, state: operation.state });
    } else if (payload.step === 'spend') {
      const current = await last();
      const status = await chariox.validation.status(current.operationId);
      const first = await spend(current.operationId);
      const replay = await spend(current.operationId);
      await log('spend', { operationId: current.operationId, state: status.state, first, replay });
    } else if (payload.step === 'change') {
      const current = await last();
      try {
        const operation = await chariox.validation.request({ action: 'send_greeting',
          parameters: { message: `${current.message} (changed)` }, operationId: current.operationId });
        await log('changed request answered', { state: operation.state });
      } catch (error) {
        await log('changed request refused', { error: error?.code ?? String(error) });
      }
    } else if (payload.step === 'unapproved') {
      const operation = await chariox.validation.request({ action: 'send_greeting', parameters: { message: 'never approved' } });
      await log('unapproved spend', { operationId: operation.operationId, result: await spend(operation.operationId) });
    }
  });
}
