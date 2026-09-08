#ifndef CHARIOX_APP_RUNTIME_H
#define CHARIOX_APP_RUNTIME_H

#include <stddef.h>
#include <stdint.h>

#if defined(__GNUC__)
#define CHARIOX_RUNTIME_EXPORT __attribute__((visibility("default")))
#else
#define CHARIOX_RUNTIME_EXPORT
#endif

#ifdef __cplusplus
extern "C" {
#endif

enum chariox_runtime_status {
  CHARIOX_RUNTIME_INVALID_CONFIG = 120,
  CHARIOX_RUNTIME_ALREADY_STARTED = 121,
  CHARIOX_RUNTIME_INITIALIZATION_FAILED = 122,
  CHARIOX_RUNTIME_BOOTSTRAP_FAILED = 123,
  CHARIOX_RUNTIME_INTERNAL_FAILURE = 124
};

// Internal ABI between signed Chariox artifacts, never an App-facing API.
// The launcher must apply the OS sandbox and resource limits BEFORE dlopen():
// Node's static initializers run while the library loads, before this entry.
// argv and bootstrap must come from the verified Chariox runtime/launcher,
// never App files, App-supplied IPC, NODE_OPTIONS, or inherited environment.
// The launcher owns descriptor filtering and Node permission arguments; the
// trusted SDK bootstrap owns the authenticated inherited IPC channel and
// imports App code only after its supervised startup handshake.
// This entry is one-shot: the launcher exits after it returns, including on
// failure. It must never reuse or dlclose an initialized runtime process.
struct chariox_runtime_config {
  uint32_t abi_version;
  uint32_t v8_threads;
  size_t argc;
  const char* const* argv;
  const char* trusted_bootstrap;
  size_t trusted_bootstrap_length;
};

CHARIOX_RUNTIME_EXPORT const char* chariox_app_runtime_node_version(void);
CHARIOX_RUNTIME_EXPORT int chariox_app_runtime_run(const struct chariox_runtime_config* config);

#ifdef __cplusplus
}
#endif
#endif
