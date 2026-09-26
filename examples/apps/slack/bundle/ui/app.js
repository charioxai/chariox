// Slack view. It reads the App's own tools through window.chariox.call, as the
// signed-in person; agents use the same tools. It shows only what the App
// stored, never Slack credentials or host paths.
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
  if (notification.can_reply) li.append(replyForm(notification))
  return li
}

// Replies go back through the App's granted Slack connection, into the
// notification's thread.
function replyForm(notification) {
  const form = document.createElement("form")
  form.className = "reply"
  const input = document.createElement("input")
  input.required = true
  input.maxLength = 4000
  input.placeholder = "Reply in thread"
  // Editing the text makes it a new reply.
  input.addEventListener("input", () => { delete form.dataset.requestId })
  input.setAttribute("aria-label", `Reply to ${LABELS[notification.kind] ?? "notification"} from ${notification.user || "Slack"}`)
  const send = document.createElement("button")
  send.type = "submit"
  send.textContent = "Reply"
  form.append(input, send)
  form.addEventListener("submit", async (event) => {
    event.preventDefault()
    send.disabled = true
    // One id per submitted reply: resending after an unclear failure posts
    // it at most once.
    const requestId = form.dataset.requestId ??= crypto.randomUUID()
    try {
      const { posted } = await window.chariox.call("reply", { id: notification.id, text: input.value, request_id: requestId })
      if (!posted) {
        say("Slack did not accept the reply.", true)
        return
      }
      delete form.dataset.requestId
      input.value = ""
      say("Replied in Slack.")
    } catch (error) {
      say(error.message || "The reply could not be sent.", true)
    } finally {
      send.disabled = false
    }
  })
  return form
}

async function refresh() {
  try {
    const kind = $("kind").value
    const { notifications } = await window.chariox.call("list_notifications", kind ? { kind, limit: 50 } : { limit: 50 })
    const next = JSON.stringify(notifications)
    // Never rebuild the list under a reply being written.
    const drafting = [...list.querySelectorAll(".reply input")]
      .some((input) => input.value || input === document.activeElement)
    if (next !== shown && !drafting) {
      shown = next
      list.replaceChildren(...notifications.map(item))
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
refresh()
setInterval(refresh, 5000)
