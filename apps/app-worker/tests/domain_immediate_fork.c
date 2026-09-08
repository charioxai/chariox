/* Tiny trusted fixture: fork immediately, before notifying the Rust supervisor.
 * The production entry must already have put both processes in its hard cgroup.
 */
#define _POSIX_C_SOURCE 200809L
#include <stdint.h>
#include <fcntl.h>
#include <unistd.h>
int main(void) {
  if (fcntl(5, F_GETFD) != -1 || fcntl(6, F_GETFD) != -1) return 1;
  const pid_t child = fork();
  if (child < 0) return 2;
  if (child > 0) {
    const int32_t identities[2] = {(int32_t)getpid(), (int32_t)child};
    if (write(3, identities, sizeof(identities)) != (ssize_t)sizeof(identities)) return 3;
  }
  for (;;) pause();
}
