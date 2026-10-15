# MP-08 / MP-10 / MP-11: bounded browser keyboard drill

Use the real built web client, kernel and hosted WSS relay. Record their
commits and binary hashes. Run the same steps before and after the change,
at viewer DPR 1 and 2, with the campaign network conditions. Keep evidence
and disposable runtime state outside the checkout.

1. Open the Browser panel and navigate to `https://www.wikipedia.org/`.
2. Click the public search field, type `chariox`, and capture the visible seed.
3. Press Control+A (Meta+A on macOS), then Backspace. Capture the cleared field.
   The expected value is empty; a remaining `chario` fails the drill.
4. Repeat through an official provider turn using the shared browser tools:
   take a fresh snapshot, resolve the public input, type the same seed, send
   key `a` with modifiers 2 (Control) or 4 (Meta), then unmodified Backspace.
   Take another fresh snapshot and verify the input's accessibility value.
   If the provider saves a large result, use its official result-file reader
   on that public snapshot only. A truncated result that prevents verification
   fails the provider cell even when the field appears cleared.
5. Verify unmodified text, Shift+Tab, and trusted paste still work. Modified
   key events must not insert text. A client below protocol 498 must refuse
   modifier-bearing keys rather than silently dropping the modifiers.

These steps exercise local protocol 498 / relay peer 107. Regression checks
must also cover out-of-range modifiers, stale documents, cancelled input,
protected focus, and paired key release. Never use internal IPC to perform
the user-visible actions or replace the real provider with a fixture.

A passing keyboard cell does not close the display gate. Record per-site
capture and mirror coverage, click latency, scroll frame rate, and stability
against the MP-08 / MP-10 campaign thresholds separately. No CAPTCHA bypass
is permitted; record an excluded Google cell when directed by the owner.
