# MP-08 / MP-10 — owner hands-on A/B

Chariox: [Open the waiting room](https://staging.chariox.com/waiting-room).
Selkies: [Open the gated baseline](https://pr972-vidlatr9stage-selkies.val.51-255-87-178.sslip.io/).

**MP-08/MP-10 status: BLOCKED; do not rent or issue this guide yet.** The
named staging Cloud `7af6207c7` has no native browser-video viewer. Its OSS
`f57ac8f7c` uses protocol 490 / peer 100; the frozen A/B runtime uses 498 / 107.
The coordinator must supply a reviewed, compatible staging viewer/runtime and
its supported machine API access before these steps can work. Rebuilding
staging main alone does not supply the requested video path.

This is the intended **20-minute** guide once compatibility passes. Chariox
uses your existing staging account; no validation-origin sign-up or identity
provider change is required. The coordinator activates the Selkies link only
when the quiet CCX43 is rented. Both remote Chromiums run on that one Ubuntu
26.04 host. Use your Mac browser, Retina display and normal home network,
with browser zoom at 100%. Device approval is the only enrollment step you
perform; the coordinator handles setup and cleanup.

1. **Minutes 0–3:** open Chariox in your usual signed-in staging browser.
   Approve the coordinator's one product device request for
   **vidlat-owner-ab CCX43**. The coordinator supplies the activation link
   privately; do not paste its device code into ratings or screenshots.
   Select **vidlat-owner-ab CCX43**, kernel **vidlat-owner-ab**, then
   **owner-ab** under **Open Existing Sessions**. Open **Browser**, choose
   **New tab**, and select **Video** if offered.
2. **Minutes 3–4:** open Selkies in a second browser tab. It starts with the
   same Wikipedia article. Both sources use DPR 2 and a 2560×1600 display.
   Keep viewer zoom at 100%. Selkies includes Chrome's toolbar and window
   borders, so its page viewport is slightly smaller; compare the same text
   and actions rather than line wraps. Scroll only one side at a time.
3. **Minutes 4–16:** spend about two minutes per site below, alternating which
   side goes first. Use Chariox's **Browser address**. In Selkies click the
   remote address bar (or focus the surface and press **Control+L**), enter
   the identical URL and press Enter. Dismiss each site's banner identically.
   Click a visible link/menu, type into search, and scroll slowly then quickly
   for 20 seconds on each side. Try selecting small text.
4. **Minutes 16–19:** rate click response, typing echo, scroll smoothness and
   text sharpness from **1 to 5** in the table. Note freezes, missing pixels,
   accidental clicks or a site's CAPTCHA. No personal information is needed.
5. **Minute 20:** tell the coordinator you are finished. The coordinator runs
   **ab-down** and deletes the quiet host and its separate kernel state.
   Your Omarchy kernel and existing staging login are unaffected.

Try these exact pages on both sides:

- [Wikipedia article](https://en.wikipedia.org/wiki/Web_browser)
- [GitHub repository](https://github.com/charioxai/chariox)
- [BBC](https://www.bbc.com/news)
- [MDN](https://developer.mozilla.org/en-US/docs/Web/JavaScript)
- [GitLab docs](https://docs.gitlab.com/)

Optional search/portal comparisons: https://www.wikipedia.org/ and a Google
search for `browser rendering`. Record any external CAPTCHA separately.

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

MP-08/MP-10 qualification: Chariox uses its native OpenH264 video path with
#965, #972, the two document-cache review fixes and vidsink's overlay fix.
Selkies is the **qualified wheel baseline**, frozen Selkies `3f87241fcd6ab`
plus official pixelflux `6974a8c16a14` GPL wheel (CPU H.264/libx264, 60fps,
8 Mbit/s). It is not an official container image. The existing strict latency,
FPS and CPU comparison remains FAIL; this is Miguel's requested subjective
comparison, not acceptance or a provider/Vault drill. No further encoder tuning
or GPL helper was added to Chariox. MP-11 peer gating and ordinary kernel
authority apply throughout.
