// Documents view. It talks to the App only through window.chariox.call, which
// runs the App's own tools as the signed-in person; agents use the same tools.
import { markdownToHtml, sanitizeHtmlInto } from "./preview.js"

const $ = (id) => document.getElementById(id)
let documents = []
let current = null // { id, revision, kind, content } of the loaded document
let dirty = false
// The App view has no modal dialogs (its sandbox omits allow-modals), so
// destructive steps ask for a second click instead.
let pendingOpen = null
let pendingDelete = false

function say(message, error = false) {
  $("status").textContent = message
  $("status").classList.toggle("error", error)
}

async function call(tool, input = {}) {
  try {
    return await window.chariox.call(tool, input)
  } catch (error) {
    say(error.message || "The App could not complete that.", true)
    throw error
  }
}

function renderList() {
  const list = $("docs")
  const selected = current?.id
  list.replaceChildren(...documents.map((doc) => {
    const item = document.createElement("li")
    const button = document.createElement("button")
    button.type = "button"
    button.textContent = doc.title
    if (doc.folder) {
      const folder = document.createElement("span")
      folder.className = "folder"
      folder.textContent = doc.folder
      button.append(folder)
    }
    button.setAttribute("aria-current", String(doc.id === selected))
    button.addEventListener("click", () => open(doc.id))
    item.append(button)
    return item
  }))
}

let listed = ""
async function refreshList() {
  const result = await call("list_documents")
  const next = JSON.stringify(result.documents)
  if (next === listed) return
  listed = next
  documents = result.documents
  renderList()
  const doc = current && documents.find((item) => item.id === current.id)
  if (current && !doc) close()
  else if (doc && doc.revision !== current.revision && !dirty) await open(doc.id)
  else if (doc && doc.revision !== current.revision) say(`A newer revision (${doc.revision}) exists. Save will be refused until you reload.`, true)
}

async function open(id) {
  if (dirty && current?.id !== id && pendingOpen !== id) {
    pendingOpen = id
    say("Unsaved changes. Choose the document again to discard them.", true)
    return
  }
  pendingOpen = null
  const doc = await call("read_document", { id })
  current = { id: doc.id, revision: doc.revision, kind: doc.kind, content: doc.content }
  dirty = false
  $("empty").hidden = true
  $("editor").hidden = false
  $("title").value = doc.title
  $("folder").value = doc.folder
  $("kind").value = doc.kind
  $("content").value = doc.content
  $("revision").textContent = `revision ${doc.revision}`
  $("versions").replaceChildren(new Option("Restore…", ""),
    ...doc.versions.filter((revision) => revision !== doc.revision).reverse().map((revision) => new Option(`revision ${revision}`, String(revision))))
  say("")
  renderPreview()
  renderList()
}

function close() {
  current = null
  dirty = false
  $("editor").hidden = true
  $("empty").hidden = false
  renderList()
}

function renderPreview() {
  const article = $("preview")
  if (article.hidden || !current) return
  const text = $("content").value
  if (current.kind === "html") article.replaceChildren(sanitizeHtmlInto(text, document))
  else article.replaceChildren(sanitizeHtmlInto(markdownToHtml(text), document))
}

async function save() {
  if (!current) return
  try {
    const saved = await call("update_document", {
      id: current.id, expected_revision: current.revision,
      content: $("content").value, title: $("title").value.trim() || "Untitled", folder: $("folder").value.trim(),
    })
    dirty = false
    say(`Saved revision ${saved.revision}.`)
    await open(saved.id)
    await refreshList()
  } catch {}
}

$("new-doc").addEventListener("click", async () => {
  const doc = await call("create_document", { title: "Untitled", kind: $("new-kind").value })
  await refreshList()
  await open(doc.id)
})
$("save").addEventListener("click", save)
$("content").addEventListener("input", () => { dirty = true; renderPreview() })
for (const id of ["title", "folder"]) $(id).addEventListener("input", () => { dirty = true })
$("toggle-preview").addEventListener("click", () => {
  const article = $("preview")
  article.hidden = !article.hidden
  $("content").hidden = !article.hidden
  $("toggle-preview").setAttribute("aria-pressed", String(!article.hidden))
  $("toggle-preview").textContent = article.hidden ? "Preview" : "Edit"
  renderPreview()
})
$("versions").addEventListener("change", async (event) => {
  const revision = Number(event.target.value)
  if (!revision || !current) return
  const restored = await call("restore_version", { id: current.id, revision, expected_revision: current.revision })
  say(`Restored revision ${revision} as revision ${restored.revision}.`)
  await open(current.id)
})
$("delete").addEventListener("click", async () => {
  if (!current) return
  if (!pendingDelete) {
    pendingDelete = true
    $("delete").textContent = "Confirm delete"
    setTimeout(() => { pendingDelete = false; $("delete").textContent = "Delete" }, 4000)
    return
  }
  pendingDelete = false
  $("delete").textContent = "Delete"
  await call("delete_document", { id: current.id })
  close()
  await refreshList()
})
$("import").addEventListener("click", async () => {
  const doc = await call("import_document", {})
  await refreshList()
  await open(doc.id)
})
$("export").addEventListener("click", async () => {
  if (!current) return
  await call("export_document", { id: current.id })
  say("Export requested; choose where to save it.")
})
document.addEventListener("keydown", (event) => {
  if ((event.metaKey || event.ctrlKey) && event.key === "s") { event.preventDefault(); save() }
})

refreshList().catch(() => {})
setInterval(() => { if (!document.hidden) refreshList().catch(() => {}) }, 2000)
