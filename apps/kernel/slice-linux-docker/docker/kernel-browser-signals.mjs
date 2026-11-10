// MP-08/MP-10/MP-11: what a web page can observe about the kernel browser. One probe, run in the same way against the
// kernel browser and against a stock Chrome of the same build, so a change in either shows as a difference in one table.
// Sites such as Google sign-in refuse a browser that exposes automation ("This browser or app may not be secure").

// Evaluated in the page (awaitPromise). Returns a flat JSON object; every value is a plain string, number, boolean or
// array of them so two runs compare key by key.
export const pageSignalsExpression = String.raw`(async () => {
  const out = {};
  const safe = (name, fn) => { try { const v = fn(); out[name] = v; } catch (e) { out[name] = 'ERR:' + e.name; } };
  const safeAsync = async (name, fn) => { try { out[name] = await fn(); } catch (e) { out[name] = 'ERR:' + e.name; } };
  const native = fn => typeof fn === 'function' && /\{\s*\[native code\]\s*\}/.test(Function.prototype.toString.call(fn));
  const nav = navigator;
  for (const k of ['webdriver', 'userAgent', 'appVersion', 'platform', 'vendor', 'vendorSub', 'productSub', 'language', 'cookieEnabled', 'doNotTrack', 'hardwareConcurrency', 'deviceMemory', 'maxTouchPoints', 'pdfViewerEnabled', 'onLine'])
    safe('navigator.' + k, () => nav[k] === undefined ? 'undefined' : nav[k]);
  safe('navigator.languages', () => nav.languages.join(','));
  safe('navigator.plugins', () => Array.from(nav.plugins).map(p => p.name).join(','));
  safe('navigator.mimeTypes', () => Array.from(nav.mimeTypes).map(p => p.type).join(','));
  safe('navigator.connection', () => nav.connection ? [nav.connection.effectiveType, typeof nav.connection.rtt, typeof nav.connection.downlink, nav.connection.saveData].join(',') : 'undefined');
  safe('navigator.keys', () => { const keys = []; for (const k in nav) keys.push(k); return keys.sort().join(','); });
  safe('navigator.ownKeys', () => Object.getOwnPropertyNames(nav).join(','));
  safe('navigator.proto.keys', () => Object.getOwnPropertyNames(Object.getPrototypeOf(nav)).sort().join(','));
  safe('navigator.getBattery', () => typeof nav.getBattery);
  await safeAsync('battery', async () => { const b = await nav.getBattery(); return [b.charging, b.level, b.chargingTime, b.dischargingTime].join(','); });
  const uad = nav.userAgentData;
  safe('uad.brands', () => uad.brands.map(b => b.brand + '/' + b.version).join(','));
  safe('uad.mobile', () => uad.mobile);
  safe('uad.platform', () => uad.platform);
  await safeAsync('uad.high', async () => { const h = await uad.getHighEntropyValues(['architecture', 'bitness', 'model', 'platformVersion', 'uaFullVersion', 'fullVersionList', 'wow64', 'formFactors']);
    return JSON.stringify({architecture: h.architecture, bitness: h.bitness, model: h.model, wow64: h.wow64, fullVersionList: (h.fullVersionList || []).map(b => b.brand + '/' + b.version), uaFullVersion: h.uaFullVersion, platformVersion: typeof h.platformVersion, formFactors: h.formFactors}); });
  safe('window.chrome', () => typeof window.chrome);
  safe('window.chrome.keys', () => window.chrome ? Object.getOwnPropertyNames(window.chrome).sort().join(',') : 'none');
  safe('window.chrome.runtime', () => typeof (window.chrome && window.chrome.runtime));
  safe('window.chrome.loadTimes', () => typeof (window.chrome && window.chrome.loadTimes));
  safe('window.chrome.csi', () => typeof (window.chrome && window.chrome.csi));
  safe('window.chrome.app', () => typeof (window.chrome && window.chrome.app));
  safe('window.chrome.loadTimes.native', () => native(window.chrome.loadTimes));
  safe('window.chrome.csi.native', () => native(window.chrome.csi));
  safe('loadTimes.keys', () => Object.keys(window.chrome.loadTimes()).sort().join(','));
  safe('screen', () => [screen.width, screen.height, screen.availWidth, screen.availHeight, screen.availLeft, screen.availTop, screen.colorDepth, screen.pixelDepth].join('x'));
  safe('window.size', () => [innerWidth, innerHeight, outerWidth, outerHeight, screenX, screenY].join('x'));
  safe('window.chromeBand', () => [outerWidth - innerWidth, outerHeight - innerHeight].join('x'));
  safe('devicePixelRatio', () => devicePixelRatio);
  safe('visualViewport', () => [visualViewport.width, visualViewport.height, visualViewport.scale].join('x'));
  safe('screen.orientation', () => screen.orientation.type + '/' + screen.orientation.angle);
  safe('media.pointer', () => ['(pointer: fine)', '(hover: hover)', '(any-pointer: fine)', '(prefers-color-scheme: light)', '(prefers-reduced-motion: no-preference)'].map(q => matchMedia(q).matches).join(','));
  safe('document.hasFocus', () => document.hasFocus());
  safe('document.visibility', () => document.visibilityState + '/' + document.hidden);
  safe('touch', () => ['ontouchstart' in window, typeof TouchEvent, typeof Touch].join(','));
  safe('Notification.permission', () => Notification.permission);
  await safeAsync('permissions', async () => { const r = []; for (const name of ['notifications', 'geolocation', 'camera', 'microphone', 'clipboard-read', 'clipboard-write', 'midi', 'persistent-storage']) { try { r.push(name + ':' + (await nav.permissions.query({name})).state); } catch (e) { r.push(name + ':' + e.name); } } return r.join(','); });
  safe('Intl', () => [Intl.DateTimeFormat().resolvedOptions().timeZone, Intl.DateTimeFormat().resolvedOptions().locale, new Date().getTimezoneOffset()].join(','));
  safe('speechSynthesis', () => typeof speechSynthesis + ':' + speechSynthesis.getVoices().length);
  // media codecs
  safe('codecs', () => { const v = document.createElement('video'); return ['video/mp4; codecs="avc1.42E01E"', 'video/mp4; codecs="avc1.64001F, mp4a.40.2"', 'video/webm; codecs="vp8"', 'video/webm; codecs="vp9"', 'video/mp4; codecs="hev1.1.6.L93.B0"', 'audio/mpeg', 'audio/mp4; codecs="mp4a.40.2"', 'audio/ogg; codecs="vorbis"', 'audio/webm; codecs="opus"'].map(t => v.canPlayType(t) || '-').join('|'); });
  // webgl and canvas
  const gl = () => { const c = document.createElement('canvas'); return c.getContext('webgl') || c.getContext('experimental-webgl'); };
  safe('webgl', () => { const g = gl(); if (!g) return 'none'; const e = g.getExtension('WEBGL_debug_renderer_info'); return [g.getParameter(g.VENDOR), g.getParameter(g.RENDERER), e ? g.getParameter(e.UNMASKED_VENDOR_WEBGL) : '-', e ? g.getParameter(e.UNMASKED_RENDERER_WEBGL) : '-'].join(' | '); });
  safe('webgl.extensions', () => { const g = gl(); return g ? g.getSupportedExtensions().length : -1; });
  const canvasHash = () => { const c = document.createElement('canvas'); c.width = 220; c.height = 40; const x = c.getContext('2d'); x.textBaseline = 'top'; x.font = '16px Arial'; x.fillStyle = '#f60'; x.fillRect(10, 5, 100, 20); x.fillStyle = '#069'; x.fillText('Chariox canvas probe 1', 2, 12); const s = c.toDataURL(); let h = 0; for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) | 0; return h; };
  safe('canvas.stable', () => canvasHash() === canvasHash());
  await safeAsync('audio.stable', async () => { const run = async () => { const c = new OfflineAudioContext(1, 4410, 44100); const o = c.createOscillator(); o.type = 'triangle'; o.frequency.value = 10000; const k = c.createDynamicsCompressor(); o.connect(k); k.connect(c.destination); o.start(0); const b = await c.startRendering(); let s = 0; for (const v of b.getChannelData(0)) s += Math.abs(v); return s; }; return (await run()) === (await run()); });
  // Runtime.enable tell: a console.debug of an Error reads its stack only when a CDP client has Runtime enabled
  safe('cdp.stackTell', () => { let hit = false; const e = new Error(); Object.defineProperty(e, 'stack', {get() { hit = true; return ''; }}); console.debug(e); return hit; });
  safe('cdp.prepareStackTrace', () => { let hit = false; const old = Error.prepareStackTrace; Error.prepareStackTrace = () => { hit = true; return ''; }; new Error().stack; Error.prepareStackTrace = old; return hit; });
  // natives: no page-visible function in these prototypes may be a JavaScript function in disguise
  const hosts = {nav: Object.getPrototypeOf(nav), permissions: Permissions.prototype, geolocation: Geolocation.prototype, clipboard: Clipboard.prototype, mediaDevices: MediaDevices.prototype, notification: Notification, permissionStatus: PermissionStatus.prototype, function: Function.prototype, error: Error, window: window};
  safe('natives.nonNative', () => { const bad = []; for (const [label, host] of Object.entries(hosts)) for (const k of Object.getOwnPropertyNames(host)) { let d; try { d = Object.getOwnPropertyDescriptor(host, k); } catch { continue; } for (const f of [d.value, d.get, d.set]) if (typeof f === 'function' && !native(f) && !/^class\s/.test(Function.prototype.toString.call(f))) bad.push(label + '.' + k); } return bad.join(','); });
  safe('natives.own', () => [[nav.permissions, 'query'], [nav.geolocation, 'getCurrentPosition'], [nav.clipboard, 'readText'], [nav.mediaDevices, 'getUserMedia'], [nav, 'userAgent'], [nav, 'webdriver']].filter(([o, k]) => o && Object.hasOwn(o, k)).map(([, k]) => k).join(','));
  safe('natives.descriptors', () => [Object.getOwnPropertyDescriptor(Notification, 'permission').get.name, Notification.requestPermission.name, Notification.requestPermission.length, nav.permissions.query.name, nav.permissions.query.length, 'prototype' in nav.permissions.query].join(','));
  safe('natives.toStringOfToString', () => Function.prototype.toString.call(Function.prototype.toString));
  safe('natives.queryToString', () => Function.prototype.toString.call(nav.permissions.query));
  safe('globals.odd', () => Object.getOwnPropertyNames(window).filter(n => /chariox|^__(?!proto)|webdriver|selenium|cdc_|puppeteer|playwright|\$cdc|domAutomation/i.test(n)).join(','));
  safe('document.odd', () => Object.getOwnPropertyNames(document).filter(n => /cdc_|webdriver|selenium/i.test(n)).join(','));
  safe('symbols.window', () => Object.getOwnPropertySymbols(window).map(String).join(','));
  // consistency across contexts
  await safeAsync('iframe', async () => { const f = document.createElement('iframe'); document.body.appendChild(f); const w = f.contentWindow; const r = [w.navigator.webdriver, w.navigator.userAgent === nav.userAgent, w.navigator.platform === nav.platform, typeof w.chrome, w.Function.prototype.toString.call(nav.permissions.query) === Function.prototype.toString.call(nav.permissions.query), w.navigator.languages.join(',') === nav.languages.join(',')].join(','); f.remove(); return r; });
  await safeAsync('worker', () => new Promise(resolve => { const code = 'let g=null;try{const c=new OffscreenCanvas(1,1).getContext("webgl");const e=c.getExtension("WEBGL_debug_renderer_info");g=c.getParameter(e.UNMASKED_RENDERER_WEBGL)}catch(e){g="ERR"} postMessage({ua:navigator.userAgent,platform:navigator.platform,hc:navigator.hardwareConcurrency,languages:navigator.languages.join(","),webdriver:navigator.webdriver,brands:navigator.userAgentData?navigator.userAgentData.brands.map(b=>b.brand).join(","):"none",gl:g})'; const w = new Worker(URL.createObjectURL(new Blob([code]))); w.onmessage = e => resolve(JSON.stringify(e.data)); w.onerror = () => resolve('ERR'); setTimeout(() => resolve('TIMEOUT'), 4000); }));
  safe('worker.matchesPage', () => { try { const w = JSON.parse(out.worker); return [w.ua === nav.userAgent, w.platform === nav.platform, w.hc === nav.hardwareConcurrency, w.languages === nav.languages.join(','), w.webdriver === nav.webdriver, !/Headless/.test(w.brands)].join(','); } catch { return out.worker; } });
  await safeAsync('serviceWorker', async () => {
    const registration = await nav.serviceWorker.register('/sw.js');
    const worker = registration.installing || registration.waiting || registration.active;
    const reply = new Promise(resolve => { nav.serviceWorker.onmessage = event => resolve(JSON.stringify(event.data)); setTimeout(() => resolve('TIMEOUT'), 4000); });
    await new Promise(resolve => { if (worker.state === 'activated') resolve(); else worker.onstatechange = () => worker.state === 'activated' && resolve(); setTimeout(resolve, 3000); });
    (registration.active || worker).postMessage('probe');
    const result = await reply; await registration.unregister();
    try { const w = JSON.parse(result); return [w.ua === nav.userAgent, w.platform === nav.platform, w.hc === nav.hardwareConcurrency, w.languages === nav.languages.join(','), w.webdriver === nav.webdriver].join(','); } catch { return result; }
  });
  return JSON.stringify(out);
})()`;

