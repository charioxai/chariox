# MD-DISPLAY-02/04: structure-mirroring research only

`dom-mirror-source.js` is the exact isolated-world serializer used in the Phase7
DOM/hybrid fixture measurements. It is not imported by the product. It does not
implement the kernel Vault barrier, actor admission or relay encryption. Do not
use it on authenticated pages. Password/OTP/private fixture values are omitted;
that is narrower than production observation protection.

The executed full replay package, receipts, images and file-SHA inventory are
external at `display/phase7/dom-hybrid/prototype-source/` under the lane evidence
root; no browser/account state is retained there. From the lane checkout:

```sh
MD_TOOLS=/absolute/public-node-tools MD_OUTPUT=/absolute/external/evidence \
MD_MODES=dom,hybrid MD_PAGES=docs,spa,form,media,iframe \
node /absolute/prototype-source/run.mjs
```

Public dependencies: Node22, playwright-core, ws, pngjs, installed sandbox-capable
Chrome and Xvfb. The package launches disposable headed source/client browsers,
uses loopback plaintext WebSocket and input replay through fixture CDP, renders
sanitized records in a scriptless sandbox with CSP network/navigation/form
restrictions, and cleans only its exact owned resources. It bypasses production
kernel/relay admission. Switching data additionally uses `MD_SWITCH=1`; the
bound run files and actual command are retained in the receipt package.

Snapshot records omit script/style/network URL attributes and unsafe CSS
functions. Media/canvas/foreign frames remain opaque and use fixture video
regions in hybrid mode. Logical mirrored counters acknowledge DOM state, not
pixel-perfect source rendering. Full screenshot PSNR exposes layout drift. See
`docs/MULTIDOMAIN_DISPLAY_PERFORMANCE.md` for measured scope and security work
required before any production mirroring decision.
