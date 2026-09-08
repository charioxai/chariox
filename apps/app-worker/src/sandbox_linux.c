#if defined(__linux__)
#define _GNU_SOURCE
#include "launcher.h"

#include <errno.h>
#include <linux/audit.h>
#include <linux/capability.h>
#include <linux/filter.h>
#include <linux/sched.h>
#include <linux/seccomp.h>
#include <stddef.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/prctl.h>
#include <sys/statvfs.h>
#include <sys/syscall.h>
#include <unistd.h>

#if defined(__x86_64__)
#define CX_AUDIT_ARCH AUDIT_ARCH_X86_64
#elif defined(__aarch64__)
#define CX_AUDIT_ARCH AUDIT_ARCH_AARCH64
#else
#error Unsupported Linux App worker architecture
#endif

#define CX_DENY (SECCOMP_RET_ERRNO | EPERM)
#define CX_ALLOW(number) BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, (number), 0, 1), \
                         BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW)
#define CX_ARG_LOW(index) (offsetof(struct seccomp_data, args) + (index) * 8)
#define CX_ARG_HIGH(index) (CX_ARG_LOW(index) + 4)

static int check_namespace_mounts(const struct cx_launch_record* record) {
  static const char* roots[] = {"/app/package", "/app/data", "/app/tmp", "/runtime"};
  struct statvfs mount;
  if (statvfs("/", &mount) || !(mount.f_flag & ST_RDONLY)) return -1;
  for (size_t i = 0; i < CX_ROOT_COUNT; ++i) {
    if (strcmp(record->roots[i], roots[i]) || statvfs(roots[i], &mount)) return -1;
    unsigned long required = ST_NOSUID | ST_NODEV;
    if (i != CX_RUNTIME) required |= ST_NOEXEC;
    if (i == CX_PACKAGE || i == CX_RUNTIME) required |= ST_RDONLY;
    if ((mount.f_flag & required) != required) return -1;
  }
  // Namespace setup and cgroup attachment are the supervisor's responsibility;
  // seccomp is not a substitute for that independently validated setup. Reject
  // residual capabilities even inside the worker's private user namespace.
  struct __user_cap_header_struct header = {_LINUX_CAPABILITY_VERSION_3, 0};
  struct __user_cap_data_struct data[2] = {{0}};
  if (syscall(SYS_capget, &header, data)) return -1;
  for (size_t i = 0; i < 2; ++i)
    if (data[i].effective || data[i].permitted || data[i].inheritable) return -1;
  return 0;
}

int cx_apply_platform_sandbox(const struct cx_launch_record* record) {
  if (check_namespace_mounts(record) || prctl(PR_SET_DUMPABLE, 0, 0, 0, 0) ||
      prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0)) return CX_LAUNCH_SANDBOX;

  const unsigned int thread_required = CLONE_VM | CLONE_FS | CLONE_FILES |
      CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM;
  const unsigned int thread_allowed = thread_required | CLONE_SETTLS |
      CLONE_PARENT_SETTID | CLONE_CHILD_SETTID | CLONE_CHILD_CLEARTID;
  const struct sock_filter filter[] = {
      BPF_STMT(BPF_LD | BPF_W | BPF_ABS, offsetof(struct seccomp_data, arch)),
      BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, CX_AUDIT_ARCH, 1, 0),
      BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS),
      BPF_STMT(BPF_LD | BPF_W | BPF_ABS, offsetof(struct seccomp_data, nr)),
#ifdef __NR_clone3
      // glibc falls back to the inspectable clone ABI on ENOSYS. BPF cannot
      // safely inspect clone3's userspace structure and must never allow it.
      BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, __NR_clone3, 0, 1),
      BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ERRNO | ENOSYS),
