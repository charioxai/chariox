// Todo reference App backend. Todos live in kernel-managed App state; due reminders are kernel-owned
// wakes, so this App does not stay running to wait. When a reminder falls due, the App commits its
// `todo_due` occurrence and the reminder mark atomically; the kernel hands it to the configured workflow.
// The imports are injected so the same handlers run under Node tests and as the wasm component (app.ts).
const KEY = 'todos';
// The package's data schema version: 0 until it declares migrations.
const SCHEMA = 0;
// One App automation routes `todo_due` to the workflow the user chose with
// `/app automation add <installation> reminders todo_due …`.
const AUTOMATION = 'reminders';
// An installation has at most 256 wakes; MAX_TODOS keeps due Todos below that. The single `todos`
// value is at most 256 KiB of UTF-8, checked before each write so the App reports its own limit.
const MAX_TODOS = 150;
const MAX_VALUE_BYTES = 256 * 1024;
// Without a configured (active) automation, or for a very old due time, the kernel refuses the
// occurrence; the reminder is still recorded.
const OPTIONAL_OCCURRENCE = new Set(['NOT_FOUND', 'AUTOMATION_INACTIVE', 'OCCURRENCE_TOO_OLD']);

const fail = (code, message) => { throw { code, message, retryable: false }; };
// A refusal from a host import arrives wrapped, one the App raises itself does not.
const codeOf = error => (error?.payload ?? error)?.code;
const byteLength = text => new TextEncoder().encode(text).length;
const newId = () => Array.from(crypto.getRandomValues(new Uint8Array(4)), byte => byte.toString(16).padStart(2, '0')).join('');

