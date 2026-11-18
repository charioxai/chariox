// Slack reference App backend. Slack events reach it only through the generic path: the owner's Slack
// connection at the Slack event generator, an App inbox route per event type (`/app inbox add … --connection`),
// and this App's signed incoming events. It keeps the latest notifications in its own state and hands each one
// to the workflow the owner chose, through the ordinary App automation `notifications`. Replies and reactions go
// back through the Slack connection the owner granted this App (`/app connection grant`), with the generator's
// reply context: the App never sees a Slack token. No Slack code runs in the kernel.
// The imports are injected so the same handlers run under Node tests and as the wasm component (app.ts).

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

const fail = (code, message) => { throw { code, message, retryable: false }; };
// A refusal from a host import arrives wrapped, one the App raises itself does not.
const codeOf = error => (error?.payload ?? error)?.code;
const bytes = value => new TextEncoder().encode(JSON.stringify(value) ?? '').length;
// Cut at a code point, never inside a surrogate pair.
const cut = (text, units) => {
  const head = text.slice(0, units);
  return /[\uD800-\uDBFF]$/.test(head) ? head.slice(0, -1) : head;
};

export function createApp({ state, events, connections, feed }) {
  async function load() {
    const record = await state.get(KEY);
    return { items: record ? JSON.parse(record.value).items : [], version: record?.version ?? null };
  }

  // Compare-and-set on the state version, so concurrent deliveries and tool calls never overwrite each
  // other. `mutate` returns what changed (a notification, or `{ cleared }`) or undefined when nothing did;
  // `change` is what a window reads from the feed: the ids that changed, or that everything was cleared.
  async function change(mutate, effects = () => ({}), feedChange = result => ({ id: result.id })) {
    for (let attempt = 0; attempt < 5; attempt += 1) {
      const { items, version } = await load();
      const next = [...items];
      const result = mutate(next);
      if (result === undefined) return null;
      const kept = next.slice(0, KEEP);
      while (kept.length > 1 && bytes({ items: kept }) > MAX_VALUE_BYTES) kept.pop();
      try {
        await feed.transact({
          schemaVersion: SCHEMA,
          checks: [{ key: KEY, version }],
          writes: [{ tag: 'put', val: [KEY, JSON.stringify({ items: kept })] }],
          occurrences: [],
          wakes: [],
          ...effects(result),
        }, feedChange(result));
        return result;
      } catch (error) {
        if (codeOf(error) !== 'CONFLICT') throw error;
      }
    }
    return fail('CONFLICT', 'Notifications changed too often; try again');
  }

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
    const context = replyContext(payload.reply_context);
    return {
      id: occurrenceId,
      kind,
      text: cut(text, 4000),
      // Part of a thread (a reply, not its first message): the conversation
      // continues there.
      in_thread: Boolean(context?.thread_ts && context.thread_ts !== context.message_ts),
      channel: String(event.channel ?? item.channel ?? payload.reply_context?.channel_id ?? '').slice(0, 64),
      user: String(event.user ?? '').slice(0, 64),
      occurred_at: payload.occurred_at,
      // Opaque to the App: the generator binds it to the connection, so a
      // later reply goes to the same conversation.
      reply_context: context,
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
    const event = metadata?.event ?? {};
    const bots = (Array.isArray(metadata?.authorizations) ? metadata.authorizations : [])
      .filter(value => value?.is_bot && value.user_id).map(value => value.user_id);
    const app = metadata?.api_app_id;
    if (!bots.length && !app) return Boolean(event.bot_id) || event.subtype === 'bot_message';
    return bots.includes(event.user)
      || (Boolean(app) && (event.app_id === app || event.bot_profile?.app_id === app));
  }

  function occurrence(item) {
    const occurredAtMs = Number.isFinite(Date.parse(item.occurred_at)) ? Date.parse(item.occurred_at) : Date.now();
    const where = `${item.in_thread ? ' in a thread' : ''}${item.channel ? ` in <#${item.channel}>` : ''}`;
    const who = item.user ? ` from <@${item.user}>` : '';
    return {
      automationId: AUTOMATION,
      occurrenceId: events.occurrenceId(`slack:${item.id}`, BigInt(occurredAtMs)),
      eventVersion: 1,
      occurredAtMs: BigInt(occurredAtMs),
      scheduleRevision: null,
      payload: JSON.stringify({
        kind: item.kind, text: item.text, occurred_at: item.occurred_at,
        ...(item.channel ? { channel: item.channel } : {}), ...(item.user ? { user: item.user } : {}),
      }),
      invocation: { prompt: `Slack ${item.kind.replace('_', ' ')}${where}${who}: ${item.text}`, artifacts: [] },
    };
  }

  // Deliveries are at least once; the occurrence identity keeps a redelivered
  // Slack event from being stored or forwarded twice.
  async function receive(kind, payload, { occurrenceId }) {
    // Edits, unfurls and deletions (hidden message events) are not new
    // messages, and their text and author live elsewhere: they are not kept.
    if (payload.metadata?.event?.hidden) return null;
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
        return { merged: true, id: items[same].id };
      }
      items.unshift(item);
      return item;
    }, item => (forward && !item.merged ? { occurrences: [occurrence(item)] } : {}));
    // This App's own messages and reactions are kept but start no run: a
    // reply or reaction posted by an automation must not trigger it again.
    try {
      await accept(!ownEvent(payload.metadata));
    } catch (error) {
      if (!OPTIONAL_OCCURRENCE.has(codeOf(error))) throw error;
      await accept(false);
    }
    return null;
  }

  // The notification's own connection and reply context: a reply can only
  // go back to the conversation Slack delivered it from.
  async function target(id) {
    const { items } = await load();
    const item = items.find(entry => entry.id === id);
    if (!item) fail('NOT_FOUND', `No Slack notification ${id}`);
    if (!item.reply_context || !item.connection_id) {
      fail('INVALID_ARGUMENT', 'This notification cannot be answered in Slack');
    }
    return item;
  }

  // Without a request_id every call acts; with one, a retry of the same
  // request acts at most once (the generator keeps the first outcome).
  const key = (tool, requestId) => (requestId ? `${tool}:${requestId}` : null);

  async function act(item, action, input, idempotencyKey) {
    const { accepted, result } = await connections.action({
      connectionId: item.connection_id,
      action,
      input: JSON.stringify(input),
      context: JSON.stringify(item.reply_context),
      idempotencyKey,
    });
    return { accepted, result: result ? JSON.parse(result) : null };
  }

  return {
    tools: {
      async list_notifications({ kind, limit = 20 } = {}) {
        const { items } = await load();
        return {
          notifications: items
            .filter(item => !kind || item.kind === kind)
            .slice(0, limit)
            .map(({ reply_context: context, connection_id: connection, message: _message, ...item }) => ({
              ...item, can_reply: Boolean(context && connection),
            })),
        };
      },

      async reply({ id, text, mode = 'thread', request_id: requestId }) {
        if (!text.trim()) fail('INVALID_ARGUMENT', 'A reply needs some text');
        const item = await target(id);
        // A message in a thread is answered in that thread: a channel post would
        // split the conversation the person is having.
        const { accepted, result } = await act(item, 'notification.reply', { text, mode: item.in_thread ? 'thread' : mode }, key('reply', requestId));
        return { posted: accepted, message_ts: result?.message_ts ?? null };
      },

      async react({ id, name, request_id: requestId }) {
        const item = await target(id);
        const { accepted } = await act(item, 'slack.reaction.add', { name }, key('react', requestId));
        return { reacted: accepted };
      },

      async clear_notifications() {
        const cleared = await change(items => {
          const count = items.length;
          items.length = 0;
          return { cleared: count };
        }, () => ({}), () => ({ cleared: true }));
        return cleared ?? { cleared: 0 };
      },

      // What a window reads to follow notifications that arrive or are cleared elsewhere (see the page SDK's `chariox.changes`).
      view_changes: input => feed.changes(input),
    },

    // One handler per signed incoming event, all of one shape.
    events: Object.fromEntries(KINDS.map(kind => [kind, (payload, context) => receive(kind, payload, context)])),
  };
}
