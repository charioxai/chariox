#define _GNU_SOURCE
#include "runtime.h"

#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <pthread.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/resource.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>
#if defined(__linux__)
#include <sys/inotify.h>
#include <sys/syscall.h>
#endif

// Test-only replacement runtime artifact. The production launcher has no test
// mode. Release installation never accepts this unsigned fixture as a runtime.
static int failed = 0;

static void check(const char* name, int passed) {
  printf("%s:%s\n", name, passed ? "ok" : "FAIL");
  if (!passed) failed = 1;
}

static int create_private(const char* root, const char* name) {
  char path[2048];
  snprintf(path, sizeof(path), "%s/%s", root, name);
  int fd = open(path, O_WRONLY | O_CREAT | O_EXCL, 0600);
  if (fd < 0) return 0;
  int valid = write(fd, "probe", 5) == 5 && fsync(fd) == 0;
  close(fd);
  return valid;
}

static void* thread_entry(void* context) {
  *(int*)context = 1;
  return context;
}

__attribute__((constructor)) static void before_runtime_entry(void) {
  const char* data = getenv("CHARIOX_APP_DATA");
  check("constructor_already_confined", data && create_private(data, "constructor-ran"));
  check("environment_filtered", !getenv("CX_TEST_SECRET") && !getenv("NODE_OPTIONS") && !getenv("PATH"));
  check("ambient_descriptor_closed", fcntl(31, F_GETFD) == -1 && errno == EBADF);
  char path[2048];
  snprintf(path, sizeof(path), "%s/escape", getenv("CHARIOX_APP_PACKAGE"));
  int fd = open(path, O_RDONLY);
  check("constructor_host_read_denied", fd < 0);
  if (fd >= 0) close(fd);
}

const char* chariox_app_runtime_node_version(void) { return "24.20.0"; }

