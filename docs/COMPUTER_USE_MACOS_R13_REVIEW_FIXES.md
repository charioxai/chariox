# R13 fixes for review #964

The newest lane review is `REVIEW_964_6101151bd.md`, with one P1 and two P2
findings. All three are fixed fail-first on `cu/macos-m5`, one commit each.
The r12 statement that #964 had no findings was incorrect and is corrected.
M0-M4 retain their heads. M2-M4 native subscriptions already omit bitrate, and
unsigned-release mode and the codec driver exist only in M5. No mirror is needed.

| Finding | Fix |
| --- | --- |
| P1 native subscribe rejected by protocol 497 | `7443c32e6` omits browser bitrate, retaining target/codecs |
| P2 unsigned policy executable killed on Apple Silicon | `d340ee593` keeps policy tests ad-hoc signed and uses unsigned linker flags only for bundle executables |
| P2 missing codec evidence parent | This commit creates the evidence directory and passes one artifact path to encoder and decoder |

## Fail-first evidence

Evidence is under
`/Users/miguel/.codex/evidence/browser-computer-use-macos/cumac-r13-20261011`.

- `subscribe-red.log` records all three actual serialized native attachment
  offers rejected by the production `KernelComputerCommand` serde definition,
  with `unknown field bitrate`. `subscribe-green.log` passes all six presenter
  tests, including default PNG, AVC and explicit browser bitrate. Adding bitrate
  back to each captured request still fails deserialization. The focused Rust
  executable imports the actual command source; text aliases only satisfy
  unrelated input variants. No whole-kernel test runs on this Mac.
- `signing-red.log` records the real policy executable killed with exit 137
  before any bundle compile. `signing-green.log` runs the real policy code,
  checks its ad-hoc signature and proves that all three inert release executables
  remain unsigned. The production build driver supplies the flags, while a
  compiler wrapper substitutes tiny inert source for bundle source. No helper
  is built or launched. Reusing the destination still fails the fresh-path guard.
- `codec-red.log` records the actual encoder test's artifact write failing with
  Cocoa error 4 and POSIX ENOENT before later regressions or decoding. The old
  driver/test paths were relocated to fresh scratch paths, preserving their
  control flow and avoiding overwrite of r11 evidence. `prepare-codec-red.py`
  reproduces that fixture from the preceding commit. `codec-green.log` runs the
  unmodified final production driver against a fresh evidence root. Actual
  VideoToolbox hardware encoding, SPS/PPS, quiet source, settled exact repair,
  PNG fallback and independent software decoding all pass.
- `node-suite.log` records 61 passing presenter/recovery/visibility and Mac
  component tests, with no skips. Shell syntax and `git diff --check` pass.

Re-run the focused serde boundary with `CUMAC_BUILD_DIR` under the approved
build root and `bash apps/browser-display/check-native-contract.sh`. Run
`build-signing.test.py` and `codec-driver.test.py` with Python, one at a time.
Set `CUMAC_BUILD_DIR` and `CUMAC_EVIDENCE_DIR` to fresh approved paths for codec
evidence. Every Swift compile uses `-j2`; the recorded executions also use the
critical-pressure and 3 GiB process-group RSS guard retained at
`/Users/miguel/.chariox/dev/cumac/build/r11/guard.py`.

## Handoff limits

OWNER_RECHECK retains one conditional matching-build appointment. These fixes
do not establish real client/relay/provider performance or a notarized release.
Matching-build/grant, real callbacks, M4 Vault/private exclusions and opaque
AX/query/language coverage remain blockers. Historical accepted rows and M0
aggregate FAIL retain their statuses, heads and dates. M2-M5 stay BLOCKED.

Local 497, relay peer 106, frozen display #950, Cloud #325 and the shared wire
contract remain unchanged. No push, CI, sudo, TCC request, Keychain access,
provider/hosted run, credential/key use, helper replacement or installed bundle
launch occurs. The retained r6 helper's executable SHA-256 remains
`b02c5ba5aa6619698ae045a7fff4f4403b7fcf79ff6f09a981143a6ff8282028` and its
CDHash remains `8b87ab7229c79a45f21eaddb7b2ef833095cf9d0`.

Inventoried scratch sources, inert release-test artifacts and compiler caches
were removed, totaling 661 files and 339,754,024 bytes. The exact inventory is
`cleanup-inventory.json`. Focused codec/serde executables, public H264 and logs
remain. Cleanup never traverses symlink targets or includes credentials,
retained bundles, shared infrastructure or evidence.
