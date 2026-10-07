# Mirrored layout at protocol 454

The reference renderer uses the host's canonical 1280×800 CSS viewport. DPR is
selected before navigation and never divides CSS input coordinates. Both
clients and the host retain the native document/generation/sequence and target
checks. Layout drift still fences the affected region; its tolerance is unchanged.

Resolved CSSOM widths/heights do not preserve intrinsic sizing: turning `auto`
into pixels changes collapsed margins, flex sizing and shrink-to-fit layout.
The observer retains computed sizing through Typed OM. Native controls keep
their used dimensions while their source text is replaced by an inert baseline.
Source scrollbar gutters are retained when projected content does not overflow.
Deferred auto-height blocks carry a text-free anonymous flow box and their
collapsed edge margins. `wbr` retains its native inert word-break semantics.

Closed shadow roots remain opaque. Native CDP can admit the geometry of a
single block-in-inline flow surrogate, without reading text, attributes, URLs,
or serializing the closed tree. The surrogate is rechecked at every retained
frame/chunk fence. Other closed layouts continue through the ordinary drift
and protection checks. Hidden/zero-opacity descendants cannot rasterize a
surrounding public parent; becoming visible re-enables geometry checks.

Linux system fonts used by public mirrored text are identified by native CDP.
Only exact PostScript-name matches under `/usr/share/fonts/` and
`/usr/local/share/fonts/` are admitted through Fontconfig (`fc-match` required).
Symlinks outside those roots, unknown names, oversized/non-sfnt fonts and
protected-policy reads are refused. Fonts travel as bounded, content-addressed
WOFF resources with private CSS aliases, within the existing resource budgets.
No profile path, page URL, arbitrary file, secret text or credential is read.
The fixed system-font roots must contain redistributable fonts approved by the
operator. Web fonts retain the existing loader-bound Chromium resource path.
Other hosts or unreadable fonts retain the ordinary protected drift fallback.

WOFF packaging follows the [W3C WOFF 1.0 specification](https://www.w3.org/TR/WOFF/):
glyph/advance tables are preserved and tables compressed independently, with
no private metadata block or subsetting. Public native Canvas font metrics freeze
size-specific ascent/descent in the standard horizontal/OS2 font metrics;
checksums are recomputed per the [OpenType font header specification](https://learn.microsoft.com/en-us/typography/opentype/spec/head).
This prevents a second platform/DPR rounding pass without changing glyphs or
input coordinates. Variable/signed fonts are not rewritten. Browser decoder
validation still runs.

Public-site source/renderer diagnostics explain geometry, but acceptance also
requires the real built waiting-room client, release kernel, relay and physical
pointer/keyboard events at DPR 1 and 2. Hosted performance remains a separate
coordinator gate; a source-only diagnostic never establishes it.