#endif
      BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, __NR_clone, 0, 13),
      BPF_STMT(BPF_LD | BPF_W | BPF_ABS, CX_ARG_HIGH(0)),
      BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, 0, 1, 0),
      BPF_STMT(BPF_RET | BPF_K, CX_DENY),
      BPF_STMT(BPF_LD | BPF_W | BPF_ABS, CX_ARG_LOW(0)),
      BPF_STMT(BPF_ALU | BPF_AND | BPF_K, ~thread_allowed),
      BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, 0, 1, 0),
      BPF_STMT(BPF_RET | BPF_K, CX_DENY),
      BPF_STMT(BPF_LD | BPF_W | BPF_ABS, CX_ARG_LOW(0)),
      BPF_STMT(BPF_ALU | BPF_AND | BPF_K, thread_required),
      BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, thread_required, 1, 0),
      BPF_STMT(BPF_RET | BPF_K, CX_DENY),
      BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
      BPF_STMT(BPF_LD | BPF_W | BPF_ABS, offsetof(struct seccomp_data, nr)),
      // glibc getrlimit uses prlimit64. Permit self queries only; a non-NULL
      // new_limit could alter native ceilings after the supervisor handshake.
      BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, __NR_prlimit64, 0, 14),
      BPF_STMT(BPF_LD | BPF_W | BPF_ABS, CX_ARG_HIGH(0)),
      BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, 0, 1, 0),
      BPF_STMT(BPF_RET | BPF_K, CX_DENY),
      BPF_STMT(BPF_LD | BPF_W | BPF_ABS, CX_ARG_LOW(0)),
      BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, 0, 2, 0),
      BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, (unsigned int)getpid(), 1, 0),
      BPF_STMT(BPF_RET | BPF_K, CX_DENY),
      BPF_STMT(BPF_LD | BPF_W | BPF_ABS, CX_ARG_HIGH(2)),
      BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, 0, 1, 0),
      BPF_STMT(BPF_RET | BPF_K, CX_DENY),
      BPF_STMT(BPF_LD | BPF_W | BPF_ABS, CX_ARG_LOW(2)),
      BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, 0, 1, 0),
      BPF_STMT(BPF_RET | BPF_K, CX_DENY),
      BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
      BPF_STMT(BPF_LD | BPF_W | BPF_ABS, offsetof(struct seccomp_data, nr)),
      // Permit only pthread-directed signals within this worker's process.
      BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, __NR_tgkill, 0, 4),
      BPF_STMT(BPF_LD | BPF_W | BPF_ABS, CX_ARG_LOW(0)),
      BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, (unsigned int)getpid(), 1, 0),
      BPF_STMT(BPF_RET | BPF_K, CX_DENY),
      BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
      BPF_STMT(BPF_LD | BPF_W | BPF_ABS, offsetof(struct seccomp_data, nr)),
      BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, __NR_ioctl, 0, 5),
      BPF_STMT(BPF_LD | BPF_W | BPF_ABS, CX_ARG_LOW(1)),
      BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, FIONREAD, 2, 0),
      BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, FIONBIO, 1, 0),
      BPF_STMT(BPF_RET | BPF_K, CX_DENY),
      BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
      BPF_STMT(BPF_LD | BPF_W | BPF_ABS, offsetof(struct seccomp_data, nr)),
      CX_ALLOW(__NR_read), CX_ALLOW(__NR_write), CX_ALLOW(__NR_readv), CX_ALLOW(__NR_writev),
      CX_ALLOW(__NR_pread64), CX_ALLOW(__NR_pwrite64), CX_ALLOW(__NR_preadv), CX_ALLOW(__NR_pwritev),
      CX_ALLOW(__NR_copy_file_range), CX_ALLOW(__NR_sendfile),
      CX_ALLOW(__NR_close), CX_ALLOW(__NR_close_range), CX_ALLOW(__NR_fcntl), CX_ALLOW(__NR_dup),
      CX_ALLOW(__NR_dup3), CX_ALLOW(__NR_lseek), CX_ALLOW(__NR_fstat), CX_ALLOW(__NR_statx),
      CX_ALLOW(__NR_openat), CX_ALLOW(__NR_readlinkat), CX_ALLOW(__NR_getdents64),
      CX_ALLOW(__NR_faccessat), CX_ALLOW(__NR_faccessat2), CX_ALLOW(__NR_newfstatat),
      CX_ALLOW(__NR_mkdirat), CX_ALLOW(__NR_unlinkat), CX_ALLOW(__NR_renameat),
      CX_ALLOW(__NR_renameat2), CX_ALLOW(__NR_linkat), CX_ALLOW(__NR_symlinkat),
      CX_ALLOW(__NR_fchmod), CX_ALLOW(__NR_fchmodat), CX_ALLOW(__NR_umask),
      CX_ALLOW(__NR_ftruncate), CX_ALLOW(__NR_truncate), CX_ALLOW(__NR_fallocate),
      CX_ALLOW(__NR_utimensat), CX_ALLOW(__NR_fsync), CX_ALLOW(__NR_fdatasync),
      CX_ALLOW(__NR_statfs), CX_ALLOW(__NR_fstatfs), CX_ALLOW(__NR_chdir), CX_ALLOW(__NR_getcwd),
      CX_ALLOW(__NR_mmap), CX_ALLOW(__NR_mprotect), CX_ALLOW(__NR_munmap), CX_ALLOW(__NR_mremap),
      CX_ALLOW(__NR_madvise), CX_ALLOW(__NR_msync), CX_ALLOW(__NR_brk),
      CX_ALLOW(__NR_rt_sigaction), CX_ALLOW(__NR_rt_sigprocmask), CX_ALLOW(__NR_rt_sigreturn),
      CX_ALLOW(__NR_sigaltstack), CX_ALLOW(__NR_rt_sigtimedwait),
      CX_ALLOW(__NR_getpid), CX_ALLOW(__NR_getppid), CX_ALLOW(__NR_gettid),
      CX_ALLOW(__NR_getuid), CX_ALLOW(__NR_geteuid), CX_ALLOW(__NR_getgid), CX_ALLOW(__NR_getegid),
      CX_ALLOW(__NR_getgroups), CX_ALLOW(__NR_uname), CX_ALLOW(__NR_sysinfo),
      CX_ALLOW(__NR_getrandom), CX_ALLOW(__NR_clock_gettime), CX_ALLOW(__NR_clock_getres),
      CX_ALLOW(__NR_gettimeofday), CX_ALLOW(__NR_nanosleep), CX_ALLOW(__NR_clock_nanosleep),
      CX_ALLOW(__NR_futex), CX_ALLOW(__NR_set_tid_address), CX_ALLOW(__NR_set_robust_list),
      CX_ALLOW(__NR_rseq), CX_ALLOW(__NR_sched_yield), CX_ALLOW(__NR_sched_getaffinity),
      CX_ALLOW(__NR_sched_getparam), CX_ALLOW(__NR_sched_getscheduler), CX_ALLOW(__NR_getrusage),
      CX_ALLOW(__NR_getrlimit), CX_ALLOW(__NR_pipe2), CX_ALLOW(__NR_eventfd2),
      CX_ALLOW(__NR_inotify_init1), CX_ALLOW(__NR_inotify_add_watch), CX_ALLOW(__NR_inotify_rm_watch),
      CX_ALLOW(__NR_epoll_create1), CX_ALLOW(__NR_epoll_ctl), CX_ALLOW(__NR_epoll_pwait),
      CX_ALLOW(__NR_ppoll), CX_ALLOW(__NR_pselect6),
      CX_ALLOW(__NR_getsockopt), CX_ALLOW(__NR_getsockname), CX_ALLOW(__NR_getpeername),
      CX_ALLOW(__NR_shutdown), CX_ALLOW(__NR_exit), CX_ALLOW(__NR_exit_group),
