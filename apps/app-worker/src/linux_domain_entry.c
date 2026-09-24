/* Trusted pre-bubblewrap entry; never an App-provided executable or argument set.
 * FD5 is the owned cgroup.procs writer; FD6 the pinned bubblewrap executable.
 * Both disappear before bubblewrap runs. The existing App ABI remains FD0..4.
 */
#if defined(__linux__)
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <linux/magic.h>
#include <stdint.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/vfs.h>
#include <unistd.h>

enum { DOMAIN_FAILURE = 108, EXEC_FAILURE = 109 };

int main(int argc, char** argv) {
  struct stat cgroup, binary;
  struct statfs filesystem;
  if (argc < 2 || argc > 128 || fstat(5, &cgroup) || fstatfs(5, &filesystem) ||
      filesystem.f_type != CGROUP2_SUPER_MAGIC || !S_ISREG(cgroup.st_mode) ||
      (fcntl(5, F_GETFL) & O_ACCMODE) != O_WRONLY ||
      fstat(6, &binary) || !S_ISREG(binary.st_mode) || binary.st_nlink != 1 ||
      !(binary.st_mode & 0111) || (binary.st_mode & 06022) ||
      (binary.st_uid != 0 && binary.st_uid != geteuid())) return DOMAIN_FAILURE;
  size_t bytes = 0;
  for (int index = 1; index < argc; ++index) {
    const size_t length = strnlen(argv[index], 2049);
    if (length > 2048 || (bytes += length + 1) > 32768) return DOMAIN_FAILURE;
  }
  ssize_t written;
  do { written = write(5, "0\n", 2); } while (written < 0 && errno == EINTR);
  if (written != 2 || close(5) || fcntl(6, F_SETFD, FD_CLOEXEC) ||
      syscall(SYS_close_range, 7U, ~0U, 0U)) return DOMAIN_FAILURE;
  char* environment[] = {"LANG=C.UTF-8", "TZ=UTC", NULL};
  /* A held executable descriptor avoids reopening a possibly replaced path.
   * The factory's enrolled immutable lease also prevents mutation of its bytes.
   */
  syscall(SYS_execveat, 6, "", &argv[1], environment, AT_EMPTY_PATH);
  return EXEC_FAILURE;
}
#endif
