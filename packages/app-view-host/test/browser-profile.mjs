import { mkdtemp, rm, mkdir, writeFile, readFile, readdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { clientViewChromiumPreferences } from '../chromium-launch.mjs';
import { join } from 'node:path';

// Incognito Chromium suppresses explicit preconnect and DNS-prefetch, making
// their positive controls vacuous. Use a fresh ordinary profile, then remove it.
export async function browserProfile(chromium, options) {
  if (Buffer.byteLength(tmpdir()) > 60) throw new Error('Use a TMPDIR shorter than 61 bytes for Chromium Unix sockets');
  const directory = await mkdtemp(join(tmpdir(), 'av1-browser-'));
  const { captureDirectory, clientView, ...launch } = options;
  if (clientView) {
    await mkdir(join(directory, 'Default'));
    await writeFile(join(directory, 'Default/Preferences'), JSON.stringify(clientViewChromiumPreferences));
  }
  let context, cdp, events = [], samples = [];
  if (captureDirectory) {
    await mkdir(captureDirectory, { recursive: true });
    const quote = value => "'" + value.replaceAll("'", "'\\''") + "'";
    const wrapper = join(directory, 'trace-chrome');
    await writeFile(wrapper, '#!/bin/sh\nexec /usr/bin/strace -ff -ttt -yy -s 256 -e trace=network,process,close -o '
      + quote(join(captureDirectory, 'syscall')) + ' ' + quote(launch.executablePath) + ' "$@"\n', { mode: 0o700 });
    launch.executablePath = wrapper;
  }
  try { context = await chromium.launchPersistentContext(directory, launch); }
  catch (error) { await rm(directory, { recursive: true }); throw error; }
  async function snapshot(label) {
    if (!cdp) return;
    const { processInfo } = await cdp.send('SystemInfo.getProcessInfo');
    const processes = [];
    for (const info of processInfo) {
      const pid = Number(info.id);
      try {
        const command = (await readFile(`/proc/${pid}/cmdline`, 'utf8')).split('\0').filter(Boolean);
        const tasks = (await readdir(`/proc/${pid}/task`)).map(Number);
        processes.push({ ...info, pid, command, tasks });
      } catch (error) { processes.push({ ...info, pid, exited: error.code === 'ENOENT' }); }
    }
    const frames = [];
    for (const page of context.browser().contexts().flatMap(context => context.pages())) for (const frame of page.frames()) {
      const marker = `av1-frame-${samples.length}-${frames.length}`;
      try {
        // Native User Timing events carry the actual renderer PID, including
        // opaque out-of-process frames. A URL or Target id alone does not.
        await frame.evaluate(marker => { performance.mark(marker); }, marker);
        frames.push({ url: frame.url(), parentUrl: frame.parentFrame()?.url() ?? null, marker });
      } catch (error) { frames.push({ url: frame.url(), error: error.message }); }
    }
    samples.push({ label, time: Date.now(), processes, frames });
  }
  try {
    if (captureDirectory) {
      cdp = await context.browser().newBrowserCDPSession();
      cdp.on('Tracing.dataCollected', ({ value }) => events.push(...value));
      await cdp.send('Tracing.start', { categories: 'devtools.timeline,disabled-by-default-devtools.timeline,disabled-by-default-devtools.timeline.frame,blink.user_timing', transferMode: 'ReportEvents' });
    }
    await snapshot('launched');
  } catch (error) {
    try { await context.close(); } finally { await rm(directory, { recursive: true }); }
    throw error;
  }
  return { context, snapshot, async close() {
    let traceError;
    try {
      if (cdp) {
        await snapshot('before-close');
        const complete = new Promise(resolve => cdp.once('Tracing.tracingComplete', resolve));
        await cdp.send('Tracing.end'); await complete;
      }
    } catch (error) { traceError = error; }
    try { await context.close(); } finally {
      try {
        if (captureDirectory) {
          const prefs = JSON.parse(await readFile(join(directory, 'Default/Preferences'), 'utf8'));
          await writeFile(join(captureDirectory, 'launch.json'), JSON.stringify({ requestedArgs: options.args,
            dedicated: clientView === true, effectivePreferences: {
              networkPrediction: prefs.net?.network_prediction_options,
              safeBrowsing: prefs.safebrowsing?.enabled, enhancedSafeBrowsing: prefs.safebrowsing?.enhanced,
              searchSuggestions: prefs.search?.suggest_enabled, dnsOverHttpsMode: prefs.dns_over_https?.mode,
            } }, null, 2));
          for (const sample of samples) for (const frame of sample.frames) {
            frame.rendererPids = [...new Set(events.filter(event => event.name === frame.marker).map(event => event.pid))];
          }
          await writeFile(join(captureDirectory, 'pid-map.json'), JSON.stringify(samples, null, 2));
          await writeFile(join(captureDirectory, 'frame-processes.json'), JSON.stringify(events.filter(event =>
            /frame|process_name|thread_name/i.test(event.name)), null, 2));
        }
      } finally { await rm(directory, { recursive: true }); }
    }
    if (traceError) throw traceError;
  } };
}
