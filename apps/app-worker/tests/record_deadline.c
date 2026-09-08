#include "launcher.h"

#include <stdio.h>
#include <sys/socket.h>
#include <unistd.h>

int main(void) {
  int pair[2];
  if (socketpair(AF_UNIX, SOCK_STREAM, 0, pair)) return 1;
  int peer = pair[1];
  // Keep the peer above FD4 so dup2 cannot replace it.
  if (peer == CX_APP_CONTROL_FD) {
    peer = dup(peer);
    if (peer < 0) return 1;
  }
  if (dup2(pair[0], CX_APP_CONTROL_FD) != CX_APP_CONTROL_FD) return 1;
  struct cx_launch_record record = {0};
  int64_t started = cx_monotonic_ms();
  int result = cx_read_record(&record, started + 25);
  int64_t elapsed = cx_monotonic_ms() - started;
  close(peer);
  close(CX_APP_CONTROL_FD);
  if (result != CX_LAUNCH_TIMEOUT || elapsed < 20 || elapsed > 1000) return 1;
  puts("bounded_monotonic_record_deadline:ok");
  return 0;
}
