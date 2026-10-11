# MP-08 / MP-10 — owner hands-on A/B

Chariox: [Open the waiting room](https://pr972-vidlatr8ab.val.51-255-87-178.sslip.io/waiting-room).
Selkies: [Open the gated baseline](https://pr972-vidlatr8ab-selkies.val.51-255-87-178.sslip.io/).

The coordinator activates these links only when the quiet CCX43 is rented.
Both remote Chromiums run on that one Ubuntu 26.04 host. Use your Mac browser,
Retina display and normal home network; leave browser zoom at 100%. Only one
side should be actively scrolling at a time. Your sign-in uses your own email
through the normal product login. There is no shared password or secret URL.

1. Sign in to Chariox, then approve the coordinator's normal device enrollment
   for **vidlat-owner-ab CCX43** in the same account. Select that Machine and
   Kernel in the waiting room, then click **owner-ab** under **Open Existing Sessions**.
   Open the **Browser** panel and **New tab** so this browser context owns
   its tab. Choose **Video** if a mode selector appears.
2. Open the Selkies link in a second tab. It starts with the same Wikipedia
   article. Both sources use DPR 2 and a 2560×1600 display. Keep viewer zoom at 100%.
   Selkies includes Chrome’s toolbar and window borders, leaving a smaller
   page viewport (1272×709 in rehearsal); Chariox’s page is 1280×800. Compare
   the same text and actions rather than matching line wraps.
3. In Chariox's **Browser address**, open one site below. In Selkies, focus the
   remote surface, press **Control+L** for the remote Linux Chrome,
   or click its visible address bar, type the exact same URL and press Enter. Dismiss each site's banner identically. Alternate
   which side you try first on each site.
4. On each side, click a visible link/menu, type into search (do not submit
   personal information), and scroll slowly, then quickly, for 20 seconds.
   Compare click response, typing echo, scroll smoothness and small-text
   sharpness. Repeat on the next site; try browser text selection as well.
5. Fill the table, including freezes, missing pixels or accidental clicks.
   Leave one side open for a longer normal browsing session if useful. Tell
   the coordinator when finished so both runtimes and the rented host can be
   removed. Your ratings decide whether further improvement is worthwhile.

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
