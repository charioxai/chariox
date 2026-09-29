// Calls each drill tool once when the view opens and lists what came back.
const results = document.getElementById('results');
const show = (line) => { const item = document.createElement('li'); item.textContent = line; results.append(item); };
async function run(label, tool, input) {
  const started = Date.now();
  try {
    const output = await window.chariox.call(tool, input);
    show(`${label}: ok ${JSON.stringify(output)} (${Date.now() - started} ms)`);
  } catch (error) {
    show(`${label}: error ${error?.code ?? error?.name ?? 'unknown'} ${String(error?.message ?? '').slice(0, 120)} (${Date.now() - started} ms)`);
  }
}
(async () => {
  await run('valid call', 'echo', { text: 'hello' });
  await run('output breaks its schema', 'bad_output', {});
  await run('typed App error', 'refuse', {});
  await run('input breaks its schema', 'echo', { text: 42 });
  await run('undeclared tool', 'missing', {});
  await run('past the call deadline', 'slow', { ms: 45000 });
  show('done');
})();
