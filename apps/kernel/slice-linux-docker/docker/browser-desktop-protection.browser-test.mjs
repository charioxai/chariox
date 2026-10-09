// Opt-in MP-08/MP-11: headed Chromium on an Xvfb/openbox desktop with AT-SPI.
// Real X11 pixels; CDP regions placed at Chromium's AT-SPI document origin.
// Requires DISPLAY, AT_SPI_BUS_ADDRESS, python3-xlib/pyatspi and an explicit
// CHARIOX_KERNEL_BROWSER_EXECUTABLE (DPR via CHARIOX_PROTECTION_TEST_DPR).
import test from 'node:test';
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { setTimeout as delay } from 'node:timers/promises';
import { fenceBrowserCapture, measurePresented } from './browser-protection-regions.mjs';
import { VAULT_VALUE, launchChromium, openFixture, serveFixture } from './browser-protection-fixture.mjs';
const executable = process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE, dpr = Number(process.env.CHARIOX_PROTECTION_TEST_DPR ?? 1);
assert.ok(executable && process.env.DISPLAY && process.env.AT_SPI_BUS_ADDRESS, 'Explicit Chromium and an owned X11/AT-SPI desktop required');
const run = promisify(execFile);
const helper = fileURLToPath(new URL('./browser-desktop-protection.py', import.meta.url));

// Captures the X11 root, masks every window of the browser process with the
// production placement (whole window when unbound), returns exact-color counts.
const CENSUS = `
import importlib.util, json, sys, pyatspi
from Xlib import X, display
spec = importlib.util.spec_from_file_location('p', sys.argv[1]); p = importlib.util.module_from_spec(spec); spec.loader.exec_module(p)
request = json.loads(sys.stdin.read()); pid = request['pid']
desktop = pyatspi.Registry.getDesktop(0)
apps = [desktop.getChildAtIndex(i) for i in range(desktop.childCount)]
apps = [app for app in apps if app is not None and app.get_process_id() == pid]
docs = p.document_rects(apps[0], pyatspi) if len(apps) == 1 else []
d = display.Display(); screen = d.screen(); root = screen.root
masks, bound = [], []
for wid in root.get_full_property(d.intern_atom('_NET_CLIENT_LIST'), X.AnyPropertyType).value:
    window = d.create_resource_object('window', int(wid))
    owner = window.get_full_property(d.intern_atom('_NET_WM_PID'), X.AnyPropertyType)
    if owner is None or int(owner.value[0]) != pid or window.get_attributes().map_state != X.IsViewable: continue
    geometry = window.get_geometry(); origin = root.translate_coords(window, 0, 0)
    client = [origin.x, origin.y, geometry.width, geometry.height]
    placed = p.window_masks(request['protection'], client, client, docs)
    bound.append(placed is not None)
    masks.extend(placed if placed is not None else [client])
W, H = screen.width_in_pixels, screen.height_in_pixels
raw = bytearray(root.get_image(0, 0, W, H, X.ZPixmap, 0xffffffff).data)
def count(data):
    magenta = cyan = 0
    for i in range(0, len(data), 4):
        b, g, r = data[i], data[i+1], data[i+2]
        if r == 255 and g == 0 and b == 255: magenta += 1
        elif r == 0 and g == 255 and b == 255: cyan += 1
    return magenta, cyan
before = count(raw)
for x, y, w, h in masks:
    for row in range(max(0, y), min(H, y+h)):
        start, end = (row*W+max(0, x))*4, (row*W+min(W, x+w))*4
        if end > start: raw[start:end] = bytes(end-start)
after = count(raw)
if request.get('evidence'):
    from PIL import Image
    Image.frombytes('RGB', (W, H), bytes(raw), 'raw', 'BGRX').save(request['evidence'])
print(json.dumps({'before': before, 'after': after, 'bound': bound, 'docs': docs}))
`;
// Optional fixture evidence (masked desktop PNGs); never real-site content.
const evidence = process.env.CHARIOX_PROTECTION_TEST_EVIDENCE;
let shots = 0;
async function census(pid, protection) {
  const child = execFile('python3', ['-I', '-c', CENSUS, helper], { maxBuffer: 1 << 20 });
  child.stdin.end(JSON.stringify({ pid, protection, ...(evidence ? { evidence: path.join(evidence, `desktop-dpr${dpr}-${++shots}.png`) } : {}) }));
  let out = ''; child.stdout.on('data', chunk => { out += chunk; });
  const code = await new Promise(resolve => child.on('close', resolve));
  assert.equal(code, 0, 'census helper failed');
  return JSON.parse(out);
}
async function withDesktop(run_, query = '') {
  const root = await mkdtemp(path.join(tmpdir(), 'cx-desktop-protection-'));
  const fixture = await serveFixture();
  const chromium = await launchChromium({ executable, dpr, root, headless: false, args: ['--force-renderer-accessibility', '--window-position=0,0'] });
  try {
    const opened = await openFixture(chromium.browser, fixture.url + query);
    await delay(500); // AT-SPI exposes the document after its first accessibility update.
    await run_({ ...chromium, ...opened, pid: chromium.child.pid });
  } finally { await chromium.close(); await fixture.close(); await rm(root, { recursive: true, force: true }); }
}
const policy = { values: [VAULT_VALUE], targets: [], unknown: false };
const xdotool = (...args) => run('xdotool', args);

