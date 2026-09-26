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
  return li
}

async function refresh() {
  try {
    const kind = $("kind").value
    const { notifications } = await window.chariox.call("list_notifications", kind ? { kind, limit: 50 } : { limit: 50 })
    const next = JSON.stringify(notifications)
    if (next !== shown) {
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
