// The notes page of the AV1 native-open fixture. It runs in the opaque App frame and reaches its backend
// only through window.chariox.call; whatever it shows was read back from the App's durable state.
const root = document.getElementById('app');
// The App frame is sandboxed without allow-forms, so a <form> never submits: the page wires its own controls.
const controls = document.createElement('div');
controls.className = 'controls';
const input = document.createElement('input');
input.name = 'text'; input.autocomplete = 'off'; input.setAttribute('aria-label', 'Note');
const add = document.createElement('button');
add.type = 'button'; add.textContent = 'Add note';
const status = document.createElement('p');
status.id = 'status'; status.setAttribute('role', 'status');
const list = document.createElement('ul');
list.id = 'notes';
controls.append(input, add);
root.append(controls, status, list);

function show(state, text, code) {
  status.dataset.state = state;
  if (code) status.dataset.code = code; else delete status.dataset.code;
  status.textContent = text;
}

function render(notes) {
  list.replaceChildren(...notes.map(note => {
    const item = document.createElement('li');
    item.dataset.noteId = String(note.id);
    item.textContent = note.text;
    return item;
  }));
}

async function run(method, payload) {
  show('working', 'Working');
  try {
    const result = await chariox.call(method, payload);
    show('ready', 'Saved');
    return result;
  } catch (error) {
    show(error.code === 'CALL_FAILED' ? 'failed' : 'refused', error.message, error.code);
    return undefined;
  }
}

async function refresh() {
  const notes = await run('list_notes', {});
  if (notes) { render(notes); show('ready', notes.length ? 'Loaded' : 'No notes yet'); }
}

async function save() {
  if (!await run('add_note', { text: input.value })) return;
  input.value = '';
  await refresh();
  show('ready', 'Saved');
}
add.addEventListener('click', save);
input.addEventListener('keydown', event => { if (event.key === 'Enter') save(); });

refresh();
