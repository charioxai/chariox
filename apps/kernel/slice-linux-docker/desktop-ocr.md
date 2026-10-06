# Desktop text recognition

Text reading and text lookup share `slice-text-finder.py --image`. The helper
combines Tesseract's automatic page segmentation with a block pass. Automatic
segmentation alone can find a taskbar while completely missing readable text
inside a small window on a dark desktop.

MP-08/MP-10/MP-11: lookup normalizes query and recognized words to Unicode NFC
before case folding, preserving the recognized text and original pixel boxes.
Segmentation results coalesce only when their normalized labels agree and their
boxes have at least 75% intersection over union. A broad substring hit must not
swallow a different label or a substantially shifted occurrence.

The helper uses installed supported Tesseract language models, up to eight per
observation. The image includes English and German. Additional supported models
are French, Spanish, Italian, Portuguese, Russian, Ukrainian, simplified and
traditional Chinese, Japanese, Korean, Arabic and Hindi; install the corresponding
Tesseract packages in the Environment to enable them. OSD/script models are not
text languages. More than eight supported installed models fails explicitly
rather than silently dropping a language. Model enumeration has a two-second
bound; each recognition pass retains its fifteen-second bound. Model availability
and OCR accuracy on other scripts remain distinct from normalized query matching.

Both passes use the original image. Lookup coordinates remain desktop pixels;
there is no crop, scale, window movement, or focus change. Overlapping detections
of the same target are coalesced, while separate occurrences remain separate.
Text reading removes duplicate lines. Recognition remains approximate, not a
guarantee of transcription accuracy or complete reading-order reconstruction.

TSV stays in memory per invocation, so concurrent lookups do not share a scratch
file. Each subprocess has a 15-second timeout. The older QUERY TSV entry point
remains available for saved-launcher compatibility during support refresh.

The focused tests exercise the public lookup command with controlled OCR output.
The live desktop-settings drill checks real editor rendering, text lookup,
original-pixel coordinates and non-duplicated targets on startup, restart and
saved-home restoration. Evidence images remain outside Git.