#if defined(__x86_64__)
      CX_ALLOW(__NR_open), CX_ALLOW(__NR_stat), CX_ALLOW(__NR_lstat), CX_ALLOW(__NR_access),
      CX_ALLOW(__NR_readlink), CX_ALLOW(__NR_mkdir), CX_ALLOW(__NR_rmdir), CX_ALLOW(__NR_unlink),
      CX_ALLOW(__NR_rename), CX_ALLOW(__NR_link), CX_ALLOW(__NR_symlink), CX_ALLOW(__NR_chmod),
      CX_ALLOW(__NR_dup2), CX_ALLOW(__NR_pipe), CX_ALLOW(__NR_poll), CX_ALLOW(__NR_select),
      CX_ALLOW(__NR_epoll_wait), CX_ALLOW(__NR_arch_prctl), CX_ALLOW(__NR_time),
#endif
      // No socket/socketpair/connect/bind/listen/accept, exec/fork, ptrace,
      // process_vm, memfd, mounts/namespaces, keyrings, bpf, perf, or io_uring.
      // Unknown/new syscalls remain denied until reviewed and tested.
      BPF_STMT(BPF_RET | BPF_K, CX_DENY),
  };
  struct sock_fprog program = {
      (unsigned short)(sizeof(filter) / sizeof(filter[0])), (struct sock_filter*)filter};
  return prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, &program) == 0 ? 0 : CX_LAUNCH_SANDBOX;
}
#endif
