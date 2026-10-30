// Controlled one-shot sync stall for an isolated kernel. No payloads are logged.
#define _GNU_SOURCE
#include <dlfcn.h>
#include <fcntl.h>
#include <limits.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

static void stamp(const char *root, const char *name) {
    char path[PATH_MAX], text[64];
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    snprintf(path, sizeof(path), "%s/%s", root, name);
    int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0600);
    if (fd >= 0) {
        int len = snprintf(text, sizeof(text), "%lld.%09ld\n", (long long)now.tv_sec, now.tv_nsec);
        if (write(fd, text, (size_t)len) < 0) { /* the drill will refuse missing evidence */ }
        close(fd);
    }
}

static void stall(int fd) {
    const char *root = getenv("CHARIOX_SYNC_STALL_ROOT");
    if (!root) return;
    char link[64], target[PATH_MAX], arm[PATH_MAX], begun[PATH_MAX];
    snprintf(link, sizeof(link), "/proc/self/fd/%d", fd);
    ssize_t len = readlink(link, target, sizeof(target) - 1);
    if (len <= 0) return;
    target[len] = 0;
    size_t root_len = strlen(root);
    if ((size_t)len <= root_len || strncmp(target, root, root_len) || target[root_len] != '/') return;
    if (!strstr(target, "/chariox/state.db")) return;
    snprintf(arm, sizeof(arm), "%s/stall-armed", root);
    snprintf(begun, sizeof(begun), "%s/stall-claimed", root);
    // rename is the one-shot claim, even if another syncing thread arrives.
    if (rename(arm, begun)) return;
    stamp(root, "stall-started");
    struct timespec remaining = { .tv_sec = 2, .tv_nsec = 0 };
    while (nanosleep(&remaining, &remaining) != 0) {}
    stamp(root, "stall-ended");
}

static int (*original_fsync)(int);
static int (*original_fdatasync)(int);
static pthread_once_t initialized = PTHREAD_ONCE_INIT;
static void initialize(void) {
    original_fsync = dlsym(RTLD_NEXT, "fsync");
    original_fdatasync = dlsym(RTLD_NEXT, "fdatasync");
}
int fsync(int fd) {
    pthread_once(&initialized, initialize);
    stall(fd);
    return original_fsync(fd);
}
int fdatasync(int fd) {
    pthread_once(&initialized, initialize);
    stall(fd);
    return original_fdatasync(fd);
}
