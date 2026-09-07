#include <errno.h>
#include <fcntl.h>
#include <linux/sched.h>
#include <seccomp.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static void fail(const char *message, int error) {
  fprintf(stderr, "%s: %s\n", message, strerror(error < 0 ? -error : error));
  exit(EXIT_FAILURE);
}

int main(int argc, char **argv) {
  if (argc != 2) {
    fprintf(stderr, "usage: %s OUTPUT\n", argv[0]);
    return EXIT_FAILURE;
  }

  scmp_filter_ctx filter = seccomp_init(SCMP_ACT_ALLOW);
  if (filter == NULL) fail("seccomp_init", ENOMEM);

  int error = seccomp_rule_add(
      filter,
      SCMP_ACT_ERRNO(EPERM),
      SCMP_SYS(unshare),
      1,
      SCMP_A0(SCMP_CMP_MASKED_EQ, CLONE_NEWUSER, CLONE_NEWUSER));
  if (error < 0) fail("seccomp unshare rule", error);

  error = seccomp_rule_add(
      filter,
      SCMP_ACT_ERRNO(EPERM),
      SCMP_SYS(clone),
      1,
      SCMP_A0(SCMP_CMP_MASKED_EQ, CLONE_NEWUSER, CLONE_NEWUSER));
  if (error < 0) fail("seccomp clone rule", error);

  // Classic seccomp cannot inspect the structure behind clone3's pointer.
  // ENOSYS makes libc and runtimes fall back to clone without breaking
  // ordinary provider threads, whose flags are checked by the rule above.
  error = seccomp_rule_add(filter, SCMP_ACT_ERRNO(ENOSYS), SCMP_SYS(clone3), 0);
  if (error < 0) fail("seccomp clone3 rule", error);

  int output = open(argv[1], O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0444);
  if (output < 0) fail("open seccomp output", errno);
  error = seccomp_export_bpf(filter, output);
  if (error < 0) fail("seccomp_export_bpf", error);
  if (close(output) != 0) fail("close seccomp output", errno);
  seccomp_release(filter);
  return EXIT_SUCCESS;
}
