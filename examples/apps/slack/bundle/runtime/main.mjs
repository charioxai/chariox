// Slack reference App backend. Slack events reach it only through the
// generic path: the owner's Slack connection at the Slack event generator,
// an App inbox route per event type (`/app inbox add … --connection`), and
// this App's signed incoming events. It keeps the latest notifications in its
// own state and hands each one to the workflow the owner chose, through the
// ordinary App automation `notifications`. No Slack code runs in the kernel.

const KEY = 'notifications';
// The package's data schema version: 0 until it declares migrations.
const SCHEMA = 0;
// One App automation routes `notification` to the workflow the owner chose
// with `/app automation add <installation> notifications notification …`.
const AUTOMATION = 'notifications';
const KEEP = 100;
// One state value holds every kept notification and is capped at 256 KiB;
// the oldest are dropped to stay under this budget.
const MAX_VALUE_BYTES = 200 * 1024;
// A larger reply context is not kept; that notification cannot be answered.
const MAX_CONTEXT_BYTES = 4 * 1024;
// Without a configured (active) automation the kernel refuses the
// occurrence; the notification is still kept.
const OPTIONAL_OCCURRENCE = new Set(['NOT_FOUND', 'AUTOMATION_INACTIVE', 'OCCURRENCE_TOO_OLD']);
const KINDS = ['mentioned', 'channel_message', 'reaction_added'];

export default function register(chariox) {
  const fail = (code, message) => new chariox.AppError(code, message);

  async function load() {
    const record = await chariox.state.get(KEY);
    return { items: record?.value?.items ?? [], version: record?.version ?? null };
  }

  // Compare-and-set on the state version, so concurrent deliveries and tool
  // calls never overwrite each other.
  async function change(mutate, extra = () => ({})) {
    for (let attempt = 0; attempt < 5; attempt += 1) {
      const { items, version } = await load();
      const next = [...items];
      const result = mutate(next);
      if (result === undefined) return null;
      const kept = next.slice(0, KEEP);
      while (kept.length > 1 && bytes({ items: kept }) > MAX_VALUE_BYTES) kept.pop();
      try {
        await chariox.state.transaction({
          schemaVersion: SCHEMA,
          checks: [{ key: KEY, version }],
          writes: [{ key: KEY, value: { items: kept } }],
          ...extra(result),
        });
        return result;
      } catch (error) {
        if (error?.code !== 'CONFLICT') throw error;
      }
    }
    throw fail('CONFLICT', 'Notifications changed too often; try again');
  }

  const bytes = value => Buffer.byteLength(JSON.stringify(value) ?? '');
  // Cut at a code point, never inside a surrogate pair.
  const cut = (text, units) => {
    const head = text.slice(0, units);
    return /[\uD800-\uDBFF]$/.test(head) ? head.slice(0, -1) : head;
  };

  // What a person reads, from the Slack payload the generator forwarded.
  function notification(kind, occurrenceId, payload) {
    const event = payload.metadata?.event ?? {};
    const item = event.item ?? {};
    const text = kind === 'reaction_added'
      ? `:${event.reaction ?? 'reaction'}: on a message`
      : String(event.text ?? payload.text ?? '');
    return {
      id: occurrenceId,
      kind,
      text: cut(text, 4000),
      channel: String(event.channel ?? item.channel ?? payload.reply_context?.channel_id ?? '').slice(0, 64),
      user: String(event.user ?? '').slice(0, 64),
      occurred_at: payload.occurred_at,
      // Opaque to the App: the generator binds it to the connection, so a
      // later reply goes to the same conversation.
      reply_context: bytes(payload.reply_context ?? null) <= MAX_CONTEXT_BYTES ? payload.reply_context ?? null : null,
    };
  }

  function occurrence(item) {
    const occurredAtMs = Number.isFinite(Date.parse(item.occurred_at)) ? Date.parse(item.occurred_at) : Date.now();
    const where = item.channel ? ` in <#${item.channel}>` : '';
    const who = item.user ? ` from <@${item.user}>` : '';
    return {
      automationId: AUTOMATION,
      occurrenceId: chariox.events.occurrenceId(`slack:${item.id}`, occurredAtMs),
      eventVersion: 1,
      occurredAtMs,
      payload: {
        kind: item.kind, text: item.text, occurred_at: item.occurred_at,
        ...(item.channel ? { channel: item.channel } : {}), ...(item.user ? { user: item.user } : {}),
      },
      invocation: { prompt: `Slack ${item.kind.replace('_', ' ')}${where}${who}: ${item.text}`, artifacts: [] },
    };
  }

  // Deliveries are at least once; the occurrence identity keeps a redelivered
  // Slack event from being stored or forwarded twice.
  async function receive(kind, { occurrenceId, payload }) {
    const accept = forward => change(items => {
      if (items.some(item => item.id === occurrenceId)) return undefined;
      const item = notification(kind, occurrenceId, payload);
      items.unshift(item);
      return item;
    }, item => (forward ? { occurrences: [occurrence(item)] } : {}));
    try {
      await accept(true);
    } catch (error) {
      if (!OPTIONAL_OCCURRENCE.has(error?.code)) throw error;
      await accept(false);
    }
    return null;
  }
  for (const kind of KINDS) chariox.events.register(kind, event => receive(kind, event));

  chariox.tools.register('list_notifications', async ({ kind, limit = 20 } = {}) => {
    const { items } = await load();
    return {
      notifications: items
        .filter(item => !kind || item.kind === kind)
        .slice(0, limit)
        .map(({ reply_context: _context, ...item }) => item),
    };
  });

  chariox.tools.register('clear_notifications', async () => {
    const cleared = await change(items => {
      const count = items.length;
      items.length = 0;
      return { cleared: count };
    });
    return cleared ?? { cleared: 0 };
  });
}
