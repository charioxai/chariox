// MP-08 / MP-09 / MP-10 / MP-11: source deliveries have already passed the
// kernel's signed inbox schema. Targets are named App automations, never
// provider credentials or workflow ids in the handler.
export const eventTypes = {
  pull_request_opened: 'pull_request.opened',
  pull_request_updated: 'pull_request.synchronize',
  review_requested: 'pull_request.review_requested',
  issue_opened: 'issues.opened',
  workflow_run_completed: 'workflow_run.completed',
};
const SETTINGS = 'automations';
const outgoing = new Set(Object.keys(eventTypes).map(name => `${name}_notification`));
const fail = (code, message) => { throw { code, message, retryable: false }; };

export function createApp({ state, events }) {
  async function settings() {
    const record = await state.get(SETTINGS);
    return { record, automations: record ? JSON.parse(record.value) : {} };
  }

  return {
    tools: {
      async get_automations() {
        return { automations: (await settings()).automations };
      },
      async configure_automations({ automations }) {
        if (!automations || typeof automations !== 'object' || Array.isArray(automations)) {
          fail('INVALID_ARGUMENT', 'Provide an outgoing event to automation names map');
        }
        for (const [event, names] of Object.entries(automations)) {
          if (!outgoing.has(event) || !Array.isArray(names)
            || names.some(name => typeof name !== 'string' || !name || new TextEncoder().encode(name).length > 128 || /[\s\p{Cc}\uFEFF]/u.test(name))
            || new Set(names).size !== names.length) {
            fail('INVALID_ARGUMENT', 'Use declared outgoing events and unique automation names');
          }
        }
        const { record } = await settings();
        await state.transact({
          schemaVersion: 0, checks: [{ key: SETTINGS, version: record?.version ?? null }],
          writes: [{ tag: 'put', val: [SETTINGS, JSON.stringify(automations)] }],
          occurrences: [], wakes: [],
        });
        return { automations };
      },
    },
    events: Object.fromEntries(Object.entries(eventTypes).map(([name, type]) => [name, async (payload, ctx) => {
      const time = Date.parse(payload.occurred_at);
      if (payload.source.generator_id !== 'dev.chariox.github'
        || payload.source.event_type !== type || payload.source.event_type_version !== 1
        || !Number.isSafeInteger(time) || time < 0) {
        fail('INVALID_ARGUMENT', 'Use a GitHub delivery with its original event type and occurrence time');
      }
      const { automations } = await settings();
      const event = `${name}_notification`;
      if (!Object.hasOwn(automations, event)) {
        fail('NOT_CONFIGURED', `Configure automation names for ${event} before enabling its inbox route`);
      }
      const occurredAtMs = BigInt(time);
      const occurrenceId = events.occurrenceId(`github:${name}:${ctx.occurrenceId}`, occurredAtMs);
      // One emit per target avoids the transaction batch cap. A target that
      // cannot take the occurrence (a removed agent or a mistyped name) must not
      // keep the others from running, so every name is tried before the first
      // failure is raised. The inbox retries with the same identities and the
      // kernel dedupes per target; after the last attempt the occurrence
      // settles as failed in the route's counts.
      let failure;
      for (const automationId of automations[event]) {
        ctx.signal?.throwIfAborted();
        try {
          await events.emit({
            automationId, occurrenceId, eventVersion: 1, occurredAtMs, scheduleRevision: null,
            payload: JSON.stringify(payload),
            invocation: { prompt: payload.text, artifacts: [] },
          });
        } catch (error) {
          failure ??= error;
        }
      }
      if (failure) throw failure;
      return null;
    }])),
  };
}
