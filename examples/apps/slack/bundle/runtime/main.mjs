// Slack reference App backend. Slack events reach it only through the
// generic path: the owner's Slack connection at the Slack event generator,
// an App inbox route per event type (`/app inbox add … --connection`), and
// this App's signed incoming events. It keeps the latest notifications in its
// own state and hands each one to the workflow the owner chose, through the
// ordinary App automation `notifications`. Replies and reactions go back
// through the Slack connection the owner granted this App
// (`/app connection grant`), with the generator's reply context: the App never
// sees a Slack token. No Slack code runs in the kernel.

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
// A reply context is kept only while small and flat (the Slack generator's
// is a few ids); otherwise that notification cannot be answered. The state
// value's node and depth limits then hold too.
const MAX_CONTEXT_BYTES = 4 * 1024;
const MAX_CONTEXT_FIELDS = 16;
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

  function replyContext(value) {
    if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
    const fields = Object.values(value);
    const flat = fields.every(field => field === null || ['string', 'number', 'boolean'].includes(typeof field));
    return flat && fields.length <= MAX_CONTEXT_FIELDS && bytes(value) <= MAX_CONTEXT_BYTES ? value : null;
  }

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
      reply_context: replyContext(payload.reply_context),
      connection_id: payload.source?.connection_id ?? null,
      // Slack sends a mention in a channel both as a mention and as a channel
      // message: the same channel and timestamp name the same message.
      message: kind !== 'reaction_added' && event.channel && event.ts
        ? `${String(event.channel).slice(0, 64)}:${String(event.ts).slice(0, 32)}` : null,
    };
  }

  // Whether Slack says this App itself acted: its bot user (from the event's
  // authorizations) or its app id. Without that identity, any bot counts.
  function ownEvent(metadata) {
    // An edit or link unfurl names its author inside the changed message.
    const event = metadata?.event?.subtype === 'message_changed'
      ? metadata.event.message ?? {}
      : metadata?.event ?? {};
    const bots = (Array.isArray(metadata?.authorizations) ? metadata.authorizations : [])
      .filter(value => value?.is_bot && value.user_id).map(value => value.user_id);
    const app = metadata?.api_app_id;
    if (!bots.length && !app) return Boolean(event.bot_id) || event.subtype === 'bot_message';
    return bots.includes(event.user)
      || (Boolean(app) && (event.app_id === app || event.bot_profile?.app_id === app));
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
      // The other kind of a message already kept is not kept or forwarded
      // again: one message, one notification, one workflow run. A mention
      // arriving after its channel message marks it as a mention.
      // A redelivered mention after a merge matches the merged item too.
      const same = item.message ? items.findIndex(kept => kept.message === item.message) : -1;
      if (same >= 0) {
        if (kind !== 'mentioned' || items[same].kind === kind) return undefined;
        items[same] = { ...items[same], kind };
        return { merged: true };
      }
      items.unshift(item);
      return item;
    }, item => (forward && !item.merged ? { occurrences: [occurrence(item)] } : {}));
    // This App's own messages and reactions are kept but start no run: a
    // reply or reaction posted by an automation must not trigger it again.
    try {
      // Edits and unfurls (hidden message events) are not new messages.
      await accept(!payload.metadata?.event?.hidden && !ownEvent(payload.metadata));
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
        .map(({ reply_context: _context, connection_id: _connection, message: _message, ...item }) => ({
          ...item, can_reply: Boolean(_context && _connection),
        })),
    };
  });

  // The notification's own connection and reply context: a reply can only
  // go back to the conversation Slack delivered it from.
  async function target(id) {
    const { items } = await load();
    const item = items.find(entry => entry.id === id);
    if (!item) throw fail('NOT_FOUND', `No Slack notification ${id}`);
    if (!item.reply_context || !item.connection_id) {
      throw fail('INVALID_ARGUMENT', 'This notification cannot be answered in Slack');
    }
    return item;
  }

  // Without a request_id every call acts; with one, a retry of the same
  // request acts at most once (the generator keeps the first outcome).
  const key = (tool, requestId) => (requestId ? { idempotencyKey: `${tool}:${requestId}` } : {});

  chariox.tools.register('reply', async ({ id, text, mode = 'thread', request_id: requestId }) => {
    if (!text.trim()) throw fail('INVALID_ARGUMENT', 'A reply needs some text');
    const item = await target(id);
    const { accepted, result } = await chariox.connections.action({
      connectionId: item.connection_id,
      action: 'notification.reply',
      input: { text, mode },
      context: item.reply_context,
      ...key('reply', requestId),
    });
    return { posted: accepted, message_ts: result?.message_ts ?? null };
  });

  chariox.tools.register('react', async ({ id, name, request_id: requestId }) => {
    const item = await target(id);
    const { accepted } = await chariox.connections.action({
      connectionId: item.connection_id,
      action: 'slack.reaction.add',
      input: { name },
      context: item.reply_context,
      ...key('react', requestId),
    });
    return { reacted: accepted };
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
