// Slack view, a native view: `window.chariox` is the page's only way to the App backend, which it reads as the
// signed-in person; agents use the same tools. It shows only what the App stored, never Slack credentials or host
// paths. The sandbox has no forms, so Reply answers its button and Enter. The list is read once and read again
// when the App's change feed says a notification arrived or was cleared.
const root = document.getElementById("app")
root.innerHTML = `<main>
  <header>
    <h1>Slack</h1>
    <label class="filter">Show
      <select id="kind" aria-label="Show">
        <option value="">everything</option>
        <option value="mentioned">mentions</option>
        <option value="channel_message">channel messages</option>
        <option value="reaction_added">reactions</option>
      </select>
    </label>
  </header>
  <p id="status" role="status" aria-live="polite"></p>
  <ul id="notifications" aria-label="Slack notifications"></ul>
  <p id="empty" class="hint" hidden>No Slack notifications yet. The owner connects Slack once, then routes
    each event type here with <code>/app inbox add</code> and <code>--connection</code>, and chooses the
    workflow that handles them with <code>/app automation add … notifications notification</code>.</p>
  <button id="clear" class="link" type="button">Clear</button>
</main>`
const $ = (id) => document.getElementById(id)
const list = $("notifications")
const status = $("status")
const LABELS = { mentioned: "Mention", channel_message: "Message", reaction_added: "Reaction" }
let shown = ""

function say(message, error = false) {
  status.textContent = message
  status.classList.toggle("error", error)
}

function item(notification) {
  const li = document.createElement("li")
  const meta = document.createElement("div")
  meta.className = "meta"
  const when = new Date(notification.occurred_at)
  const where = notification.channel ? ` · #${notification.channel}` : ""
  const who = notification.user ? ` · @${notification.user}` : ""
  meta.textContent = `${LABELS[notification.kind] ?? notification.kind}${where}${who} · ${
    Number.isNaN(when.getTime()) ? notification.occurred_at : when.toLocaleString()}`
  const text = document.createElement("div")
  text.className = "text"
  text.textContent = notification.text
  li.append(meta, text)
  if (notification.can_reply) li.append(replyRow(notification))
  return li
}

// Replies go back through the App's granted Slack connection, into the
// notification's thread.
function replyRow(notification) {
  const row = document.createElement("div")
  row.className = "reply"
  row.dataset.id = notification.id
  const input = document.createElement("input")
  input.maxLength = 4000
  input.placeholder = "Reply in thread"
  // Editing the text makes it a new reply.
  input.addEventListener("input", () => { delete row.dataset.requestId })
  input.setAttribute("aria-label", `Reply to ${LABELS[notification.kind] ?? "notification"} from ${notification.user || "Slack"}`)
  const send = document.createElement("button")
  send.type = "button"
  send.textContent = "Reply"
  row.append(input, send)
  async function reply() {
    if (!input.value.trim()) return
    send.disabled = true
    // One id per submitted reply: resending after an unclear failure posts
    // it at most once.
    const requestId = row.dataset.requestId ??= crypto.randomUUID()
    try {
      const { posted } = await window.chariox.call("reply", { id: notification.id, text: input.value, request_id: requestId })
      if (!posted) {
        say("Slack did not accept the reply.", true)
        return
      }
      delete row.dataset.requestId
      input.value = ""
      say("Replied in Slack.")
    } catch (error) {
      say(error.message || "The reply could not be sent.", true)
    } finally {
      send.disabled = false
    }
  }
  send.addEventListener("click", reply)
  input.addEventListener("keydown", (event) => { if (event.key === "Enter") reply() })
  return row
}

async function refresh() {
  try {
    const kind = $("kind").value
    const { notifications } = await window.chariox.call("list_notifications", kind ? { kind, limit: 50 } : { limit: 50 })
    const next = JSON.stringify(notifications)
    if (next !== shown) {
      shown = next
      // A reply being written survives the rebuild: its text, request id and
      // focus move to the same notification's new row.
      const drafts = new Map([...list.querySelectorAll(".reply")].map((row) => [row.dataset.id, {
        value: row.querySelector("input").value,
        requestId: row.dataset.requestId,
        focused: row.contains(document.activeElement),
      }]))
      list.replaceChildren(...notifications.map(item))
      for (const row of list.querySelectorAll(".reply")) {
        const draft = drafts.get(row.dataset.id)
        if (!draft) continue
        const input = row.querySelector("input")
        input.value = draft.value
        if (draft.requestId) row.dataset.requestId = draft.requestId
        if (draft.focused) input.focus()
      }
    }
    $("empty").hidden = notifications.length > 0
    if (status.classList.contains("error")) say("")
  } catch (error) {
    say(error.message || "The App could not list notifications.", true)
  }
}

$("kind").addEventListener("change", () => { shown = ""; refresh() })
$("clear").addEventListener("click", async () => {
  try {
    const { cleared } = await window.chariox.call("clear_notifications", {})
    say(`Cleared ${cleared} notification${cleared === 1 ? "" : "s"}.`)
    shown = ""
    refresh()
  } catch (error) {
    say(error.message || "The App could not clear notifications.", true)
  }
})

// Slack events arrive while the window is open: read the feed's position, then the list, then follow the feed.
// The position is read first, so a notification in between is read again rather than missed.
async function followFeed() {
  let since
  while (since === undefined) {
    try {
      since = (await window.chariox.changes(0)).latest
    } catch {
      await new Promise((resolve) => setTimeout(resolve, 2000))
    }
  }
  await refresh()
  for (;;) {
    try {
      const answer = await window.chariox.changes(since, 20000)
      since = answer.latest
      if (answer.dropped || answer.changes.length) await refresh()
    } catch {
      await new Promise((resolve) => setTimeout(resolve, 2000))
    }
  }
}
followFeed()
