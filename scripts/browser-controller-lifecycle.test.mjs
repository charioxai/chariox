// Include the real Linux lifecycle regression in the default root test command.
await import("../apps/kernel/slice-linux-docker/docker/browser-controller-lifecycle.test.mjs");
await import("../apps/kernel/slice-linux-docker/docker/browser-controller-upload-store.test.mjs");
