#ifndef CHARIOX_APP_HPP
#define CHARIOX_APP_HPP

#include "chariox/await.hpp"
#include <cstring>

namespace chariox {

class context {
public:
  explicit context(cx_task *owner) noexcept : owner_(owner) {}
  const cx_request &request() const noexcept { return *cx_task_request(owner_); }
  const cx_json_doc *json() const noexcept { return cx_task_json(owner_); }
  // The supervisor resolves the C-owned cancellation future at this deadline.
  uint64_t deadline_ms() const noexcept { return request().context.deadline_ms; }

  template <typename Start> future<Start> import(Start start) const noexcept {
    return {owner_, std::move(start)};
  }
  yield_awaitable yield() const noexcept { return {}; }
  auto wait_for(uint64_t nanoseconds) const noexcept {
    return import([nanoseconds]() noexcept { return wasi_clocks_0_3_0_monotonic_clock_wait_for(nanoseconds); });
  }
  // Monotonic marks and the invocation's wall-clock deadline are different units.
  auto wait_until(uint64_t mark) const noexcept {
    return import([mark]() noexcept { return wasi_clocks_0_3_0_monotonic_clock_wait_until(mark); });
  }
  byte_stream stream(chariox_app_files_stream_u8_t handle, std::size_t capacity) const noexcept {
    return {owner_, handle, capacity};
  }

  completion ok(const char *json = nullptr, std::size_t size = 0) const noexcept {
    if (json && size > CX_JSON_MAX_BYTES) {
      cx_task_fail(owner_, "INVALID_JSON", "result must be bounded interoperable JSON");
      return {};
    }
    const char *copy = json ? copy_text(json, size) : nullptr;
    if (!json || copy) cx_task_ok(owner_, copy, size);
    return {};
  }
  completion fail(const char *code, const char *message) const noexcept {
    auto c = copy_text(code, std::strlen(code));
    auto m = copy_text(message, std::strlen(message));
    if (c && m) cx_task_fail(owner_, c, m);
    return {};
  }
  completion error(const chariox_app_app_types_app_error_t &error) const noexcept {
    auto c = copy_text(reinterpret_cast<const char *>(error.code.ptr), error.code.len);
    auto m = copy_text(reinterpret_cast<const char *>(error.message.ptr), error.message.len);
    if (c && m) {
      chariox_app_app_types_app_error_t copy = error;
      copy.code = {reinterpret_cast<uint8_t *>(c), error.code.len};
      copy.message = {reinterpret_cast<uint8_t *>(m), error.message.len};
      cx_task_error(owner_, &copy);
    }
    return {};
  }

private:
  cx_task *owner_;
  char *copy_text(const char *text, std::size_t size) const noexcept {
    // fail() uses NUL-terminated strings; error()/ok() retain explicit lengths.
    if (size == SIZE_MAX) { cx_task_fail(owner_, "APP_MEMORY_LIMIT", "result is too large"); return nullptr; }
    auto copy = static_cast<char *>(cx_task_alloc(owner_, size + 1, nullptr));
    if (copy) { if (size) std::memcpy(copy, text, size); copy[size] = 0; }
    return copy;
  }
};

template <bool (*Register)(chariox_app_app_types_app_error_t *), call (*Run)(context)>
constexpr cx_app application() noexcept {
  return {
    sizeof(void *), Register,
    [](cx_task *owner) noexcept -> cx_poll {
      auto &slot = *static_cast<void **>(cx_task_state(owner));
      if (!slot) {
        slot = Run(context(owner)).release();
        if (!slot) return cx_task_fail(owner, "APP_MEMORY_LIMIT", "coroutine allocation failed");
      }
      auto frame = call::handle::from_address(slot);
      auto &promise = frame.promise();
      promise.owner = owner;
      if (promise.prepare) {
        promise.poll = promise.prepare(promise.waiting);
        if (promise.poll != CX_READY) return promise.poll;
        promise.prepare = nullptr;
        promise.waiting = nullptr;
      }
      frame.resume();
      return promise.poll;
    },
    [](void *state) noexcept {
      auto slot = *static_cast<void **>(state);
      if (slot) call::handle::from_address(slot).destroy();
    },
  };
}

} // namespace chariox
#endif
