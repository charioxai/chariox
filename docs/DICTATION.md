# Kernel dictation

Dictation turns speech into text on the kernel the person is connected to. Every kernel is the
same: the feature is not "local" or "remote", it runs wherever the person's Prompt bar is
connected. Audio goes from the browser to that kernel, end-to-end encrypted like every request, and
is never stored or sent to a third party.

## Model

The kernel binary contains the whisper.cpp engine (CPU, about 2.3 MB linked) but not a speech
model. The model downloads on first use, after the person agreed in the client:

- `whisper-base-q5_1` (whisper.cpp `base`, multilingual, 5-bit quantized): 59,707,625 bytes
  (about 57 MiB), SHA-256 `422f1ae4…8898`, from an immutable revision of the upstream
  `ggerganov/whisper.cpp` repository (`apps/kernel/src/dictation/catalog.rs`).
- Chosen over `tiny-q5_1` (about 31 MiB, roughly half the time) because it is clearly better on
  Spanish ("dengue", "hipótesis") while English was identical on the same samples; about 0.9 s
  to transcribe 15 s of speech and about 200 MB of memory while loaded on a 4-thread CPU.
  The loaded model is released after five idle minutes.
- The download resumes (`<model>.part`, HTTP `Range`), is verified against the pinned size and
  SHA-256 and is installed under its final name only when it matches; a mismatch removes the partial
  file and reports `failed`. State lives in `<private runtime state>/dictation/`
  (`ggml-base-q5_1.bin`, `settings.json`).

## Audio

Browsers record with MediaRecorder, decode and resample to 16 kHz mono 16-bit PCM in the page and
upload it in chunks, so the kernel needs no audio decoders. See `docs/PROTOCOL.md` (594/200).

## Building

`whisper-rs` builds whisper.cpp with CMake and generates bindings with libclang. Kernel build
environments therefore need `cmake`, a C++ compiler and `libclang-dev` (the publication Dockerfile
installs them); the kernel then links `libstdc++` dynamically.
