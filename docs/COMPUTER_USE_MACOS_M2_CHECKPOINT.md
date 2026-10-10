# M2 native encrypted viewer checkpoint

`cu/macos-m2` merges frozen display #950 at
`f04b4006b3b8b1b833560022128b0345db873e5a` into the published M2 source. Frozen
Cloud#325 `536bb5b1a` is the viewer reference and was not modified. Native display
adds public Computer subscribe/next/close operations, so the allocated current
versions are local 497 / relay peer 106. `docs/PROTOCOL.md` lists every wire
change. Browser minima, relay admission floor and binary negotiation stay fixed.

The kernel admits a terminal caller and its current native Desktop string epoch,
then supplies the private attachment owner. Registration uses the existing relay
display envelope with `native:<epoch>` and verifies that exact private attachment.
Native next credits use the existing encrypted CXD1 transient event path. Frame
bytes leave the command response before history/replay. The shared presenter
renders native PNG/damage through its existing decoder, validates exact bound
geometry and repairs by closing and subscribing afresh. Native delivery has one
outstanding pull credit; M2 is an observer and input remains separately bounded.
The feature stays off unless `CHARIOX_MACOS_COMPUTER_DISPLAY=1` is set. Raw local
and agent display requests refuse.

Each helper source binds the signed public fixture PID/window, pairing epoch,
Quartz origin, native pixel dimensions and 1x/2x scale. SCK captures only that
window without cursor/audio/microphone. Registry protection masks the whole
window and unknown or failed secure AX classification withholds capture. Masks
precede encoding, damage comparison and cache retention. Real-app capture stays
refused until M4. Stream metadata, policy, geometry, authority, lease and source
revision are checked at final publication; revoked or closed callbacks cannot
publish retained pixels.

Two subscriptions retain at most one protected 16 MiB RGBA base each with a
60-second idle lease. Clipped 64-pixel tiles preserve exact pixels. More than
260 tiles or 32 KiB of encoded damage becomes a full PNG. PNGs cap at 512 KiB
and serialized packets at 700 KiB inside the existing private channel limit.
These bounds do not establish measured memory/performance acceptance.

R8 review fixes cover #920 @bc53791b1 foreground/pointer takeover in the fence
runner, #923 @8fc71e560 unconfirmed release when cancellation closes the socket,
and #957 @b5300f01f unsupported typing plus permissive cancellation acceptance.
The caret drill now uses the actual admitted AX text click path. Only structured
`browser_action_cancelled` passes its in-flight cancellation oracle. A key-down
without authenticated release confirmation latches owner-reset-required before
Stop/Start can pair a new helper. Focus loss, replaced windows or human pointer
movement stop global runner warps and prevent restoration over owner activity.

Component validation includes six sequential focused Swift checks, actual Mac
adapter Rust tests, native DTO snapshot execution, shared-client typechecking
and encrypted CXD1 tests, CLI typechecking, native presenter/host regressions,
Python runner tests and independent public-PNG damage/repair reconstruction.
The merged kernel and its test sources compile. Whole-kernel test-binary
codegen hit the configured memory guard and stopped before test execution;
no whole-kernel test PASS is claimed. Fail-first logs preserve the native geometry,
metadata, attachment, cancellation oracle and uncertain-release regressions.
Pointer takeover coverage is headless component evidence.

No r8 helper was rebuilt, signed or launched. The retained r6 helper remains
SHA-256 `b02c5ba5aa6619698ae045a7fff4f4403b7fcf79ff6f09a981143a6ff8282028` and
awaits the pending morning re-grant. `apps/macos-computer-use/OWNER_RECHECK.md`
separates its standalone historical replay from the next combined build/grant
and unattended strict seat plus encrypted-viewer suites. The old r6 kernel
cannot pass the new explicit cancellation oracle or implement protocol 497.

M2 remains BLOCKED on the next matching grant, native Cloud adapter integration,
hosted encrypted Web/local-TUI/remote-TUI routes and real app/site/provider plus
loss/reconnect/sleep/lock acceptance. M3 physical holds/human observation/fatal
release, M4 AX/OCR/private coverage and M5 codec/Developer ID/notarized release
remain pending. The feature is not ready for owner staging. No mock or synthetic
receipt promotes an acceptance row.

Rollback removes explicit native display/helper/scope configuration. Preserve
durable homes, keys/backups, profiles and reviewer/relay infrastructure. All build
outputs stay under `/Users/miguel/.chariox/dev/cumac/build`; Swift is sequential
at `-j2`, Rust checks/harnesses at `-j1`. No push or CI occurred.
