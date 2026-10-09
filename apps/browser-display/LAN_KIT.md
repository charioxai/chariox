# MP-08/MP-10/MP-11: Linux LAN display kit

The kit contains the production kernel/relay focused test ELF, embedded controller,
fixtures, viewer and public dependencies. Node is the official self-contained
linux-x64 **22.20.0** binary from nodejs.org; its archive SHA256 is pinned in
`lan_node_runtime.py`. No distro libnode or external builtin files are used.
Python/PyAV and the explicit loader are bundled. Runtime identities are generated
through product paths in disposable state. The host supplies sandboxed Chromium,
Xvfb, FFmpeg, iproute2 and ethtool. No provider accounts or operator keys are used.

MP-08/MP-10: display-mode Chromium uses grayscale text antialiasing so captured
glyphs do not assume a particular viewing panel's LCD subpixel order. Glyph
outlines retain antialiasing at the negotiated native density.

From a clean committed checkout, after building both ELFs at that source with `--features native-display`:

```sh
export MD_NODE_ARCHIVE=/absolute/node-v22.20.0-linux-x64.tar.xz
export MD_NATIVE_WORKER=/absolute/chariox-kernel
export MD_KERNEL_BUILD_SOURCE=$(git rev-parse HEAD)
python3 apps/browser-display/build-lan-kit.py /absolute/kernel-tests \
  /absolute/public-node-tools /absolute/pyav-tools /absolute/evidence-output
```

Download the archive from
`https://nodejs.org/dist/v22.20.0/node-v22.20.0-linux-x64.tar.xz`.
Expected SHA256:
`00bbd05e306ea68b6e13e17360d0e2f680b493ef95f2fea1c4296ff7437530bc`.
Verify the kit tarball against the coordinator's SHA256 before extracting.
`KIT_MANIFEST.json` records source, build identity and every public file hash.

Arch/Omarchy prerequisites (coordinator has owner install authorization):

```sh
sudo pacman -S --needed chromium xorg-server-xvfb ffmpeg iproute2 ethtool \
  libva-utils intel-media-driver
```

Selkies legacy GPU baseline additionally requires `gstreamer gst-plugins-base
 gst-plugins-good gst-plugins-bad gst-plugins-ugly gst-plugin-va python-gobject
 python-gst x264`. Baseline measurements are separate from the production path.

Run these exact **1920×1080 CSS, DPR1** commands from the extracted directory.
Both use the same fixtures, encrypted transport and 8 Mbps budget. The matrix runs
sequentially. For a docs smoke set `MD_CASES=local:docs:8000000`.

Software x264 (hardware disabled even if a device is present):

```sh
sudo env MD_GEOMETRY=1920x1080 MD_CODEC=avc1.420033 MD_SOFTWARE=1 \
  MD_CASES=local:docs:8000000,local:canvas:8000000,local:video:8000000,local:scroll30:8000000,local:scroll60:8000000,local:wheel30:8000000,local:wheel60:8000000 \
  ./display-lan-kit/run-lan.sh
```

VAAPI probe and matrix (iHD device access inherited from invoking user groups):

```sh
sudo env MD_GEOMETRY=1920x1080 MD_CODEC=avc1.420033 MD_SOFTWARE=0 \
  MD_CASES=local:docs:8000000,local:canvas:8000000,local:video:8000000,local:scroll30:8000000,local:scroll60:8000000,local:wheel30:8000000,local:wheel60:8000000 \
  ./display-lan-kit/run-lan.sh
```

`/dev/dri` and advertised FFmpeg encoders establish capability only. Successful
`motion_backend_vaapi` traces establish actual use; unavailable hardware falls
back to software and must be reported as such. The receipt records the actual driver initialization result. Native-feature kernels use the same masked capture/codec path for VAAPI and software. A requested hardware run without successful VAAPI packets prints HARDWARE REQUEST FAILED and records hardware_fallback=true.

MP-08/MP-10: repeat the same seven-case software and hardware matrices at
Retina density by adding `MD_GEOMETRY=1280x800 MD_DPR=2` to each command.
Report all cases, including requested hardware fallback, separately. For the
unchanged privacy regression run three sequential DPR2 canvas campaigns with
`MD_CASES=local:canvas:8000000 MD_PROTECTED=1 MD_DYNAMIC_PROTECTED=1
MD_PROTECTION_REPETITIONS=70`; add `MD_SUPERVISOR_CRASH=1` to the third run.
Each campaign retains screenshots and logs. Zero masking violations and exact
owned cleanup are required; these fixture runs remain supplementary evidence.

