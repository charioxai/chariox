// Deliveries reach the App through the kernel's inbox routes, which the subscriptions create (src/subscriptions).
// The signed inbox schema has already checked a delivery's shape; this checks it is the event its route names.
import { incoming } from './event-types.mjs';

const fail = (code, message) => { throw { code, message, retryable: false }; };

/** The pull request a delivery is about, for the window: repository and number (absent for events that name none). */
const subject = payload => ({ repo: payload.metadata?.repository?.full_name ?? null, number: payload.metadata?.pull_request?.number ?? payload.metadata?.issue?.number ?? null });

export function createEventsApp({ subscriptions, changed = async () => {} }) {
  return {
    events: Object.fromEntries(Object.entries(incoming).map(([name, type]) => [name, async (payload, ctx) => {
      const time = Date.parse(payload.occurred_at);
      if (payload.source.generator_id !== 'dev.chariox.github' || payload.source.event_type !== type
        || payload.source.event_type_version !== 1 || !Number.isSafeInteger(time) || time < 0) {
        fail('INVALID_ARGUMENT', 'Use a GitHub delivery with its original event type and occurrence time');
      }
      // An open window follows this change and reloads what the event is about.
      await changed({ kind: 'event', ...subject(payload) }).catch(() => {});
      await subscriptions.deliver(type, payload, ctx);
      return null;
    }])),
  };
}
