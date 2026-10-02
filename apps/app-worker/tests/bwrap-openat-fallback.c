#include <assert.h>
#include <stdarg.h>
#include <sys/syscall.h>
#include <stdlib.h>
#include <stdio.h>
#include "utils.h"
#include <errno.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <unistd.h>
#include <string.h>

static int calls, moving, move_root, move_destination;
int cx_test_openat(int dirfd, const char *name, int flags, ...) {
  int fd = openat(dirfd, name, flags);
  if (moving && fd >= 0 && !strcmp(name, "file")) {
    assert(!renameat(move_root, "package", move_destination, "package"));
    moving = 0;
  }
  return fd;
}
long cx_test_syscall(long number, ...) {
  assert(number == SYS_openat2);
  ++calls;
  errno = ENOSYS;
  return -1;
}

int main(void) {
  char scratch[] = "/tmp/chariox-bwrap-fallback.XXXXXX";
  assert(mkdtemp(scratch));
  int root = open(scratch, O_PATH | O_DIRECTORY | O_CLOEXEC);
  assert(root >= 0);
  assert(!mkdirat(root, "package", 0700));
  int file = openat(root, "package/file", O_CREAT | O_WRONLY | O_CLOEXEC, 0600);
  assert(file >= 0); close(file);
  assert(!symlinkat("/etc", root, "escape"));
  assert(!symlinkat("file", root, "package/link"));
  const char *good[] = {"package", "package/file", "/package/file", "package/./file", "/", "."};
  for (size_t i = 0; i < sizeof(good)/sizeof(*good); ++i) {
    int fd = safe_openat(root, scratch, good[i], O_PATH | O_CLOEXEC, 0);
    assert(fd >= 0);
    assert(fcntl(fd, F_GETFD) & FD_CLOEXEC);
    struct stat held, named;
    assert(!fstat(fd, &held));
    if (!strcmp(good[i], "/") || !strcmp(good[i], ".")) assert(!fstat(root, &named));
    else assert(!fstatat(root, good[i][0] == '/' ? good[i]+1 : good[i], &named, 0));
    assert(held.st_dev == named.st_dev && held.st_ino == named.st_ino); close(fd);
  }
  assert(calls == 1); /* First ENOSYS disables openat2; later opens use fallback. */
  const char *bad[] = {"escape/passwd", "package/link", "package/../package/file", "../etc/passwd", "package/file/"};
  for (size_t i = 0; i < sizeof(bad)/sizeof(*bad); ++i)
    assert(safe_openat(root, scratch, bad[i], O_PATH | O_CLOEXEC, 0) < 0);
  /* Flags which could modify an object before validation fail closed. */
  assert(safe_openat(root, scratch, "package/file", O_TRUNC | O_RDONLY, 0) < 0);
  char outside[] = "/tmp/chariox-bwrap-outside.XXXXXX";
  assert(mkdtemp(outside));
  move_root = root;
  move_destination = open(outside, O_PATH | O_DIRECTORY | O_CLOEXEC);
  assert(move_destination >= 0);
  moving = 1;
  assert(safe_openat(root, scratch, "package/file", O_PATH | O_CLOEXEC, 0) < 0);
  assert(!moving); /* The ancestor actually moved outside the pinned root. */
  assert(!renameat(move_destination, "package", root, "package"));
  close(move_destination); assert(!rmdir(outside));
  unlinkat(root, "escape", 0); unlinkat(root, "package/link", 0);
  unlinkat(root, "package/file", 0); unlinkat(root, "package", AT_REMOVEDIR);
  close(root); assert(!rmdir(scratch));
  puts("13 fallback checks passed (injected ENOSYS, normal paths, symlinks, dot-dot, trailing slash, flags, ancestor rename)");
}