// What the server saw on a request from the page: the headers that carry the browser's identity.
export const identityHeaders = ['user-agent', 'accept-language', 'sec-ch-ua', 'sec-ch-ua-mobile', 'sec-ch-ua-platform', 'sec-ch-ua-full-version-list', 'sec-ch-ua-arch', 'sec-ch-ua-bitness', 'sec-ch-ua-model', 'sec-ch-ua-platform-version', 'sec-fetch-site', 'sec-fetch-mode', 'sec-fetch-dest', 'upgrade-insecure-requests', 'accept', 'accept-encoding', 'dnt'];

// Server that records identity headers and asks for the high-entropy hints, so a second request shows all of them.
export function identityServerHandler(seen) {
  return (request, response) => {
    const headers = {};
    for (const name of identityHeaders) if (request.headers[name] !== undefined) headers[name] = request.headers[name];
    seen.push({url: request.url, headers});
    if (request.url.startsWith('/sw.js')) { response.setHeader('content-type', 'text/javascript'); response.end("self.onmessage = e => e.source.postMessage({ua: navigator.userAgent, platform: navigator.platform, hc: navigator.hardwareConcurrency, languages: navigator.languages.join(','), webdriver: navigator.webdriver}); self.oninstall = () => self.skipWaiting(); self.onactivate = e => e.waitUntil(self.clients.claim());"); return; }
    if (request.url.startsWith('/echo')) { response.setHeader('content-type', 'application/json'); response.end('{}'); return; }
    response.setHeader('accept-ch', 'Sec-CH-UA-Full-Version-List, Sec-CH-UA-Arch, Sec-CH-UA-Bitness, Sec-CH-UA-Model, Sec-CH-UA-Platform-Version');
    response.setHeader('content-type', 'text/html');
    response.end('<!doctype html><title>Signals</title><p>Signals</p>');
  };
}

