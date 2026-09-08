/* Test-only ABI/lifecycle fixture. This is intentionally not a sandbox and is
 * never linked into a production launcher or selected by public API. */
#define _POSIX_C_SOURCE 200809L
#include "launcher.h"
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

extern char** environ;

static int send_all(int fd, const void* data, size_t size) {
  const char* bytes = data;
  while (size) {
    ssize_t count = write(fd, bytes, size);
    if (count < 0 && errno == EINTR) continue;
    if (count <= 0) return 1;
    bytes += count; size -= (size_t)count;
  }
  return 0;
}

int main(int argc, char** argv) {
  if (argc != 2 || getsid(0) != getpid() || getpgrp() != getpid()) return 80;
  if (!getenv("LANG") || strcmp(getenv("LANG"), "C.UTF-8") ||
      !getenv("TZ") || strcmp(getenv("TZ"), "UTC")) return 81;
  size_t entries = 0;
  for (char** item = environ; *item; ++item) ++entries;
  if (entries != 2 || getenv("HOME") || getenv("PATH")) return 82;
  if (fcntl(240, F_GETFD) != -1 || errno != EBADF) return 83;
  char byte;
  if (read(0, &byte, 1) != 0) return 84;
  if (!strcmp(argv[1], "early")) return 42;
  if (!strcmp(argv[1], "timeout")) for (;;) pause();
  int flags = fcntl(CX_APP_CONTROL_FD, F_GETFL);
  if (flags < 0 || fcntl(CX_APP_CONTROL_FD, F_SETFL, flags | O_NONBLOCK)) return 85;
  struct cx_launch_record record = {0};
  int64_t deadline = cx_monotonic_ms() + 5000;
  int result = cx_read_record(&record, deadline);
  if (result) return result;
  if (strcmp(record.generation, "7") || strcmp(record.installation, "installation_1") ||
      record.nofile != 128 || record.cpu_seconds != 30 || record.heap_mib != 64 ||
      record.v8_threads != 1 || record.max_file_bytes != 1048576 ||
      strcmp(record.bootstrap, "// trusted fixture bootstrap\n")) return 86;
  if (!strcmp(argv[1], "wrong")) strcpy(record.generation, "8");
  result = cx_ready_and_wait(&record, deadline);
  if (result) return result;
  close(CX_APP_CONTROL_FD);
  char marker[1200];
  int count = snprintf(marker, sizeof(marker), "%s/continued", record.roots[CX_DATA]);
  if (count < 0 || (size_t)count >= sizeof(marker)) return 87;
  int file = open(marker, O_WRONLY | O_CREAT | O_EXCL, 0600);
  if (file < 0) return 88;
  close(file);
  if (!strcmp(argv[1], "grow")) {
    /* Tiny test-only allocation after Continue; never allocate the production
     * ceiling on the developer's machine. Volatile stores commit each page. */
    size_t bytes = 8 * 1024 * 1024;
    volatile unsigned char* growth = malloc(bytes);
    if (!growth) return 92;
    for (size_t offset = 0; offset < bytes; offset += 4096) growth[offset] = 1;
  }
  if (!strcmp(argv[1], "flood")) {
    char block[8192]; memset(block, 'x', sizeof(block));
    for (;;) if (send_all(1, block, sizeof(block))) return 89;
  }
  static const char event[] = "{\"kind\":\"event\",\"version\":1,\"generation\":\"7\",\"name\":\"fixture.ready\",\"data\":{}}";
  size_t size = sizeof(event) - 1;
  unsigned char header[4] = {(unsigned char)(size >> 24), (unsigned char)(size >> 16),
      (unsigned char)(size >> 8), (unsigned char)size};
  if (send_all(3, header, 4) || send_all(3, event, size)) return 90;
  if (!strcmp(argv[1], "hang") || !strcmp(argv[1], "grow")) {
    close(1); close(2); for (;;) pause();
  }
  if (send_all(1, "fixture complete\n", 17) || send_all(2, "diagnostic\n", 11)) return 91;
  free(record.bootstrap);
  return 0;
}
