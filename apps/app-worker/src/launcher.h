#ifndef CHARIOX_APP_LAUNCHER_H
#define CHARIOX_APP_LAUNCHER_H

#include <stddef.h>
#include <stdint.h>

#define CX_APP_SDK_FD 3
#define CX_APP_CONTROL_FD 4
#define CX_APP_PATH_MAX 1024
#define CX_APP_BOOTSTRAP_MAX (256 * 1024)
#define CX_APP_RECORD_MAX 280000

enum cx_launch_status {
  CX_LAUNCH_INVALID = 100,
  CX_LAUNCH_PATHS = 101,
  CX_LAUNCH_DESCRIPTORS = 102,
  CX_LAUNCH_LIMITS = 103,
  CX_LAUNCH_SANDBOX = 104,
  CX_LAUNCH_HANDSHAKE = 105,
  CX_LAUNCH_LIBRARY = 106,
  CX_LAUNCH_VERSION = 107,
  CX_LAUNCH_ENVIRONMENT = 108,
  CX_LAUNCH_TIMEOUT = 109
};

enum cx_root { CX_PACKAGE, CX_DATA, CX_TMP, CX_RUNTIME, CX_ROOT_COUNT };

struct cx_launch_record {
  uint32_t nofile;
  uint32_t cpu_seconds;
  uint32_t heap_mib;
  uint32_t v8_threads;
  uint64_t max_file_bytes;
  char generation[129];
  char installation[129];
  char digest[65];
  char roots[CX_ROOT_COUNT][CX_APP_PATH_MAX + 1];
  char* bootstrap;
  size_t bootstrap_length;
};

// Trusted supervisor protocol only. There is no App-selectable executable,
// library, profile, Node argument, test mode, or bootstrap path.
int cx_read_record(struct cx_launch_record*, int64_t deadline_ms);
int cx_ready_and_wait(const struct cx_launch_record*, int64_t deadline_ms);
int64_t cx_monotonic_ms(void);
int cx_prepare_descriptors(void);
int cx_prepare_process(const struct cx_launch_record*);
int cx_apply_platform_sandbox(const struct cx_launch_record*);

#endif
