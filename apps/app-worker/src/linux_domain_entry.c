/* Trusted pre-bubblewrap entry; never an App-provided executable or argument set.
 * FD5 is the owned cgroup.procs writer; FD6 the pinned bubblewrap executable.
 * FD7 borrows the authenticated storage lease for the mapping handshake.
 * All disappear before bubblewrap runs. The existing App ABI remains FD0..4.
 */
#if defined(__linux__)
#define _GNU_SOURCE
#include <arpa/inet.h>
#include <errno.h>
#include <grp.h>
#include <linux/capability.h>
#include <poll.h>
#include <sched.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/prctl.h>
#include <sys/socket.h>
#include <time.h>
#include <fcntl.h>
#include <linux/magic.h>
#include <stdint.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/vfs.h>
#include <unistd.h>

enum { DOMAIN_FAILURE = 108, EXEC_FAILURE = 109 };

/* FD7 is the parent's already acquired code/storage lease, borrowed exclusively
 * while the parent waits for this native launch. Never accepted from App code.
 * A root helper maps only our own uid/primary gid in this new user namespace;
 * Linux forbids unprivileged setgroups both before mapping and after deny.
 */
static int64_t monotonic_ms(void) {
  struct timespec value;
  if (clock_gettime(CLOCK_MONOTONIC, &value)) return -1;
  return (int64_t)value.tv_sec * 1000 + value.tv_nsec / 1000000;
}
static int transfer(int writing, void* bytes, size_t count, int64_t deadline) {
  size_t done = 0;
  while (done < count) {
    struct pollfd ready = {7, writing ? POLLOUT : POLLIN, 0};
    const int64_t now = monotonic_ms();
    if (now < 0 || now >= deadline || poll(&ready, 1, (int)(deadline - now)) != 1) return -1;
    ssize_t n = writing ? send(7, (char*)bytes + done, count - done, MSG_NOSIGNAL)
                        : recv(7, (char*)bytes + done, count - done, 0);
    if (n < 0 && (errno == EINTR || errno == EAGAIN)) continue;
    if (n <= 0) return -1;
    done += (size_t)n;
  }
  return 0;
}
static int minimal_groups(void) {
  struct ucred peer; socklen_t length = sizeof(peer);
  if (getsockopt(7, SOL_SOCKET, SO_PEERCRED, &peer, &length) ||
      length != sizeof(peer) || peer.uid != 0 || peer.pid <= 1) return -1;
  const uid_t uid = getuid(); const gid_t gid = getgid();
  if (!uid || !gid || uid != geteuid() || gid != getegid()) return -1;
  char stat[4096]; FILE* source = fopen("/proc/self/stat", "re");
  if (!source) return -1;
  size_t n = fread(stat, 1, sizeof(stat) - 1, source); int failed = ferror(source);
  fclose(source); if (failed || n == sizeof(stat) - 1) return -1; stat[n] = 0;
  char* field = strrchr(stat, ')'); if (!field) return -1;
  field += 1; unsigned long long birth = 0;
  for (int index = 0; index <= 19; ++index) {
    while (*field == ' ') ++field;
    char* end = strchr(field, ' ');
    if (!end && index != 19) return -1;
    if (index == 19) { char* tail; errno = 0; birth = strtoull(field, &tail, 10);
      if (errno || !birth || tail == field || (*tail != ' ' && *tail != '\n')) return -1;
    } else field = end + 1;
  }
  if (unshare(CLONE_NEWUSER)) return -1;
  char request[160]; int size = snprintf(request, sizeof(request),
    "{\"operation\":\"map_worker_groups_v1\",\"pid\":%d,\"birth\":%llu}", getpid(), birth);
  if (size <= 0 || size >= (int)sizeof(request)) return -1;
  const int64_t started = monotonic_ms(); if (started < 0) return -1;
  const int64_t deadline = started + 5000;
  uint32_t frame = htonl((uint32_t)size);
  if (transfer(1, &frame, sizeof(frame), deadline) || transfer(1, request, (size_t)size, deadline) ||
      transfer(0, &frame, sizeof(frame), deadline)) return -1;
  char reply[128]; size = (int)ntohl(frame);
  if (size <= 0 || size >= (int)sizeof(reply) || transfer(0, reply, (size_t)size, deadline)) return -1;
  reply[size] = 0;
  if (strcmp(reply, "{\"status\":\"groups_mapped_v1\"}") || setgroups(0, NULL) || getgroups(0, NULL) != 0) return -1;
  /* No namespace capability survives into bubblewrap, much less App code. */
  for (int cap = 0; cap < 64; ++cap)
    if (prctl(PR_CAPBSET_DROP, cap, 0, 0, 0) && errno != EINVAL) return -1;
  struct __user_cap_header_struct header = {_LINUX_CAPABILITY_VERSION_3, 0};
  struct __user_cap_data_struct capabilities[2] = {{0}, {0}};
  if (syscall(SYS_capset, &header, capabilities) || prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) || prctl(PR_SET_DUMPABLE, 1, 0, 0, 0)) return -1;
  return 0;
}
static int failure(void) { close(7); return DOMAIN_FAILURE; }

int main(int argc, char** argv) {
  if (prctl(PR_SET_DUMPABLE, 0, 0, 0, 0)) return failure();
  struct stat cgroup, binary;
  struct statfs filesystem;
  if (argc < 2 || argc > 128 || fstat(5, &cgroup) || fstatfs(5, &filesystem) ||
      filesystem.f_type != CGROUP2_SUPER_MAGIC || !S_ISREG(cgroup.st_mode) ||
      (fcntl(5, F_GETFL) & O_ACCMODE) != O_WRONLY ||
      fstat(6, &binary) || !S_ISREG(binary.st_mode) || binary.st_nlink != 1 ||
      !(binary.st_mode & 0111) || (binary.st_mode & 06022) ||
      (binary.st_uid != 0 && binary.st_uid != geteuid())) return failure();
  size_t bytes = 0;
  for (int index = 1; index < argc; ++index) {
    const size_t length = strnlen(argv[index], 2049);
    if (length > 2048 || (bytes += length + 1) > 32768) return failure();
  }
  ssize_t written;
  do { written = write(5, "0\n", 2); } while (written < 0 && errno == EINTR);
  if (written != 2 || close(5) || fcntl(6, F_SETFD, FD_CLOEXEC) ||
      syscall(SYS_close_range, 8U, ~0U, 0U)) return failure();
  if (minimal_groups() || close(7)) return failure();
  char* environment[] = {"LANG=C.UTF-8", "TZ=UTC", NULL};
  /* A held executable descriptor avoids reopening a possibly replaced path.
   * The factory's enrolled immutable lease also prevents mutation of its bytes.
   */
  syscall(SYS_execveat, 6, "", &argv[1], environment, AT_EMPTY_PATH);
  return EXEC_FAILURE;
}
#endif
