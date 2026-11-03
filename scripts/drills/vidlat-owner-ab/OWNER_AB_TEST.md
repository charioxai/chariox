# MP-08 / MP-10 — owner hands-on A/B (20 minutes)

Chariox: [Open the disposable-account waiting room](https://pr972-vidlatab.val.51-255-87-178.sslip.io/waiting-room).
Selkies: [Open the gated baseline](https://pr972-vidlatab-selkies.val.51-255-87-178.sslip.io/).

**One-step sign-in:** open the Chariox link. This private validation stack
signs you in automatically as `pr972-vidlatab@chariox.invalid`, a disposable
test account. No email, password, approval code or existing account is needed.
The coordinator enrolls the quiet host beforehand. Both links become active
when the coordinator starts the test; use your admitted home network
(46.128.9.75). If your IP changed, tell the coordinator.

Both remote Chromiums run side by side on one quiet Ubuntu 26.04 CCX43.
Use your Mac browser and Retina display with browser zoom at 100%.

1. **Minutes 0–3:** open Chariox, select Machine **vidlat-owner-ab drill host**,
   Kernel **vidlat-owner-ab**, then session **owner-ab** under **Open Existing
   Sessions**. Open **Browser**, enter the Wikipedia URL below and choose
   **New tab**. Select **Video** if offered.
2. **Minutes 3–4:** open Selkies in a second tab. It starts with the same
   Wikipedia article. Both sources use DPR 2 and a 2560×1600 display. Keep
   viewer zoom at 100%. Selkies includes the remote Chrome toolbar and borders,
   so compare the same text and actions, allowing for different line wraps.
3. **Minutes 4–16:** spend about two minutes per site, alternating which side
   goes first. In Chariox use **Browser address** then **Go**. In Selkies click
   its remote address bar, or focus its surface and press **Control+L**, enter
   the identical URL and press Enter. Dismiss banners identically. Click a
   visible link/menu, type in a search field, scroll slowly then quickly for
   20 seconds, and select small text on each side. Navigate back to the listed
   page before switching sides.
4. **Minutes 16–19:** enter 1–5 ratings below. Note freezes, missing pixels,
   accidental clicks or CAPTCHAs. Use public content only; enter no personal
   information or credentials into this disposable environment.
5. **Minute 20:** tell the coordinator you are finished. The coordinator
   stops both pipelines and deletes the quiet host and disposable kernel state.

Use these exact pages on both sides:

- [Wikipedia article](https://en.wikipedia.org/wiki/Web_browser)
- [GitHub repository](https://github.com/charioxai/chariox)
- [BBC](https://www.bbc.com/news)
- [MDN](https://developer.mozilla.org/en-US/docs/Web/JavaScript)
- [GitLab docs](https://docs.gitlab.com/)

Optional portal/search: https://www.wikipedia.org/ and a Google search for
`browser rendering`. Record any external CAPTCHA separately.

Rate each property **1 = poor, 3 = usable, 5 = excellent**.

| Site | Side | Click response | Typing echo | Scroll smoothness | Text sharpness | Notes |
|---|---|---|---|---|---|---|
| Wikipedia | Chariox | | | | | |
| Wikipedia | Selkies | | | | | |
| GitHub | Chariox | | | | | |
| GitHub | Selkies | | | | | |
| BBC | Chariox | | | | | |
| BBC | Selkies | | | | | |
| MDN | Chariox | | | | | |
| MDN | Selkies | | | | | |
| GitLab docs | Chariox | | | | | |
| GitLab docs | Selkies | | | | | |

MP-08/MP-10 qualification: Chariox uses the frozen native OpenH264 video
build (OSS `ae4ffec402`, Cloud `1921b7cc05`), including the vidsink overlay fix.
Selkies is the **qualified wheel baseline**: frozen `3f87241fcd6ab` plus
pixelflux `6974a8c16a14` GPL wheel, CPU H.264/libx264, 60 fps, 8 Mbit/s.
This is Miguel’s requested subjective comparison. Strict latency/FPS/CPU
acceptance remains FAIL; this guide does not claim provider/Vault or soak
acceptance. OpenH264 stays and optimization remains paused. MP-11 kernel
ownership, private admission and guarded cleanup apply.
