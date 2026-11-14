// What the Todo window holds and how it follows the App's change feed, apart from the page so Node can test it.

/** Apply one change from the feed: a Todo put in place (or added) or a Todo deleted. Replaying a change is harmless. */
export function applyChange(todos, delta) {
  if (delta.deleted !== undefined) return todos.filter(todo => todo.id !== delta.deleted);
  const known = todos.some(todo => todo.id === delta.todo.id);
  return known ? todos.map(todo => todo.id === delta.todo.id ? delta.todo : todo) : [...todos, delta.todo];
}

/** A Todo whose reminder is still to come, which is when the window says whether reminders reach anyone. */
export const awaitsReminder = todo => !todo.done && !todo.reminded && todo.due_at_ms != null;

/**
 * One round of the feed after position `since`: apply what arrived, or read the whole list again when the
 * window fell behind the feed's kept changes. Returns the position to ask from next.
 */
export async function follow(changes, since, { apply, reload }) {
  const answer = await changes(since, 20000);
  if (answer.dropped) await reload();
  else for (const { delta } of answer.changes) apply(delta);
  return answer.latest;
}
