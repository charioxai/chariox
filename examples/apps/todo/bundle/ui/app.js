// Todo view. It talks to the App only through window.chariox.call, which runs
// the App's own tools as the signed-in person; agents use the same tools.
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
  const next = JSON.stringify(result.todos)
  if (next === shown) return
  shown = next
  todos = result.todos
  render()
}

// Always re-render afterwards, so a refused change (e.g. a checkbox the
// browser already flipped) shows the stored state again.
async function mutate(tool, input) {
  let ok = false
  try {
    await call(tool, input)
    ok = true
    say("")
  } catch {}
  shown = ""
  await refresh()
  return ok
}

$("new-todo").addEventListener("submit", async (event) => {
  event.preventDefault()
  const title = $("title").value.trim()
  if (!title) return
  const due = $("due").value
  const input = { title }
  if (due) input.due_at_ms = new Date(due).getTime()
  if (!(await mutate("create_todo", input))) return
  $("new-todo").reset()
  $("title").focus()
})
$("open-only").addEventListener("change", () => render())

// On a wide window, keep the right side for Chariox's private conversation
// panel: the agent's work shows next to the list, drawn by the terminal. The
// App only learns that the area is reserved.
const PANEL_WIDTH = 380
async function reservePanel() {
  const panel = window.chariox?.panel
  if (!panel) return
  const wide = innerWidth >= 960
  document.body.classList.toggle("with-panel", wide)
  try {
    await (wide
      ? panel.reserve({ x: innerWidth - PANEL_WIDTH, y: 0, width: PANEL_WIDTH, height: innerHeight })
      : panel.release())
  } catch {
    document.body.classList.remove("with-panel")
  }
}
let resized
addEventListener("resize", () => { clearTimeout(resized); resized = setTimeout(reservePanel, 150) })
reservePanel()

// Agents and other views change Todos too; a short poll keeps this view current
// without any network access.
refresh()
setInterval(() => { if (!document.hidden) refresh() }, 2000)
