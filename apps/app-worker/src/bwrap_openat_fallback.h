/* Fail-closed fallback for the pinned bubblewrap build when a service seccomp
 * filter hides openat2. Chariox passes canonical, symlink-free bind sources.
 * Keep every ancestor pinned, then verify the complete chain before returning.
 * Unlike upstream's general fallback, this deliberately refuses symlinks/.. . */
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

static int cx_same_inode(const struct stat *a, const struct stat *b) {
  return a->st_dev == b->st_dev && a->st_ino == b->st_ino;
}

static int cx_openat_nofollow(int root, const char *path, int flags, int mode) {
  char names[PATH_MAX];
  char *components[PATH_MAX / 2];
  int fds[PATH_MAX / 2 + 1], count = 0, result = -1, saved;
  struct stat held, named, parent;
  (void) mode;
  /* No creation/truncation side effects before containment is checked. */
  if (!path || strlen(path) >= sizeof(names) ||
      ((flags & (O_CREAT | O_TRUNC)) || (flags & O_TMPFILE) == O_TMPFILE) ||
      ((flags & O_ACCMODE) != O_RDONLY)) {
    errno = EINVAL;
    return -1;
  }
  if (path[0] && path[strlen(path) - 1] == '/') flags |= O_DIRECTORY;
  strcpy(names, path);
  char *cursor = names;
  while (*cursor) {
    while (*cursor == '/') ++cursor;
    if (!*cursor) break;
    char *name = cursor;
    while (*cursor && *cursor != '/') ++cursor;
    if (*cursor) *cursor++ = '\0';
    if (!strcmp(name, "..")) { errno = EXDEV; return -1; }
    if (!strcmp(name, ".")) continue;
    components[count++] = name;
  }
  if (!count) {
    int fd = openat(root, ".", flags | O_NOFOLLOW | O_CLOEXEC);
    if (fd < 0) return -1;
    if (fstat(fd, &held) || fstat(root, &named)) {
      saved = errno;
      close(fd);
      errno = saved;
      return -1;
    }
    if (!cx_same_inode(&held, &named)) { close(fd); errno = EXDEV; return -1; }
    return fd;
  }
  fds[0] = fcntl(root, F_DUPFD_CLOEXEC, 3);
  if (fds[0] < 0) return -1;
  int opened = 0;
  for (int i = 0; i < count; ++i) {
    int options = i + 1 == count ? flags : O_PATH | O_DIRECTORY;
    fds[i + 1] = openat(fds[i], components[i], options | O_NOFOLLOW | O_CLOEXEC);
    if (fds[i + 1] < 0) goto out;
    ++opened;
    if (fstat(fds[i + 1], &held)) goto out;
    /* O_PATH|O_NOFOLLOW opens the link itself; it does not fail on it. */
    if (S_ISLNK(held.st_mode)) { errno = ELOOP; goto out; }
    if (i + 1 < count && !S_ISDIR(held.st_mode)) { errno = ENOTDIR; goto out; }
  }
  for (int i = count; i > 0; --i) {
    if (fstat(fds[i], &held) ||
        fstatat(fds[i - 1], components[i - 1], &named, AT_SYMLINK_NOFOLLOW)) goto out;
    if (!cx_same_inode(&held, &named)) { errno = EXDEV; goto out; }
    if (S_ISDIR(held.st_mode)) {
      int up = openat(fds[i], "..", O_PATH | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
      if (up < 0) goto out;
      int failed = fstat(up, &named);
      saved = errno;
      close(up);
      errno = saved;
      if (failed || fstat(fds[i - 1], &parent)) goto out;
      if (!cx_same_inode(&named, &parent)) { errno = EXDEV; goto out; }
    }
  }
  result = fds[count];
  --opened;
out:
  saved = errno;
  for (int i = opened; i >= 0; --i) close(fds[i]);
  errno = saved;
  return result;
}
