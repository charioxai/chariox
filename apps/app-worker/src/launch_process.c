#define _GNU_SOURCE
#include "launcher.h"

#include <errno.h>
#include <fcntl.h>
#include <stdbool.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>
#if defined(__APPLE__)
#include <libproc.h>
#endif
#if defined(__linux__)
#include <sys/syscall.h>
#endif

extern char** environ;

static bool is_prefix(const char* parent, const char* child) {
  size_t length = strlen(parent);
  return !strncmp(parent, child, length) && (child[length] == '/' || !child[length]);
}

static int open_root(const char* path, bool writable) {
  if (path[0] != '/' || path[1] == '\0') return -1;
  char* canonical = realpath(path, NULL);
  bool exact = canonical && !strcmp(path, canonical);
  free(canonical);
  if (!exact) return -1;
  // Resolve every component relative to the held parent. No symlink may
  // substitute a root, and no other user may replace a trusted ancestor.
  int current = open("/", O_RDONLY | O_DIRECTORY | O_CLOEXEC);
  char components[CX_APP_PATH_MAX + 1];
  memcpy(components, path + 1, strlen(path));
  char* state = NULL;
  for (char* part = strtok_r(components, "/", &state); part;
       part = strtok_r(NULL, "/", &state)) {
    int next = openat(current, part, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
    close(current);
    if (next < 0) return -1;
    current = next;
    struct stat metadata;
    if (fstat(current, &metadata) ||
        (metadata.st_uid != geteuid() && metadata.st_uid != 0) ||
        (metadata.st_mode & (S_IWGRP | S_IWOTH))) { close(current); return -1; }
  }
  struct stat metadata;
  if (fstat(current, &metadata) || (writable && metadata.st_uid != geteuid())) {
    close(current);
    return -1;
  }
  return current;
}

static bool private_stream(int fd, struct stat* metadata) {
  struct sockaddr_un address;
  socklen_t size = sizeof(address);
  int kind = 0;
  socklen_t kind_size = sizeof(kind);
  return !fstat(fd, metadata) && S_ISSOCK(metadata->st_mode) &&
      !getsockopt(fd, SOL_SOCKET, SO_TYPE, &kind, &kind_size) && kind == SOCK_STREAM &&
      !getpeername(fd, (struct sockaddr*)&address, &size) && address.sun_family == AF_UNIX;
}

static bool filter_descriptors(void) {
  struct stat sdk, control, output;
  if (!private_stream(CX_APP_SDK_FD, &sdk) || !private_stream(CX_APP_CONTROL_FD, &control) ||
      (sdk.st_dev == control.st_dev && sdk.st_ino == control.st_ino)) return false;
  for (int fd = 1; fd <= 2; ++fd) {
    if (fstat(fd, &output) || !(S_ISFIFO(output.st_mode) || S_ISSOCK(output.st_mode)) ||
        (output.st_dev == sdk.st_dev && output.st_ino == sdk.st_ino) ||
        (output.st_dev == control.st_dev && output.st_ino == control.st_ino)) return false;
  }
  int null_fd = open("/dev/null", O_RDONLY | O_CLOEXEC);
  if (null_fd < 0) return false;
  bool ok = dup2(null_fd, STDIN_FILENO) == STDIN_FILENO;
  if (null_fd != STDIN_FILENO) close(null_fd);
  if (!ok) return false;
#if defined(__APPLE__)
  int size = proc_pidinfo(getpid(), PROC_PIDLISTFDS, 0, NULL, 0);
  if (size <= 0 || size > 4 * 1024 * 1024) return false;
  size += 32 * (int)sizeof(struct proc_fdinfo);
  struct proc_fdinfo* descriptors = malloc((size_t)size);
  if (!descriptors) return false;
  int received = proc_pidinfo(getpid(), PROC_PIDLISTFDS, 0, descriptors, size);
  if (received <= 0 || received >= size || received % sizeof(*descriptors)) {
    free(descriptors);
    return false;
  }
  for (size_t i = 0; i < (size_t)received / sizeof(*descriptors); ++i)
    if (descriptors[i].proc_fd >= 5) close(descriptors[i].proc_fd);
  free(descriptors);
#elif defined(__linux__)
  if (syscall(SYS_close_range, 5U, ~0U, 0U)) return false;
#else
#error Unsupported App worker platform
#endif
  for (int fd = 0; fd <= 4; ++fd) {
    if (fcntl(fd, F_SETFD, FD_CLOEXEC) == -1) return false;
  }
  // The startup stream is nonblocking, so every read/write remains governed
  // by the monotonic deadline even after a spurious readiness notification.
  int flags = fcntl(CX_APP_CONTROL_FD, F_GETFL);
  return flags != -1 && fcntl(CX_APP_CONTROL_FD, F_SETFL, flags | O_NONBLOCK) != -1;
}

int cx_prepare_descriptors(void) {
  return filter_descriptors() ? 0 : CX_LAUNCH_DESCRIPTORS;
}

static bool limit(int resource, rlim_t value) {
  struct rlimit previous;
  if (getrlimit(resource, &previous)) return false;
  struct rlimit next = {value, value};
  return (previous.rlim_max == RLIM_INFINITY || value <= previous.rlim_max) &&
         !setrlimit(resource, &next);
}

int cx_prepare_process(const struct cx_launch_record* record) {
  for (size_t i = 0; i < CX_ROOT_COUNT; ++i) {
    for (size_t j = 0; j < i; ++j) {
      if (is_prefix(record->roots[i], record->roots[j]) ||
          is_prefix(record->roots[j], record->roots[i])) return CX_LAUNCH_PATHS;
    }
  }
  // Ambient descriptors have already been closed before parsing the record.
  int roots[CX_ROOT_COUNT] = {-1, -1, -1, -1};
  int status = 0;
  for (size_t i = 0; i < CX_ROOT_COUNT; ++i) {
    roots[i] = open_root(record->roots[i], i == CX_DATA || i == CX_TMP);
    if (roots[i] < 0) { status = CX_LAUNCH_PATHS; break; }
    for (size_t j = 0; j < i; ++j) {
      struct stat a, b;
      if (fstat(roots[i], &a) || fstat(roots[j], &b) ||
          (a.st_dev == b.st_dev && a.st_ino == b.st_ino)) status = CX_LAUNCH_PATHS;
    }
  }
  if (!status && fchdir(roots[CX_DATA])) status = CX_LAUNCH_PATHS;
  for (size_t i = 0; i < CX_ROOT_COUNT; ++i) if (roots[i] >= 0) close(roots[i]);
  if (status) return status;
  umask(0077);
  // This does not undo loader injection before main: the supervisor MUST
  // provide its allowlisted environment to exec/posix_spawn itself.
  static char* empty_environment[] = {NULL};
  environ = empty_environment;
  if (setenv("HOME", record->roots[CX_DATA], 1) ||
      setenv("TMPDIR", record->roots[CX_TMP], 1) ||
      setenv("LANG", "C.UTF-8", 1) || setenv("TZ", "UTC", 1) ||
      setenv("UV_THREADPOOL_SIZE", "4", 1) ||
      setenv("CHARIOX_APP_GENERATION", record->generation, 1) ||
      setenv("CHARIOX_APP_INSTALLATION", record->installation, 1) ||
      setenv("CHARIOX_APP_RELEASE_DIGEST", record->digest, 1) ||
      setenv("CHARIOX_APP_PACKAGE", record->roots[CX_PACKAGE], 1) ||
      setenv("CHARIOX_APP_DATA", record->roots[CX_DATA], 1) ||
      setenv("CHARIOX_APP_TMP", record->roots[CX_TMP], 1)) return CX_LAUNCH_ENVIRONMENT;
  if (!limit(RLIMIT_CORE, 0) || !limit(RLIMIT_NOFILE, record->nofile) ||
      !limit(RLIMIT_CPU, record->cpu_seconds) ||
      !limit(RLIMIT_FSIZE, (rlim_t)record->max_file_bytes)) return CX_LAUNCH_LIMITS;
  return 0;
}