// Differences from a stock Chrome of the same build that the kernel browser is KNOWN to show, each with its reason.
// A new difference is a failing test; a listed one that disappears is a failing test too, so this list only shrinks.
export const knownDifferences = {
  // Geometry is the kernel's own: it sizes the window and the page for the viewer.
  'screen': 'the kernel sets the page and screen size for the viewer (Emulation.setDeviceMetricsOverride)',
  'window.size': 'the kernel sizes the window for the viewer',
  'window.chromeBand': 'the owned window has Chrome\'s tab strip and address bar (87 px), a bare X display has no window manager chrome',
  'visualViewport': 'the kernel sizes the window for the viewer',
  'document.hasFocus': 'the kernel focuses the page it drives; a bare Xvfb window has no focus',
  'permissions': 'the kernel denies camera and microphone for every page (no capture devices behind the kernel browser)',
  'symbols.window': 'KNOWN LEAK: the permission shim\'s response slot is a registered Symbol on window',
  'iframe': 'KNOWN LEAK: a same-origin iframe\'s own Function.prototype.toString shows the permission wrappers\' JavaScript source',
  'webgl': 'a GPU-less host: the kernel opts into SwiftShader (--enable-unsafe-swiftshader) where stock Chrome has no WebGL',
  'webgl.extensions': 'a GPU-less host: SwiftShader',
  'uad.high': 'KNOWN LEAK (headless only): --user-agent drops the high-entropy client hints (architecture, bitness, full version list)',
};

