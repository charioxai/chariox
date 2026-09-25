// Todo view. It talks to the App only through window.chariox.call, which runs
// the App's own tools as the signed-in person; agents use the same tools.
const $ = (id) => document.getElementById(id)
const list = $("todos")
const status = $("status")
let todos = []
let shown = ""

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
  const shown = openOnly ? todos.filter((todo) => !todo.done) : todos
  list.replaceChildren(...shown.map(item))
  if (!status.classList.contains("error")) {
    say(shown.length ? `${todos.filter((t) => !t.done).length} open` : "Nothing to do.")
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

// Re-render only when the list changed, so polling never moves keyboard or
// screen-reader focus off the row a person is on.
async function refresh() {
  const result = await call("list_todos")
  const next = JSON.stringify(result.todos)
  if (next === shown && !status.classList.contains("error")) return
  shown = next
  todos = result.todos
  say("")
  render()
}

async function mutate(tool, input) {
  await call(tool, input)
  await refresh()
}

$("new-todo").addEventListener("submit", async (event) => {
  event.preventDefault()
  const title = $("title").value.trim()
  if (!title) return
  const due = $("due").value
  const input = { title }
  if (due) input.due_at_ms = new Date(due).getTime()
  await mutate("create_todo", input)
  $("new-todo").reset()
  $("title").focus()
})
$("open-only").addEventListener("change", () => render())

// Agents and other views change Todos too; a short poll keeps this view current
// without any network access.
refresh().catch(() => {})
setInterval(() => { if (!document.hidden) refresh().catch(() => {}) }, 2000)
