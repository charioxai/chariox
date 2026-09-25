// Todo reference App backend. Todos live in kernel-managed App state; due
// reminders are kernel-owned wakes, so this App does not stay running to wait.
// When a reminder falls due, the App commits its `todo_due` occurrence and the
// reminder mark atomically; the kernel hands it to the configured workflow.
import { randomUUID } from 'node:crypto';

const KEY = 'todos';
const SCHEMA = 1;
// One App automation routes `todo_due` to the workflow the user chose with
// `/app automation add <installation> reminders todo_due …`.
const AUTOMATION = 'reminders';
// An installation has at most 256 wakes; MAX_TODOS keeps due Todos below
// that. The single `todos` value is at most 256 KiB of UTF-8, checked before
// each write so the App reports its own limit.
const MAX_TODOS = 150;
const MAX_VALUE_BYTES = 256 * 1024;
// Without a configured (active) automation, or for a very old due time, the
// kernel refuses the occurrence; the reminder is still recorded.
const OPTIONAL_OCCURRENCE = new Set(['NOT_FOUND', 'AUTOMATION_INACTIVE', 'OCCURRENCE_TOO_OLD']);

export default function register(chariox) {
  const fail = (code, message) => new chariox.AppError(code, message);

  async function load() {
    const record = await chariox.state.get(KEY);
    return { todos: record?.value?.todos ?? [], version: record?.version ?? null };
  }

  // Compare-and-set on the state version: concurrent human and agent edits
  // retry against the latest list instead of overwriting each other.
  async function change(mutate, extra = () => ({})) {
    for (let attempt = 0; attempt < 5; attempt += 1) {
      const { todos, version } = await load();
      const next = todos.map(todo => ({ ...todo }));
      const result = mutate(next);
      if (Buffer.byteLength(JSON.stringify({ todos: next })) > MAX_VALUE_BYTES) {
        throw fail('LIMIT_EXCEEDED', 'Todos are too large; shorten notes or delete finished Todos');
      }
      try {
        await chariox.state.transaction({
          schemaVersion: SCHEMA,
          checks: [{ key: KEY, version }],
          writes: [{ key: KEY, value: { todos: next } }],
          ...extra(next, result),
        });
        return result;
      } catch (error) {
        if (error?.code !== 'CONFLICT') throw error;
      }
    }
    throw fail('CONFLICT', 'Todos changed too often; try again');
  }

  function find(todos, id) {
    const todo = todos.find(item => item.id === id);
    if (!todo) throw fail('NOT_FOUND', `No Todo with id ${id}`);
    return todo;
  }

  // Keep the Todo's wake in step with its due time and completion.
  function wakeFor(todo) {
    const id = `todo-${todo.id}`;
    return todo.due_at_ms != null && !todo.done && !todo.reminded
      ? { op: 'set', id, dueAtMs: todo.due_at_ms, revision: String(todo.revision) }
      : { op: 'cancel', id };
  }
  const wakesFor = (_next, todo) => ({ wakes: [wakeFor(todo)] });

  chariox.tools.register('list_todos', async ({ open_only: openOnly } = {}) => {
    const { todos } = await load();
    return { todos: openOnly ? todos.filter(todo => !todo.done) : todos };
  });

  chariox.tools.register('create_todo', ({ title, due_at_ms: dueAtMs, notes }) => change(todos => {
    if (todos.length >= MAX_TODOS) throw fail('LIMIT_EXCEEDED', `At most ${MAX_TODOS} Todos`);
    const todo = { id: randomUUID().slice(0, 8), title, notes: notes ?? '', due_at_ms: dueAtMs ?? null,
      done: false, reminded: false, revision: 1, created_at_ms: Date.now() };
    todos.push(todo);
    return todo;
  }, wakesFor));

  chariox.tools.register('update_todo', ({ id, title, due_at_ms: dueAtMs, notes }) => change(todos => {
    const todo = find(todos, id);
    if (title !== undefined) todo.title = title;
    if (notes !== undefined) todo.notes = notes;
    if (dueAtMs !== undefined) {
      todo.due_at_ms = dueAtMs;
      todo.reminded = false;
    }
    todo.revision += 1;
    return todo;
  }, wakesFor));

  chariox.tools.register('complete_todo', ({ id, done }) => change(todos => {
    const todo = find(todos, id);
    todo.done = done;
    todo.revision += 1;
    return todo;
  }, wakesFor));

  chariox.tools.register('delete_todo', ({ id }) => change(todos => {
    const index = todos.findIndex(todo => todo.id === id);
    if (index < 0) throw fail('NOT_FOUND', `No Todo with id ${id}`);
    const [todo] = todos.splice(index, 1);
    return { deleted: todo.id };
  }, (_next, result) => ({ wakes: [{ op: 'cancel', id: `todo-${result.deleted}` }] })));

  // A due wake marks the Todo reminded and emits one `todo_due` occurrence in
  // the same transaction. Stale revisions (edited or completed since) are
  // ignored; the occurrence identity keeps redelivered wakes idempotent. When
  // the kernel refuses the optional occurrence (no active automation, or a
  // due time too old to deliver), the reminder is recorded without it.
  chariox.schedule.onWake(async wake => {
    if (!wake.id.startsWith('todo-')) return null;
    const id = wake.id.slice('todo-'.length);
    const remind = withOccurrence => change(todos => {
      const todo = todos.find(item => item.id === id);
      if (!todo || todo.done || todo.reminded || String(todo.revision) !== wake.revision) return null;
      todo.reminded = true;
      return todo;
    }, (_next, todo) => todo && withOccurrence ? { occurrences: [occurrence(todo)] } : {});
    try {
      await remind(true);
    } catch (error) {
      if (!OPTIONAL_OCCURRENCE.has(error?.code)) throw error;
      await remind(false);
    }
    return null;
  });

  function occurrence(todo) {
    return {
      automationId: AUTOMATION,
      occurrenceId: chariox.events.occurrenceId(`todo:${todo.id}:${todo.revision}`, todo.due_at_ms),
      eventVersion: 1,
      occurredAtMs: todo.due_at_ms,
      scheduleRevision: String(todo.revision),
      payload: { todo_id: todo.id, title: todo.title, due_at_ms: todo.due_at_ms },
      invocation: { prompt: `The Todo "${todo.title}" is due now.`, artifacts: [] },
    };
  }
}
