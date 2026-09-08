#include "runtime.h"

#include <node.h>
#include <node_version.h>
#include <v8.h>

#include <atomic>
#include <cstring>
#include <memory>
#include <string>
#include <string_view>
#include <vector>

static_assert(NODE_MAJOR_VERSION == 24 && NODE_MINOR_VERSION == 20 &&
              NODE_PATCH_VERSION == 0 && NODE_MODULE_VERSION == 137,
              "Node headers must match runtime.lock.json exactly");

namespace {
std::atomic<bool> started{false};

int run_environment(node::MultiIsolatePlatform* platform,
                    const node::InitializationResult& initialization,
                    std::string_view bootstrap) {
  std::vector<std::string> errors;
  auto setup = node::CommonEnvironmentSetup::Create(
      platform, &errors, initialization.args(), initialization.exec_args());
  if (!setup) return CHARIOX_RUNTIME_INITIALIZATION_FAILED;

  v8::Isolate* isolate = setup->isolate();
  node::Environment* environment = setup->env();
  int requested_exit = 0;
  bool exited = false;
  node::SetProcessExitHandler(environment, [&](node::Environment* env, int code) {
    requested_exit = code;
    exited = true;
    node::Stop(env);
  });
  int result = CHARIOX_RUNTIME_BOOTSTRAP_FAILED;
  {
    v8::Locker locker(isolate);
    v8::Isolate::Scope isolate_scope(isolate);
    v8::HandleScope handles(isolate);
    v8::Context::Scope context_scope(setup->context());
    if (!node::LoadEnvironment(environment, bootstrap).IsEmpty()) {
      result = node::SpinEventLoop(environment).FromMaybe(
          CHARIOX_RUNTIME_BOOTSTRAP_FAILED);
    }
    node::Stop(environment);
  }
  return exited ? requested_exit : result;
}
int run_runtime(const chariox_runtime_config* config) {
  if (config == nullptr || config->abi_version != 1 || config->argc == 0 ||
      config->argc > 64 || config->argv == nullptr || config->v8_threads == 0 ||
      config->v8_threads > 4 || config->trusted_bootstrap == nullptr ||
      config->trusted_bootstrap_length == 0 ||
      config->trusted_bootstrap_length > 256 * 1024) {
    return CHARIOX_RUNTIME_INVALID_CONFIG;
  }
  std::vector<std::string> arguments;
  for (size_t index = 0; index < config->argc; ++index) {
    if (config->argv[index] == nullptr ||
        strnlen(config->argv[index], 4097) > 4096) {
      return CHARIOX_RUNTIME_INVALID_CONFIG;
    }
    arguments.emplace_back(config->argv[index]);
  }
  if (started.exchange(true)) return CHARIOX_RUNTIME_ALREADY_STARTED;

  auto initialization = node::InitializeOncePerProcess(arguments, {
      node::ProcessInitializationFlags::kNoInitializeV8,
      node::ProcessInitializationFlags::kNoInitializeNodeV8Platform,
      node::ProcessInitializationFlags::kDisableNodeOptionsEnv,
      node::ProcessInitializationFlags::kNoParseGlobalDebugVariables,
      node::ProcessInitializationFlags::kNoAdjustResourceLimits,
      node::ProcessInitializationFlags::kNoUseLargePages,
  });
  if (initialization->early_return()) {
    node::TearDownOncePerProcess();
    return CHARIOX_RUNTIME_INITIALIZATION_FAILED;
  }

  auto platform = node::MultiIsolatePlatform::Create(config->v8_threads);
  v8::V8::InitializePlatform(platform.get());
  if (!v8::V8::Initialize()) {
    v8::V8::DisposePlatform();
    node::TearDownOncePerProcess();
    return CHARIOX_RUNTIME_INITIALIZATION_FAILED;
  }
  const int result = run_environment(
      platform.get(), *initialization,
      std::string_view(config->trusted_bootstrap,
                       config->trusted_bootstrap_length));
  v8::V8::Dispose();
  v8::V8::DisposePlatform();
  node::TearDownOncePerProcess();
  return result;
}
}  // namespace

extern "C" const char* chariox_app_runtime_node_version(void) {
  return NODE_VERSION_STRING;
}

extern "C" int chariox_app_runtime_run(const chariox_runtime_config* config) {
  try {
    return run_runtime(config);
  } catch (...) {
    // C++ exceptions must not cross the launcher's C ABI. The launcher exits
    // this process on return; a partly initialized runtime cannot be reused.
    return CHARIOX_RUNTIME_INTERNAL_FAILURE;
  }
}
