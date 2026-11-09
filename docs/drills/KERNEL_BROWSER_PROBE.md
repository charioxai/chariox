# Kernel browser identity probe (MP-08 / MP-10 / MP-11)

Sites such as Google sign-in, Cloudflare, reCAPTCHA and Turnstile refuse a browser that shows automation ("This browser or
app may not be secure"). The kernel browser must look like the Chrome a person runs, for every web service.

Rule: no launch flag and no injected script may change what a page can observe unless a test lists it. The launch flags and
what pages see of each are in `launchFlagVisibility` (`kernel-browser-process.mjs`); a flag missing there fails
`kernel-browser-process.test.mjs`.

## What CI asserts (real Chromium)

`kernel-browser-standard-chrome.browser-test.mjs` starts the real `KernelBrowserHost` and a stock Chrome of the same build,
reads 70 signals in the page, an iframe, a dedicated worker and a service worker (`kernel-browser-signals.mjs`), compares the
request headers, and fails on any difference that is not in `knownDifferences` (each with its reason). It runs the kernel headed
(Linux desktop) and headless (the Mac launch). A leak that is fixed must leave the list: the test also fails when a listed leak
no longer differs.

```sh
CHARIOX_MDACCESS_DRILL_ROOT=<disposable dir> CHARIOX_KERNEL_BROWSER_EXECUTABLE=<Chrome binary> \
  node --test apps/kernel/slice-linux-docker/docker/kernel-browser-standard-chrome.browser-test.mjs
```

## Manual probe before every staging

```sh
node apps/kernel/slice-linux-docker/docker/kernel-browser-probe.mjs --executable <the Chrome build the kernel runs> --out <dir outside the repo>
```

It runs the signal comparison and the public checks (Google email step with a throwaway address, BrowserScan, bot.sannysoft.com,
Intoli, Cloudflare Turnstile demo, reCAPTCHA v3 demo, a Cloudflare-protected page, GitHub / Microsoft / X / Apple sign-in pages up
to the first field, Stripe checkout demo) in a stock headed Chrome and in the kernel browser (headed and headless; on a Mac:
`--arms stock:headful,kernel:headless`), and prints one row per check and kernel arm. PASS means the kernel outcome equals the stock outcome on
that network. Exit code 1 on any FAIL; Google "rejected" is always a FAIL; "captcha" is the network, not the browser (WARN).
Use `--checks signals,browserscan` to run part. Screenshots land in `--out`.

Google: one email attempt per arm, address `chariox.browsercheck.nobody.847261@gmail.com` (does not exist). Never a real account,
never a password. Repeated runs from one network make Google show a CAPTCHA; wait before repeating.

## Known residual differences (listed in `knownDifferences`)

- Headless only: `--user-agent` makes Chrome drop the high-entropy client hints (`getHighEntropyValues` architecture, bitness,
  full version list are empty; the `Sec-CH-UA-Full-Version-List/Arch/Bitness` headers are not sent).
- The permission wrappers (`browser-page-permission-shim.mjs`) leave one registered Symbol on `window`, and a same-origin
  iframe's own `Function.prototype.toString` shows the wrappers' source.
- Camera and microphone report `denied` instead of `prompt`.
- A GPU-less host: stock Chrome has no WebGL, the kernel browser renders SwiftShader.