MP-08/MP-10: measure typing during active scroll and wheel separately from the
unchanged matrix by adding `MD_TYPE_DURING_MOTION=1` and setting
`MD_CASES=local:scroll60:8000000,local:wheel60:8000000`. Use
`MD_CREDIT_WINDOW=4` for the phase29/30 reference condition, and run twice.
Receipts record the first acknowledged glyph echo in presentation history; a
newer frame awaiting rAF cannot erase that proof. The aggregate reports concurrent
typing separately and stays RED when a measured motion typing probe misses the
RTT+50ms gate. Settled probes alone do not establish the concurrent target.
`wan8` provides a supplementary viewer-only 8Mbps,80msRTT condition; it is not a
hosted WSS or public-site acceptance run.

`MD_CHROME=/absolute/browser` selects Chromium. Builder runs set
`MD_MEMORY_FLOOR_GIB=12`; the laptop default reserves 15% of RAM (between 0.5 and 2 GiB), with a 10 GiB disk reserve. Explicit builder limits remain authoritative. These are drill reserves, not product runtime requirements. The old
DPR2 geometry remains available with `MD_GEOMETRY=1280x800`.

MP-11: kits bundle xxhash as an explicit dlopen dependency and prove capture/stripe imports in a chroot using only bundled libraries before creating the manifest. Native kits bundle the worker ELF. The drill preflights the system loader and system libraries first so the stock iHD driver uses its host libva/glibc stack; an incompatible host falls back to bundled libraries with exact loader errors in `native_worker.failures`. Each requested hardware failure records bounded render-device and libva initialization messages in `hardware_diagnostics`. A successful loader preflight is not evidence of hardware encoding; require `motion_backend_vaapi` receipts on the target.

Results and evidence are under the invoking user's
`~/.chariox/dev/display-lan-<run>/`. The launcher removes only its own disposable
state after verified process settlement; uncertain state is retained and RED.
No services are managed. Campaign functional success can coexist with a RED
performance aggregate (exit 1). rAF is a software presentation proxy, not photons.
These local tests do not close MP-10 managed acceptance or MP-11 security review.

MP-08/MP-10 reports require click **and type** P95 at most measured RTT
plus 50 ms, pipeline CPU at most 0.6 cores (DPR1) or 1.2 (DPR2), and exact
presentation below 300 ms at DPR1 or 500 ms at DPR2 after motion stops. Scroll60/wheel60 require at least
59 measured fps at DPR1 (a 60 Hz source with measurement overhead) or 50 fps at
DPR2. The local component kit does not establish these results on the hosted
relay, real public-site list or real desktop browser; those need separate live
receipts. Successful functional teardown alone cannot produce a performance PASS.

The bundled `baseline-campaign.mjs`, `baseline-drill.mjs` and both baseline viewers
use identical fixtures, input-to-pixel probes and Linux CPU accounting. Install
upstream baselines into operator-owned development venvs; no provider state is
needed. Use host Python for GStreamer GI (the bundled Python intentionally does
not include system GI plugins):

```sh
sudo pacman -S --needed python python-pip gstreamer gst-plugins-base \
  gst-plugins-good gst-plugins-bad gst-plugins-ugly gst-plugin-va \
  python-gobject python-gst x264
python -m venv ~/.chariox/dev/display-selkies2
~/.chariox/dev/display-selkies2/bin/pip install \
  https://github.com/selkies-project/selkies/releases/download/2.0.0/selkies-2.0.0.tar.gz
python -m venv --system-site-packages ~/.chariox/dev/display-selkies-legacy
~/.chariox/dev/display-selkies-legacy/bin/pip install \
  https://github.com/selkies-project/selkies/archive/17a3d5a1213257dc3285be5c194b16580dc19fee.tar.gz
```

Run both software baselines sequentially (absolute paths required):

```sh
sudo env MD_MEMORY_FLOOR_GIB=2 /absolute/display-lan-kit/runtime/bin/node \
  /absolute/display-lan-kit/apps/browser-display/baseline-campaign.mjs \
  /absolute/evidence/software-baselines /absolute/display-lan-kit/tools \
  /absolute/home/.chariox/dev/display-selkies2/bin/python \
  /absolute/home/.chariox/dev/display-selkies-legacy/bin/python
```

For the GPU comparison use the same command with
`MD_BASELINE_GPU=1 MD_BASELINE_ENCODER=vah264enc`; Selkies2 requests its upstream
hardware path and legacy uses GStreamer's VA encoder. The software kit can only
probe these paths here. Record actual encoder use on the laptop; a requested
hardware mode is not proof of acceleration. Keep GPU results unmeasured until
that run. Baseline receipts name their public upstream revision/version and
retain startup failures. They never establish Chariox runtime acceptance.
