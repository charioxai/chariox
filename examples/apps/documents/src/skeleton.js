// The page's markup: the client view carries script and style only, so the layout is built here.
export const skeleton = `
  <div class="layout">
    <nav aria-label="Documents">
      <div class="nav-head">
        <h1>Documents</h1>
        <span>
          <select id="new-kind" aria-label="Format for a new document">
            <option value="markdown">Markdown</option>
            <option value="html">HTML</option>
          </select>
          <button type="button" id="new-doc">New</button>
          <button type="button" id="import-docs" title="Import Markdown, HTML or text files">Import</button>
        </span>
      </div>
      <ul id="docs"></ul>
    </nav>
    <main>
    <div id="editor" hidden>
      <div class="doc-head">
        <input id="title" aria-label="Title" maxlength="200">
        <input id="folder" aria-label="Folder" maxlength="200" placeholder="Folder">
        <select id="kind" aria-label="Format" disabled>
          <option value="markdown">Markdown</option>
          <option value="html">HTML</option>
        </select>
      </div>
      <div class="toolbar">
        <button type="button" id="save">Save</button>
        <button type="button" class="link" id="toggle-preview" aria-pressed="false">Preview</button>
        <label class="link">History <select id="versions" aria-label="Restore an earlier revision"></select></label>
        <button type="button" class="link" id="export">Export</button>
        <button type="button" class="link danger" id="delete">Delete</button>
        <span id="revision" class="muted"></span>
      </div>
      <p id="status" role="status" aria-live="polite"></p>
      <textarea id="content" aria-label="Content" spellcheck="true"></textarea>
      <article id="preview" hidden aria-label="Preview"></article>
    </div>
    <p id="empty" class="muted">Choose or create a document.</p>
    </main>
  </div>
`
