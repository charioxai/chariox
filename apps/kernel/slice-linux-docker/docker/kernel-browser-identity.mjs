// MP-08/MP-10/MP-11: a headless Chrome announces itself: "HeadlessChrome" in the User-Agent and no pointer or hover.
// Sites such as Google sign-in refuse such a browser ("This browser or app may not be secure"). The macOS kernel always
// launches headless, so its browser must report what the headed Chrome of the same build reports.
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

// Chrome reports a reduced User-Agent: fixed platform token and a major-only version (UA reduction).
const platformTokens = { linux: 'X11; Linux x86_64', darwin: 'Macintosh; Intel Mac OS X 10_15_7' };

export function browserMajor(versionOutput) {
  const match = /(\d+)\.\d+\.\d+\.\d+/.exec(versionOutput);
  if (!match) throw new Error('MP-08: cannot read the browser version; set CHARIOX_KERNEL_BROWSER_EXECUTABLE to a Chrome or Chromium build');
  return Number(match[1]);
}

export async function readBrowserMajor(binary, environment, run = promisify(execFile)) {
  try { return browserMajor((await run(binary, ['--version'], { env: environment, timeout: 15000 })).stdout); }
  catch (error) { if (/cannot read the browser version/.test(error.message)) throw error; throw new Error('MP-08: cannot run the browser to read its version'); }
}

export function headlessIdentityArguments(platform, major) {
  const token = platformTokens[platform];
  if (!token) throw new Error(`MP-08: no headed identity for platform ${platform}`);
  return [
    `--user-agent=Mozilla/5.0 (${token}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/${major}.0.0.0 Safari/537.36`,
    // Headless has no input devices; media queries then report pointer: none and hover: none. 4 = fine, 2 = hover.
    '--blink-settings=primaryPointerType=4,availablePointerTypes=4,primaryHoverType=2,availableHoverTypes=2',
  ];
}
