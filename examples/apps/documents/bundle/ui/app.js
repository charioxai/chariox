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
let saving = false
let saveAgain = false
let deleteTimer = null

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
  if (current && !doc) {
    if (dirty) say("This document was deleted elsewhere. Your unsaved text is still shown; copy it before closing.", true)
    else close()
  }
  else if (doc && doc.revision !== current.revision && !dirty) await open(doc.id, { poll: true })
  else if (doc && doc.revision !== current.revision) say(`A newer revision (${doc.revision}) exists. Save will be refused until you reload.`, true)
}

// A person's choice (including the open document, to reload it) asks once
// before discarding unsaved changes; a poll never replaces them.
async function open(id, { poll = false } = {}) {
  if (dirty && !poll && pendingOpen !== id) {
    pendingOpen = id
    say("Unsaved changes. Choose the document again to discard them.", true)
    return
  }
  pendingOpen = null
  resetDelete()
  const doc = await call("read_document", { id })
  if (poll && dirty && current?.id === id) return
  current = { id: doc.id, revision: doc.revision, kind: doc.kind, content: doc.content }
  dirty = false
  $("empty").hidden = true
  $("editor").hidden = false
  $("title").value = doc.title
  $("folder").value = doc.folder
  $("kind").value = doc.kind
  $("content").value = doc.content
  showRevision(doc)
  say("")
  renderPreview()
  renderList()
}

function showRevision({ revision, versions }) {
  $("revision").textContent = `revision ${revision}`
  $("versions").replaceChildren(new Option("Restore…", ""),
    ...versions.filter((each) => each !== revision).reverse().map((each) => new Option(`revision ${each}`, String(each))))
}

function close() {
  resetDelete()
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

// Typing continues while a save is in flight; only the fields that were sent
// count as saved, so later keystrokes stay in the editor and stay dirty, and a
// save requested meanwhile runs once afterwards.
const fields = () => ({ content: $("content").value, title: $("title").value, folder: $("folder").value })
async function save() {
  if (!current || !dirty) return
  if (saving) { saveAgain = true; return }
  saving = true
  const id = current.id
  const sent = fields()
  try {
    const saved = await call("update_document", {
      id, expected_revision: current.revision,
      content: sent.content, title: sent.title.trim() || "Untitled", folder: sent.folder.trim(),
    })
    if (current?.id !== saved.id) return
    Object.assign(current, { revision: saved.revision, content: sent.content })
    showRevision(saved)
    if (JSON.stringify(fields()) === JSON.stringify(sent)) {
      dirty = false
      $("title").value = saved.title
      $("folder").value = saved.folder
    }
    say(`Saved revision ${saved.revision}.`)
    await refreshList()
  } catch {} finally {
    saving = false
    const again = saveAgain && current?.id === id
    saveAgain = false
    if (again) await save()
  }
}

// The owner chooses files in the terminal's trusted prompt, outside this view;
// imported documents then appear through the regular list refresh.
$("import-docs").addEventListener("click", async () => {
  const button = $("import-docs")
  await call("import_documents")
  button.textContent = "Choose files in the prompt…"
  button.disabled = true
  setTimeout(() => { button.textContent = "Import"; button.disabled = false }, 8000)
})

$("export").addEventListener("click", async () => {
  if (!current) return
  const { offered } = await call("export_document", { id: current.id })
  say(`Save ${offered} from the prompt in your terminal.`)
})

$("new-doc").addEventListener("click", async () => {
  if (dirty && pendingOpen !== "new") {
    pendingOpen = "new"
    say("Unsaved changes. Choose New again to discard them.", true)
    return
  }
  pendingOpen = null
  const doc = await call("create_document", { title: "Untitled", kind: $("new-kind").value })
  dirty = false
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
  if (dirty) {
    event.target.value = ""
    say("Save or discard your changes before restoring a version.", true)
    return
  }
  // Restore replaces the text by design, so nothing can be typed meanwhile.
  const locked = ["content", "title", "folder"].map($)
  for (const field of locked) field.readOnly = true
  try {
    const restored = await call("restore_version", { id: current.id, revision, expected_revision: current.revision })
    await open(restored.id)
    say(`Restored revision ${revision} as revision ${restored.revision}.`)
  } catch {} finally {
    for (const field of locked) field.readOnly = false
    event.target.value = ""
  }
})
function resetDelete() {
  pendingDelete = false
  clearTimeout(deleteTimer)
  $("delete").textContent = "Delete"
}
$("delete").addEventListener("click", async () => {
  if (!current) return
  if (!pendingDelete) {
    pendingDelete = true
    $("delete").textContent = "Confirm delete"
    clearTimeout(deleteTimer)
    deleteTimer = setTimeout(() => { pendingDelete = false; $("delete").textContent = "Delete" }, 4000)
    return
  }
  resetDelete()
  await call("delete_document", { id: current.id })
  close()
  await refreshList()
})
document.addEventListener("keydown", (event) => {
  if ((event.metaKey || event.ctrlKey) && event.key === "s") { event.preventDefault(); save() }
})

// The kernel reloads an open view when the App updates (or after a kernel
// restart). Unsaved edits survive in this origin's session storage and come
// back on the revision they started from, so a conflicting save is refused.
const DRAFT = "chariox-documents-draft"
addEventListener("pagehide", () => {
  try {
    if (current && dirty) sessionStorage.setItem(DRAFT, JSON.stringify({ id: current.id, revision: current.revision, ...fields() }))
    else sessionStorage.removeItem(DRAFT)
  } catch {}
})
async function restoreDraft() {
  let draft = null
  try {
    draft = JSON.parse(sessionStorage.getItem(DRAFT) ?? "null")
    sessionStorage.removeItem(DRAFT)
  } catch {}
  if (!draft || !documents.some((doc) => doc.id === draft.id)) return
  await open(draft.id)
  $("title").value = draft.title
  $("folder").value = draft.folder
  $("content").value = draft.content
  current.revision = draft.revision
  dirty = true
  renderPreview()
  say("Restored your unsaved changes after the App reloaded.")
}

refreshList().then(restoreDraft).catch(() => {})
setInterval(() => { if (!document.hidden) refreshList().catch(() => {}) }, 2000)