export function createApp({ state, events, feed }) {
  async function load() {
    const record = await state.get(KEY);
    return { todos: record ? JSON.parse(record.value).todos : [], version: record?.version ?? null };
  }

  // Compare-and-set on the state version: concurrent human and agent edits retry against the latest
  // list instead of overwriting each other. `mutate` returns what changed (a Todo, or `{ deleted }`)
  // or null when nothing did; that is the change a window reads from the feed.
  async function change(mutate, effects = () => ({})) {
    for (let attempt = 0; attempt < 5; attempt += 1) {
      const { todos, version } = await load();
      const next = todos.map(todo => ({ ...todo }));
      const result = mutate(next);
      if (result === null) return null;
      const text = JSON.stringify({ todos: next });
      if (byteLength(text) > MAX_VALUE_BYTES) fail('LIMIT_EXCEEDED', 'Todos are too large; shorten notes or delete finished Todos');
      try {
        await feed.transact({
          schemaVersion: SCHEMA,
          checks: [{ key: KEY, version }],
          writes: [{ tag: 'put', val: [KEY, text] }],
          occurrences: [],
          wakes: [],
          ...effects(result),
        }, result.deleted === undefined ? { todo: result } : { deleted: result.deleted });
        return result;
      } catch (error) {
        if (codeOf(error) !== 'CONFLICT') throw error;
      }
    }
    return fail('CONFLICT', 'Todos changed too often; try again');
  }

  function find(todos, id) {
    return todos.find(item => item.id === id) ?? fail('NOT_FOUND', `No Todo with id ${id}`);
  }

  // Keep the Todo's wake in step with its due time and completion.
  const wakeChange = todo => {
    const id = `todo-${todo.id}`;
    return todo.due_at_ms != null && !todo.done && !todo.reminded
      ? { tag: 'set', val: { id, dueAtMs: BigInt(todo.due_at_ms), revision: String(todo.revision) } }
      : { tag: 'cancel', val: id };
  };
  const wakesFor = todo => ({ wakes: [wakeChange(todo)] });
  const add = (todos, fields) => {
    if (todos.length >= MAX_TODOS) fail('LIMIT_EXCEEDED', `At most ${MAX_TODOS} Todos`);
    const todo = { id: newId(), notes: '', due_at_ms: null, done: false, reminded: false, revision: 1, created_at_ms: Date.now(), ...fields };
    todos.push(todo);
    return todo;
  };
  // The kernel tells the App who called: a person (the owner) or an agent, including an agent
  // operating this App's own page. Agents may delete only Todos an agent created; people may delete any.
  const byAgent = context => context?.actor?.kind === 'agent';

  function occurrence(todo, scheduled) {
    const dueAt = BigInt(todo.due_at_ms);
    return {
      automationId: AUTOMATION,
      occurrenceId: events.occurrenceId(`todo:${todo.id}:${todo.revision}`, dueAt),
      eventVersion: 1,
      occurredAtMs: dueAt,
      scheduleRevision: scheduled ? String(todo.revision) : null,
      payload: JSON.stringify({ todo_id: todo.id, title: todo.title, due_at_ms: todo.due_at_ms }),
      invocation: { prompt: `The Todo "${todo.title}" is due now.`, artifacts: [] },
    };
  }

  return {
    tools: {
      async list_todos({ open_only: openOnly } = {}) {
        const { todos } = await load();
        return { todos: openOnly ? todos.filter(todo => !todo.done) : todos };
      },

      // Whether due reminders reach the workflow: the reminders automation's state as Chariox shows it
      // ('missing' when none is configured). The workflow itself is never revealed.
      async reminder_status() {
        const reminders = (await events.automations()).find(automation => automation.automationId === AUTOMATION);
        return { state: reminders?.state ?? 'missing', last_delivery: reminders?.lastReceipt?.state ?? null };
      },

      create_todo: ({ title, due_at_ms: dueAtMs, notes }, context) => change(
        todos => add(todos, { title, notes: notes ?? '', due_at_ms: dueAtMs ?? null, created_by: byAgent(context) ? 'agent' : 'person' }),
        wakesFor),

      update_todo: ({ id, title, due_at_ms: dueAtMs, notes }) => change(todos => {
        const todo = find(todos, id);
        if (title !== undefined) todo.title = title;
        if (notes !== undefined) todo.notes = notes;
        if (dueAtMs !== undefined) {
          todo.due_at_ms = dueAtMs;
          todo.reminded = false;
        }
        todo.revision += 1;
        return todo;
      }, wakesFor),

      complete_todo: ({ id, done }) => change(todos => {
        const todo = find(todos, id);
        todo.done = done;
        todo.revision += 1;
        return todo;
      }, wakesFor),

      delete_todo: ({ id }, context) => change(todos => {
        const index = todos.findIndex(todo => todo.id === id);
        if (index < 0) fail('NOT_FOUND', `No Todo with id ${id}`);
        if (byAgent(context) && todos[index].created_by !== 'agent') {
          fail('FORBIDDEN', 'An agent can delete only Todos an agent added; ask the person to delete this one');
        }
        const [todo] = todos.splice(index, 1);
        return { deleted: todo.id };
      }, ({ deleted }) => ({ wakes: [{ tag: 'cancel', val: `todo-${deleted}` }] })),

      // What a window reads to follow changes made elsewhere (see the page SDK's `chariox.changes`).
      view_changes: input => feed.changes(input),
    },

    // An external request (routed to this installation's inbox) creates one Todo. Deliveries are at
    // least once; the occurrence identity keeps a redelivered request from creating a second Todo.
    events: {
      todo_requested: ({ title, notes, due_at_ms: dueAtMs }, { occurrenceId }) => change(todos => {
        if (todos.some(todo => todo.source_occurrence === occurrenceId)) return null;
        return add(todos, { title, notes: notes ?? '', due_at_ms: dueAtMs ?? null, source_occurrence: occurrenceId });
      }, wakesFor),
    },

    // A due wake marks the Todo reminded and emits one `todo_due` occurrence in the same transaction.
    // Stale revisions (edited or completed since) are ignored; the occurrence identity keeps
    // redelivered wakes idempotent. When the kernel refuses the optional occurrence (no active
    // automation, or a due time too old to deliver), the reminder is recorded without it.
    async wake(wake) {
      if (!wake.id.startsWith('todo-')) return;
      const id = wake.id.slice('todo-'.length);
      const remind = (withOccurrence, scheduled = true) => change(todos => {
        const todo = todos.find(item => item.id === id);
        if (!todo || todo.done || todo.reminded || String(todo.revision) !== wake.revision) return null;
        todo.reminded = true;
        return todo;
      }, todo => withOccurrence ? { occurrences: [occurrence(todo, scheduled)] } : {});
      // A wake's revision and an automation's scheduled admission are distinct. The ordinary
      // `/app automation add` binding rejects a schedule revision; retry that canonical shape only after
      // the first transaction rolled back. Both keep the same occurrence identity (including the Todo
      // revision), so both binding modes retain atomic reminder marking and redelivery deduplication.
      for (const scheduled of [true, false]) {
        try {
          await remind(true, scheduled);
          return;
        } catch (error) {
          if (scheduled && codeOf(error) === 'INVALID_ARGUMENT') continue;
          if (!OPTIONAL_OCCURRENCE.has(codeOf(error))) throw error;
          await remind(false);
          return;
        }
      }
    },
  };
}
