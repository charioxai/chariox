#ifndef CHARIOX_CALL_HPP
#define CHARIOX_CALL_HPP

#include "chariox/task.h"
#include <coroutine>
#include <cstdlib>
#include <utility>

#if defined(__cpp_exceptions) || defined(_CPPUNWIND)
#error "The Chariox C++ App SDK requires -fno-exceptions at every App translation unit"
#endif

namespace chariox {

struct completion {};

// One invocation frame, owned by the C task. This is not a schedulable task.
class call {
public:
  struct promise_type;
  using handle = std::coroutine_handle<promise_type>;

  struct promise_type {
    cx_task *owner = nullptr;
    cx_poll poll = CX_DONE;
    void *waiting = nullptr;
    cx_poll (*prepare)(void *) = nullptr;

    static void *operator new(std::size_t size) noexcept { return ::malloc(size); }
    static void operator delete(void *ptr, std::size_t) noexcept { ::free(ptr); }
    static call get_return_object_on_allocation_failure() noexcept { return {}; }
    call get_return_object() noexcept { return call(handle::from_promise(*this)); }
    std::suspend_always initial_suspend() noexcept { return {}; }
    // The C export borrows the result until it has returned it to the host.
    std::suspend_always final_suspend() noexcept { return {}; }
    void return_value(completion) noexcept { poll = CX_DONE; }
    void unhandled_exception() noexcept {
      poll = cx_task_fail(owner, "SDK_STATE", "exceptions are disabled for App exports");
    }
  };

  call() noexcept = default;
  call(const call &) = delete;
  call &operator=(const call &) = delete;
  call(call &&other) noexcept : frame_(std::exchange(other.frame_, {})) {}
  call &operator=(call &&) = delete;
  ~call() { if (frame_) frame_.destroy(); }

  void *release() noexcept { return std::exchange(frame_, {}).address(); }

private:
  explicit call(handle frame) noexcept : frame_(frame) {}
  handle frame_{};
};

// Generated result structs are zero-initialized and freed even after cancellation.
template <typename T, void (*Free)(T *)> struct owned {
  T value{};
  owned() noexcept = default;
  owned(const owned &) = delete;
  owned &operator=(const owned &) = delete;
  ~owned() { Free(&value); }
};

} // namespace chariox
#endif
