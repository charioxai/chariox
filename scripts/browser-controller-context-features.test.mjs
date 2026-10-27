// The repository's default `pnpm test` discovers only scripts/*.test.mjs.
// Keep this root registration wrapper deliberately authority-free: the exact
// acceptance cases remain in the controller's focused test beside its modules.
await import("../apps/kernel/slice-linux-docker/docker/browser-controller-context-features.test.mjs");