for (const [label, values, query] of [['Vault policy', [VAULT_VALUE], ''], ['no policy, fields only', [], '?novault&nomarkers']]) test(`DPR ${dpr} ${label}: desktop masks cover every protected frame region; ordinary content stays visible`, () => withDesktop(async ({ browser, connection, sessionId, pid }) => {
  for (const scroll of [0, 300]) {
    await connection.send('Runtime.evaluate', { expression: `scrollTo(0, ${scroll})` }, sessionId);
    const result = await fenceBrowserCapture(browser, { values, targets: [], unknown: false }, protection => census(pid, protection));
    assert.deepEqual(result.bound, [true], `scroll ${scroll}: window placement was not proven ${JSON.stringify(result.docs)}`);
    assert.ok(result.before[0] > 1000 * dpr * dpr, 'fixture shows protected pixels on the desktop');
    assert.equal(result.after[0], 0, `scroll ${scroll}: protected desktop pixels remain`);
    assert.ok(result.after[1] > result.before[1] * 0.9, `scroll ${scroll}: ordinary content withheld (${result.after[1]}/${result.before[1]})`);
  }
}, query));

test(`DPR ${dpr}: a window moved or resized after measurement is withheld`, () => withDesktop(async ({ browser, pid }) => {
  for (const change of [['windowmove', '%@', '37', '23'], ['windowsize', '%@', '900', '640']]) {
    const protection = await measurePresented(browser, policy);
    assert.ok(protection);
    await xdotool('search', '--onlyvisible', '--pid', String(pid), '--name', '.', ...change);
    await delay(700);
    const result = await census(pid, protection);
    assert.deepEqual(result.bound, [false], `${change[0]}: stale placement accepted`);
    assert.equal(result.after[0], 0);
    assert.equal(result.after[1], 0, 'the unbound window is withheld whole');
  }
}));

test(`DPR ${dpr}: page scrolling during every capture is captured fail-closed`, () => withDesktop(async ({ browser, connection, sessionId, pid }) => {
  let calls = 0;
  const result = await fenceBrowserCapture(browser, policy, async protection => {
    calls++;
    if (protection) await connection.send('Runtime.evaluate', { expression: 'scrollBy(0, 160)' }, sessionId);
    return { protection, ...(await census(pid, protection)) };
  });
  assert.equal(calls, 4);
  assert.equal(result.protection, null);
  assert.deepEqual(result.bound, [false]);
  assert.equal(result.after[0], 0);
}));

test(`DPR ${dpr}: docked DevTools (page data outside page markers) withholds the window`, () => withDesktop(async ({ browser, pid }) => {
  await xdotool('search', '--onlyvisible', '--pid', String(pid), '--name', '.', 'windowactivate', '--sync');
  await xdotool('key', 'F12');
  for (let i = 0; i < 50 && !(await browser.ensureConnection().then(c => c.send('Target.getTargets'))).targetInfos.some(t => t.url.startsWith('devtools://')); i++) await delay(100);
  await delay(1500);
  const result = await fenceBrowserCapture(browser, policy, async protection => ({ protection, ...(await census(pid, protection)) }));
  assert.equal(result.protection, null);
  assert.deepEqual(result.bound, [false]);
  assert.equal(result.after[0], 0);
  assert.equal(result.after[1], 0);
}));

test(`DPR ${dpr}: page zoom keeps placement exact (CSS to DIP to device pixels)`, () => withDesktop(async ({ browser, pid }) => {
  await xdotool('search', '--onlyvisible', '--pid', String(pid), '--name', '.', 'windowactivate', '--sync');
  for (let i = 0; i < 2; i++) { await xdotool('key', 'ctrl+plus'); await delay(400); }
  await delay(800);
  const result = await fenceBrowserCapture(browser, policy, async protection => ({ protection, ...(await census(pid, protection)) }));
  assert.ok(result.protection.pages[0].dpr > dpr * 1.2, 'page zoom applied');
  assert.deepEqual(result.bound, [true]);
  assert.equal(result.after[0], 0);
  assert.ok(result.after[1] > result.before[1] * 0.9);
}));

test(`DPR ${dpr}: emulated page density (host display mode) withholds the window`, () => withDesktop(async ({ browser, connection, sessionId, pid }) => {
  // kernel-browser-geometry displayDeviceMetrics: CSS viewport kept, density and view scale doubled. The
  // screen then shows the page magnified while AT-SPI reports the unscaled view: no provable mapping.
  const { result: size } = await connection.send('Runtime.evaluate', { returnByValue: true, expression: '[innerWidth, innerHeight]' }, sessionId);
  await connection.send('Emulation.setDeviceMetricsOverride', { width: size.value[0], height: size.value[1], deviceScaleFactor: 2 * dpr, scale: 2 * dpr, mobile: false }, sessionId);
  await delay(800);
  const result = await fenceBrowserCapture(browser, policy, async protection => ({ protection, ...(await census(pid, protection)) }));
  assert.equal(result.protection.pages[0].dpr, 2 * dpr);
  assert.deepEqual(result.bound, [false]);
  assert.equal(result.after[0], 0);
  assert.equal(result.after[1], 0);
}));
