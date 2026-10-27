#include "launcher.h"

// Fault injection for the native-probe harness only. Never link into a release
// target. Installed Apps cannot choose a policy or enable this fixture.
int cx_apply_platform_sandbox(const struct cx_launch_record* record) {
  (void)record;
  return 0;
}
