#include "launcher.h"
#include "runtime.h"

#include <dlfcn.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static int launch(void) {
  struct cx_launch_record record = {0};
  int64_t now = cx_monotonic_ms();
  if (now < 0) return CX_LAUNCH_TIMEOUT;
  int64_t deadline = now + 15000;
  int status = cx_prepare_descriptors();
  if (!status) status = cx_read_record(&record, deadline);
  if (!status) status = cx_prepare_process(&record);
  if (!status) status = cx_apply_platform_sandbox(&record);
  if (!status) status = cx_ready_and_wait(&record, deadline);
  close(CX_APP_CONTROL_FD);
  if (status) { free(record.bootstrap); return status; }

  char library[CX_APP_PATH_MAX + 64];
#if defined(__APPLE__)
  const char* filename = "libchariox-app-runtime.dylib";
#elif defined(__linux__)
  const char* filename = "libchariox-app-runtime.so";
#endif
  snprintf(library, sizeof(library), "%s/%s", record.roots[CX_RUNTIME], filename);
  // No Node library is linked into this executable. Its constructors execute
  // here only, after confinement AND the supervisor's continuation receipt.
  void* handle = dlopen(library, RTLD_NOW | RTLD_LOCAL);
  if (!handle) { free(record.bootstrap); return CX_LAUNCH_LIBRARY; }
  const char* (*version)(void) = (const char* (*)(void))dlsym(handle, "chariox_app_runtime_node_version");
  int (*run)(const struct chariox_runtime_config*) =
      (int (*)(const struct chariox_runtime_config*))dlsym(handle, "chariox_app_runtime_run");
  if (!version || !run || strcmp(version(), "24.20.0")) return CX_LAUNCH_VERSION;

  char read_flags[CX_ROOT_COUNT][CX_APP_PATH_MAX + 32];
  char write_flags[2][CX_APP_PATH_MAX + 32];
  char heap[64];
  snprintf(heap, sizeof(heap), "--max-old-space-size=%u", record.heap_mib);
  const char* argv[16] = {"chariox-app-worker", "--permission", "--no-addons", "--allow-worker", heap};
  size_t argc = 5;
  for (size_t i = 0; i < CX_ROOT_COUNT; ++i) {
    snprintf(read_flags[i], sizeof(read_flags[i]), "--allow-fs-read=%s", record.roots[i]);
    argv[argc++] = read_flags[i];
  }
  for (size_t i = 0; i < 2; ++i) {
    snprintf(write_flags[i], sizeof(write_flags[i]), "--allow-fs-write=%s", record.roots[i + CX_DATA]);
    argv[argc++] = write_flags[i];
  }
  struct chariox_runtime_config config = {
      1, record.v8_threads, argc, argv, record.bootstrap, record.bootstrap_length};
  status = run(&config);
  // Do not dlclose or reuse a process after initializing Node/V8.
  free(record.bootstrap);
  return status;
}

int main(int argc, char** argv) {
  (void)argv;
  signal(SIGPIPE, SIG_IGN);
  int status = argc == 1 ? launch() : CX_LAUNCH_INVALID;
  if (status >= CX_LAUNCH_INVALID && status <= CX_LAUNCH_TIMEOUT)
    fprintf(stderr, "app_worker_startup_failed:%d\n", status);
  return status;
}
