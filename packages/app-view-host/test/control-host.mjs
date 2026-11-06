// Test-only builds remove the named boundary before bundling. Production has
// no runtime switch that can weaken the host or publisher document.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { renderHostDocument } from '../document.mjs';

export async function controlHost(build, boundary) {
  const removePolicy = ['csp', 'all'].includes(boundary);
  const widenSandbox = ['sandbox', 'all'].includes(boundary);
  const removeBootstrap = boundary === 'all';
  const runtime = await build({
    entryPoints: [fileURLToPath(new URL('../host.mjs', import.meta.url))],
    write: false, bundle: true, format: 'iife', platform: 'browser', target: 'es2022',
    plugins: [{ name: 'av1-boundary-control', setup(builder) {
      builder.onLoad({ filter: /(?:document|bootstrap|host)\.mjs$/ }, async ({ path }) => {
        let contents = await readFile(path, 'utf8');
        if (path.endsWith('/document.mjs')) {
          if (removePolicy) contents = contents.replace(/^  const csp = .+;$/m, '  const csp = "";')
            .replaceAll('<meta http-equiv="x-dns-prefetch-control" content="off">', '');
          if (widenSandbox) contents = contents.replace("'allow-scripts'", "'allow-scripts allow-same-origin allow-forms allow-popups allow-top-navigation'");
          if (boundary === 'wasm') contents = contents.replaceAll("'wasm-unsafe-eval'", '');
        } else if (path.endsWith('/bootstrap.mjs')) {
          if (removeBootstrap) contents = contents.replace(/export function bootstrap\(\) \{[\s\S]*?\n\}\n/, 'export function bootstrap() {}\n');
          if (boundary === 'bootstrap') contents = contents.replace("'RTCPeerConnection', 'webkitRTCPeerConnection', 'WebTransport', ", '');
        } else if (boundary === 'source') {
          contents = contents.replace('event.source !== parent || ', '');
        }
        return { contents, loader: 'js' };
      });
    } }],
  });
  let html = renderHostDocument(runtime.outputFiles[0].text);
  if (removePolicy) html = html.replace(/<meta http-equiv="Content-Security-Policy"[^>]*>/, '')
    .replace('<meta http-equiv="x-dns-prefetch-control" content="off">', '');
  if (boundary === 'wasm') html = html.replaceAll("'wasm-unsafe-eval'", '');
  assert(html.includes('chariox.app-host.attach'), 'control still runs the real verifier and host');
  return html;
}
