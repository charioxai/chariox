// Lost-response drill (V1-INT-08): each `drill_step` occurrence runs one
// generator action through the owner-granted dummy connection and logs what
// the kernel answered to `app logs`. Run it against a generator that acts and
// then drops the connection: the App must learn the outcome is unknown, and
// the kernel must never send the action again.
//   keyless  one call without an idempotency key (a retry would act again)
//   keyed    one call with a fixed idempotency key (a retry acts once)
export default function register(chariox) {
  const log = (message, fields) => chariox.log.write('info', message, fields);

  chariox.events.register('drill_step', async ({ occurrenceId, payload }) => {
    const { connections } = await chariox.connections.list();
    const connection = connections.find((granted) => granted.generatorId === 'dev.chariox.dummy');
    if (!connection) return log('no dummy connection granted', {});
    const request = { connectionId: connection.connectionId, action: 'drill.echo', input: { occurrenceId } };
    if (payload.step === 'keyed') request.idempotencyKey = `drill-${occurrenceId}`;
    try {
      const result = await chariox.connections.action(request);
      await log('action answered', { step: payload.step, accepted: result.accepted });
    } catch (error) {
      await log('action failed', { step: payload.step, code: error?.code ?? 'unknown', retryable: error?.retryable === true });
    }
  });
}
