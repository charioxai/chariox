import { applyChange, awaitsReminder, follow } from "./model.js"

// The page is a native view: `window.chariox` is its only way to the App backend. The sandbox has no
// forms, so Add answers its button and Enter. Todos are read once, then followed through the change feed.
const root = document.getElementById("app")
root.innerHTML = `<main>
  <header>
    <h1>Todo</h1>
    <label class="filter"><input type="checkbox" id="open-only"> Open only</label>
  </header>
  <div id="new-todo" role="group" aria-label="New Todo">
    <input id="title" required maxlength="200" placeholder="What needs doing?" aria-label="Title">
    <input id="due" type="datetime-local" aria-label="Due (optional)">
    <button type="button" id="add">Add</button>
  </div>
  <p id="status" role="status" aria-live="polite"></p>
  <p id="reminders" class="hint" role="status" hidden></p>
  <ul id="todos" aria-label="Todos"></ul>
</main>`
const $ = (id) => document.getElementById(id)
const list = $("todos")
const status = $("status")
let todos = []
let shown = ""
let listFailed = false
list.tabIndex = -1

function say(message, error = false) {
  status.textContent = message
  status.classList.toggle("error", error)
}

async function call(tool, input = {}) {
  try {
    return await window.chariox.call(tool, input)
  } catch (error) {
    say(error.message || "The App could not complete that.", true)
    throw error
  }
}

function render() {
  const openOnly = $("open-only").checked
  const visible = openOnly ? todos.filter((todo) => !todo.done) : todos
  // Keep keyboard/screen-reader focus on the same row and control. If that
  // row left the list, focus the checkbox of the row now in its place (not
  // its Delete, which a repeated Enter would press), or the list itself.
  const active = document.activeElement
  const row = active?.closest?.("li")?.dataset.id
  const index = [...list.children].findIndex((li) => li.dataset.id === row)
  const control = active?.tagName === "BUTTON" ? "button" : active?.type === "checkbox" ? "input" : null
  list.replaceChildren(...visible.map(item))
  if (row && control) {
    const target = list.querySelector(`li[data-id="${CSS.escape(row)}"] ${control}`)
      ?? list.children[Math.min(index, list.children.length - 1)]?.querySelector("input")
      ?? list
    target.focus()
  }
  if (!status.classList.contains("error")) {
    say(visible.length ? `${todos.filter((t) => !t.done).length} open` : "Nothing to do.")
  }
}

function item(todo) {
  const li = document.createElement("li")
  li.dataset.id = todo.id
  li.classList.toggle("done", todo.done)
  const box = document.createElement("input")
  box.type = "checkbox"
  box.checked = todo.done
  box.setAttribute("aria-label", `Done: ${todo.title}`)
  box.addEventListener("change", () => mutate("complete_todo", { id: todo.id, done: box.checked }))
  const title = document.createElement("span")
  title.className = "title"
  title.textContent = todo.title
  li.append(box, title)
  if (todo.created_by === "agent") {
    const by = document.createElement("span")
    by.className = "by-agent"
    by.textContent = "added by an agent"
    li.append(by)
  }
  if (todo.due_at_ms != null) {
    const due = document.createElement("time")
    due.className = "due"
    due.dateTime = new Date(todo.due_at_ms).toISOString()
    due.textContent = new Date(todo.due_at_ms).toLocaleString([], { dateStyle: "medium", timeStyle: "short" })
    due.classList.toggle("overdue", !todo.done && todo.due_at_ms < Date.now())
    li.append(due)
  }
  const remove = document.createElement("button")
  remove.className = "link"
  remove.type = "button"
  remove.textContent = "Delete"
  remove.setAttribute("aria-label", `Delete ${todo.title}`)
  remove.addEventListener("click", () => mutate("delete_todo", { id: todo.id }))
  li.append(remove)
  return li
}

// Re-render only when the list changed, so following the feed never moves keyboard or
// screen-reader focus off the row a person is on.
function show(next) {
  const text = JSON.stringify(next)
  if (text === shown) return
  shown = text
  todos = next
  render()
  showReminders()
}

async function reload() {
  let result
  try {
    result = await window.chariox.call("list_todos", {})
  } catch (error) {
    listFailed = true
    say(error.message || "The App could not load Todos.", true)
    return
  }
  // A load error clears once loading works again; a failed change's error
  // stays until the person's next successful change.
  if (listFailed) {
    listFailed = false
    say("")
    shown = ""
  }
  show(result.todos)
}

// Reminders only reach a workflow while the reminders automation is active.
// Say so while a Todo still waits for its reminder; Chariox owns the fix.
// Chariox cannot pause an automation yet; `paused` reads like `disabled`.
const OFF = "Reminders are turned off in Chariox."
const REMINDERS = {
  paused: OFF,
  broken: "Reminders are broken. Fix the reminders automation in Chariox.",
  disabled: OFF,
  missing: "Reminders are not set up. Add the reminders automation in Chariox.",
}
let reminderState = "unknown"
async function checkReminders() {
  try {
    reminderState = (await window.chariox.call("reminder_status", {})).state
  } catch {
    reminderState = "unknown"
  }
  showReminders()
}
function showReminders() {
  const hint = $("reminders")
  const message = todos.some(awaitsReminder) ? REMINDERS[reminderState] : undefined
  if (hint.textContent !== (message ?? "")) hint.textContent = message ?? ""
  hint.hidden = !message
}

// Always read the list afterwards, so a refused change (e.g. a checkbox the
// browser already flipped) shows the stored state again.
async function mutate(tool, input) {
  let ok = false
  try {
    await call(tool, input)
    ok = true
    say("")
  } catch {}
  shown = ""
  await reload()
  return ok
}

async function add() {
  const title = $("title").value.trim()
  if (!title) return
  const due = $("due").value
  const input = { title }
  if (due) input.due_at_ms = new Date(due).getTime()
  if (!(await mutate("create_todo", input))) return
  $("title").value = ""
  $("due").value = ""
  $("title").focus()
}
$("add").addEventListener("click", add)
$("title").addEventListener("keydown", (event) => { if (event.key === "Enter") add() })
$("open-only").addEventListener("change", () => render())

// Agents and other windows change Todos too: read the feed's position, then the list, then follow the
// feed. The position is read first, so a change in between is applied again rather than missed.
async function follow_feed() {
  let since
  while (since === undefined) {
    try {
      since = (await window.chariox.changes(0)).latest
    } catch {
      await new Promise((resolve) => setTimeout(resolve, 2000))
    }
  }
  await reload()
  for (;;) {
    try {
      since = await follow(window.chariox.changes, since, {
        apply: (delta) => show(applyChange(todos, delta)),
        reload,
      })
    } catch {
      await new Promise((resolve) => setTimeout(resolve, 2000))
    }
  }
}
follow_feed()
checkReminders()
// The reminders automation changes outside the App: look again while a Todo waits for its reminder.
setInterval(() => { if (!document.hidden && todos.some(awaitsReminder)) checkReminders() }, 15000)