// On a GPU-less host the worker's WebGL renderer (its `gl` field) is SwiftShader in the kernel browser and absent in stock Chrome;
// that field alone is exempt, every other worker field is compared like any page signal.
const withoutWorkerGl = value => { try { const worker = JSON.parse(value); delete worker.gl; return JSON.stringify(worker); } catch { return value; } };

// Keys whose value differs from the stock browser and are not in the known list, with both values.
export function unexpectedDifferences(stock, kernel, { gpuless = stock.webgl === 'none' } = {}) {
  const out = [];
  const comparable = (key, value) => key === 'worker' && gpuless ? withoutWorkerGl(value) : value;
  for (const key of new Set([...Object.keys(stock), ...Object.keys(kernel)])) {
    if (comparable(key, stock[key]) === comparable(key, kernel[key])) continue;
    if (key in knownDifferences && (!/^webgl/.test(key) || gpuless)) continue;
    out.push({ key, stock: stock[key], kernel: kernel[key] });
  }
  return out;
}

// Headers a page's requests carry must be identical too (User-Agent, client hints, language).
export function unexpectedHeaderDifferences(stockSeen, kernelSeen, ignore = []) {
  const last = seen => seen.at(-1)?.headers ?? {};
  const a = last(stockSeen), b = last(kernelSeen), out = [];
  for (const key of new Set([...Object.keys(a), ...Object.keys(b)])) if (a[key] !== b[key] && !ignore.includes(key)) out.push({ key, stock: a[key], kernel: b[key] });
  return out;
}