int chariox_app_runtime_run(const struct chariox_runtime_config* config) {
  check("trusted_bootstrap", config->trusted_bootstrap_length == 8 &&
      !memcmp(config->trusted_bootstrap, "probe-v1", 8));
  char request[4];
  struct pollfd sdk = {3, POLLIN, 0};
  check("inherited_sdk_bidirectional", poll(&sdk, 1, 1000) > 0 && read(3, request, 4) == 4 &&
      !memcmp(request, "PING", 4) && write(3, "PONG", 4) == 4);
  int thread_result = 0;
  pthread_t thread;
  int created = pthread_create(&thread, NULL, thread_entry, &thread_result);
  check("native_thread_create_join", created == 0 && pthread_join(thread, NULL) == 0 && thread_result == 1);
  check("private_data_write", create_private(getenv("CHARIOX_APP_DATA"), "written"));
  check("private_tmp_write", create_private(getenv("CHARIOX_APP_TMP"), "written"));
  check("package_write_denied", !create_private(getenv("CHARIOX_APP_PACKAGE"), "forbidden"));
  char path[2048];
  snprintf(path, sizeof(path), "%s/../secret", getenv("CHARIOX_APP_DATA"));
  int fd = open(path, O_RDONLY);
  check("parent_traversal_denied", fd < 0);
  if (fd >= 0) close(fd);
  fd = open("/etc/passwd", O_RDONLY);
  check("unrelated_host_read_denied", fd < 0);
  if (fd >= 0) close(fd);
  snprintf(path, sizeof(path), "%s/fixture.bin", getenv("CHARIOX_APP_PACKAGE"));
  fd = open(path, O_RDONLY);
  check("package_read", fd >= 0);
  if (fd >= 0) {
    void* mapped = mmap(NULL, 4096, PROT_READ | PROT_EXEC, MAP_PRIVATE, fd, 0);
    check("package_executable_mapping_denied", mapped == MAP_FAILED);
    if (mapped != MAP_FAILED) munmap(mapped, 4096);
    mapped = mmap(NULL, 4096, PROT_READ, MAP_PRIVATE, fd, 0);
#if defined(__APPLE__)
    // Seatbelt alone does not necessarily mediate a later mprotect transition.
    // Keep this observed unsigned limitation visible; the stronger code-memory
    // contract belongs to the signed/hardened JIT release fixture.
    int executable = mapped != MAP_FAILED && mprotect(mapped, 4096, PROT_READ | PROT_EXEC) == 0;
    printf("unsigned_file_mapping_to_executable:%s\n", executable ? "permitted" : "denied");
    if (mapped == MAP_FAILED) failed = 1;
#else
    check("package_mapping_cannot_become_executable", mapped != MAP_FAILED &&
        mprotect(mapped, 4096, PROT_READ | PROT_EXEC) < 0);
#endif
    if (mapped != MAP_FAILED) munmap(mapped, 4096);
    close(fd);
  }
  void* jit = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
  check("anonymous_memory", jit != MAP_FAILED);
  if (jit != MAP_FAILED) munmap(jit, 4096);
  fd = socket(AF_INET, SOCK_STREAM, 0);
  if (fd >= 0) {
    struct sockaddr_in address = {0};
    address.sin_family = AF_INET;
    address.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    address.sin_port = htons(9);
    errno = 0;
    int result = connect(fd, (struct sockaddr*)&address, sizeof(address));
    check("raw_network_denied", result < 0 && (errno == EPERM || errno == EACCES));
    close(fd);
  } else check("raw_network_denied", errno == EPERM || errno == EACCES);
  pid_t child = fork();
  if (child == 0) _exit(0);
  check("fork_denied", child < 0);
  if (child > 0) waitpid(child, NULL, 0);
  errno = 0;
#if defined(__linux__)
  // PID1 has no visible parent. A non-self PID must be denied by policy even
  // when no such target exists; ESRCH alone would not prove a signal boundary.
  check("foreign_signal_denied", kill(getpid() + 1000, 0) < 0 && errno == EPERM);
#else
  check("foreign_signal_denied", kill(getppid(), 0) < 0 && (errno == EPERM || errno == EACCES));
#endif
  struct rlimit limits;
  check("native_limit_query", getrlimit(RLIMIT_NOFILE, &limits) == 0 && limits.rlim_cur == 64);
#if defined(__linux__)
  errno = 0;
  check("native_limit_mutation_denied", syscall(SYS_prlimit64, 0, RLIMIT_NOFILE, &limits, NULL) < 0 && errno == EPERM);
  errno = 0;
  check("foreign_limit_query_denied", syscall(SYS_prlimit64, getpid() + 1000, RLIMIT_NOFILE, NULL, &limits) < 0 && errno == EPERM);
  snprintf(path, sizeof(path), "%s/written", getenv("CHARIOX_APP_DATA"));
  int source = open(path, O_RDONLY);
  snprintf(path, sizeof(path), "%s/copied", getenv("CHARIOX_APP_DATA"));
  int destination = open(path, O_WRONLY | O_CREAT | O_EXCL, 0600);
  check("private_descriptor_copy", source >= 0 && destination >= 0 &&
      syscall(SYS_copy_file_range, source, NULL, destination, NULL, 5, 0) == 5);
  if (source >= 0) close(source);
  if (destination >= 0) close(destination);
  int watch_fd = inotify_init1(IN_NONBLOCK | IN_CLOEXEC);
  int watch = watch_fd < 0 ? -1 : inotify_add_watch(watch_fd, getenv("CHARIOX_APP_DATA"), IN_CREATE);
  check("private_file_watch", watch >= 0);
  char event_bytes[512];
  int generated = watch >= 0 && create_private(getenv("CHARIOX_APP_DATA"), "watched");
  ssize_t event_size = generated ? read(watch_fd, event_bytes, sizeof(event_bytes)) : -1;
  struct inotify_event event = {0};
  if (event_size >= (ssize_t)sizeof(event)) memcpy(&event, event_bytes, sizeof(event));
  check("private_file_watch_event", generated && event_size >= (ssize_t)sizeof(event) && (event.mask & IN_CREATE));
  snprintf(path, sizeof(path), "%s/escape", getenv("CHARIOX_APP_PACKAGE"));
  check("escaped_file_watch_denied", watch_fd >= 0 && inotify_add_watch(watch_fd, path, IN_MODIFY) < 0);
  if (watch >= 0) check("private_file_watch_remove", inotify_rm_watch(watch_fd, watch) == 0);
  if (watch_fd >= 0) close(watch_fd);
#endif
  // A permitted exec replaces the fixture with harmless /usr/bin/false, so
  // the harness observes failure. Flush prior findings before that can happen.
  fflush(stdout);
  char* args[] = {"false", NULL};
  char* environment[] = {NULL};
  errno = 0;
  check("exec_denied", execve("/usr/bin/false", args, environment) < 0 && (errno == EPERM || errno == EACCES));
  return failed ? 1 : 0;
}
