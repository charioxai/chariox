#if defined(__APPLE__)
#include "launcher.h"

#include <sandbox.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

// Chromium uses these exported Seatbelt entry points with parameterized
// profiles; Apple's public header only declares the deprecated named API.
// https://github.com/chromium/chromium/blob/main/sandbox/mac/seatbelt.cc
extern int sandbox_init_with_parameters(const char*, uint64_t, const char* const[], char**);
extern int sandbox_check(pid_t, const char*, int, ...);

static const char profile[] =
    "(version 1)\n"
    "(deny default)\n"
    "(allow signal (target self))\n"
    "(allow process-info-pidinfo (target self))\n"
    "(allow file-read* (subpath (param \"PACKAGE\")) (subpath (param \"RUNTIME\")) "
    "(subpath (param \"DATA\")) (subpath (param \"TMP\")))\n"
    "(allow file-write* (subpath (param \"DATA\")) (subpath (param \"TMP\")))\n"
    // Direct executable file mappings exclude package/data/tmp. Seatbelt alone
    // does not prove denial of a later mprotect transition on an existing map.
    // The signed hardened-runtime/JIT fixture must test that stronger contract.
    "(allow file-read* file-map-executable (subpath (param \"RUNTIME\")) "
    "(subpath \"/usr/lib\") (subpath \"/System/Library/dyld\") "
    "(subpath \"/System/Cryptexes/OS\") "
    "(subpath \"/System/Volumes/Preboot/Cryptexes/OS\"))\n"
    "(allow file-read-data (literal \"/dev/null\") (literal \"/dev/random\") "
    "(literal \"/dev/urandom\"))\n"
    "(allow sysctl-read (sysctl-name \"hw.activecpu\") (sysctl-name \"hw.ncpu\") "
    "(sysctl-name \"hw.logicalcpu_max\") (sysctl-name \"hw.physicalcpu_max\") "
    "(sysctl-name \"hw.memsize\") (sysctl-name \"hw.pagesize\") "
    "(sysctl-name \"hw.cputype\") (sysctl-name \"hw.cpusubtype\") "
    "(sysctl-name-prefix \"hw.optional.\") (sysctl-name \"kern.osrelease\") "
    "(sysctl-name \"kern.ostype\") (sysctl-name \"kern.osversion\"))\n";

static bool append_literal(char* buffer, size_t capacity, size_t* used,
                           const char* path, size_t length) {
  const char* prefix = "(literal \"";
  size_t required = strlen(prefix) + length * 2 + 3;
  if (required >= capacity - *used) return false;
  memcpy(buffer + *used, prefix, strlen(prefix));
  *used += strlen(prefix);
  for (size_t i = 0; i < length; ++i) {
    if (path[i] == '\\' || path[i] == '"') buffer[(*used)++] = '\\';
    buffer[(*used)++] = path[i];
  }
  memcpy(buffer + *used, "\") ", 3);
  *used += 3;
  return true;
}

int cx_apply_platform_sandbox(const struct cx_launch_record* record) {
  // Only metadata of the exact ancestors is readable, not arbitrary home or
  // workspace metadata. A bounded buffer contains the worst-case root depths.
  size_t capacity = 4 * CX_APP_PATH_MAX * CX_APP_PATH_MAX + sizeof(profile);
  char* policy = malloc(capacity);
  if (!policy) return CX_LAUNCH_SANDBOX;
  size_t used = sizeof(profile) - 1;
  memcpy(policy, profile, used);
  const char* metadata_rule = "(allow file-read-metadata ";
  memcpy(policy + used, metadata_rule, strlen(metadata_rule));
  used += strlen(metadata_rule);
  bool valid = append_literal(policy, capacity, &used, "/", 1);
  for (size_t root = 0; valid && root < CX_ROOT_COUNT; ++root) {
    const char* path = record->roots[root];
    for (size_t i = 1; valid && path[i]; ++i)
      if (path[i] == '/') valid = append_literal(policy, capacity, &used, path, i);
  }
  if (!valid || used + 3 >= capacity) { free(policy); return CX_LAUNCH_SANDBOX; }
  memcpy(policy + used, ")\n", 3);
  const char* parameters[] = {
      "PACKAGE", record->roots[CX_PACKAGE], "DATA", record->roots[CX_DATA],
      "TMP", record->roots[CX_TMP], "RUNTIME", record->roots[CX_RUNTIME], NULL};
  char* error = NULL;
  int result = sandbox_init_with_parameters(policy, 0, parameters, &error);
  free(policy);
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
  sandbox_free_error(error);
#pragma clang diagnostic pop
  // SANDBOX_FILTER_NONE=0. Never serialize the compiler diagnostic: it can
  // contain host paths. The supervisor records a stable startup failure.
  return result == 0 && sandbox_check(getpid(), NULL, 0) == 1 ? 0 : CX_LAUNCH_SANDBOX;
}
#endif
