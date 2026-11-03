#ifndef CHARIOX_AWAIT_HPP
#define CHARIOX_AWAIT_HPP

#include "chariox/call.hpp"

namespace chariox {

// Lazy: creating or moving a future never starts an import. Only co_await does.
template <typename Start> class future {
public:
  future(cx_task *owner, Start start) noexcept : owner_(owner), start_(std::move(start)) {}
  future(const future &) = delete;
  future &operator=(const future &) = delete;
  future(future &&other) noexcept
      : owner_(std::exchange(other.owner_, nullptr)), start_(std::move(other.start_)) {}
  future &operator=(future &&) = delete;

  struct awaiter {
    cx_task *owner;
    Start start;
    awaiter(cx_task *task, Start operation) noexcept : owner(task), start(std::move(operation)) {}
    awaiter(const awaiter &) = delete;
    awaiter &operator=(const awaiter &) = delete;
    awaiter(awaiter &&other) noexcept
        : owner(std::exchange(other.owner, nullptr)), start(std::move(other.start)) {}
    awaiter &operator=(awaiter &&) = delete;
    bool await_ready() const noexcept { return false; }
    bool await_suspend(call::handle frame) noexcept {
      auto &promise = frame.promise();
      auto task = std::exchange(owner, nullptr);
      if (!task || task != promise.owner) {
        promise.poll = cx_task_fail(promise.owner, "SDK_STATE", "await a future once in its own invocation");
      } else {
        promise.poll = cx_task_await(task, start());
      }
      return promise.poll != CX_READY;
    }
    void await_resume() const noexcept {}
  };

  awaiter operator co_await() && noexcept {
    return {std::exchange(owner_, nullptr), std::move(start_)};
  }

private:
  cx_task *owner_;
  Start start_;
};

struct yield_awaitable {
  bool await_ready() const noexcept { return false; }
  void await_suspend(call::handle frame) const noexcept { frame.promise().poll = CX_YIELD; }
  void await_resume() const noexcept {}
};

struct chunk {
  const uint8_t *data = nullptr;
  std::size_t size = 0;
  bool end = false;
};

class byte_stream {
public:
  byte_stream(cx_task *owner, chariox_app_files_stream_u8_t handle, std::size_t capacity) noexcept
      : owner_(owner), reader_(cx_task_reader(owner, handle, capacity)) {}
  byte_stream(const byte_stream &) = delete;
  byte_stream &operator=(const byte_stream &) = delete;
  byte_stream(byte_stream &&other) noexcept : owner_(other.owner_), reader_(std::exchange(other.reader_, nullptr)) {}
  byte_stream &operator=(byte_stream &&) = delete;
  ~byte_stream() { close(); }
  void close() noexcept { if (reader_) cx_reader_close(reader_); }

  struct awaiter {
    cx_task *owner;
    cx_reader *reader;
    chunk result{};
    static cx_poll prepare(void *ptr) noexcept {
      auto &self = *static_cast<awaiter *>(ptr);
      if (!self.reader) return cx_task_fail(self.owner, "SDK_STATE", "stream was moved or could not be opened");
      auto status = cx_reader_next(self.reader, &self.result.data, &self.result.size);
      self.result.end = status == CX_READ_END;
      if (status == CX_READ_WAIT) return CX_WAIT;
      return status == CX_READ_ERROR ? CX_DONE : CX_READY;
    }
    bool await_ready() const noexcept { return false; }
    bool await_suspend(call::handle frame) noexcept {
      auto &promise = frame.promise();
      if (owner != promise.owner) {
        promise.poll = cx_task_fail(promise.owner, "SDK_STATE", "read a stream in its own invocation");
        return true;
      }
      promise.poll = prepare(this);
      if (promise.poll == CX_WAIT) { promise.waiting = this; promise.prepare = prepare; }
      return promise.poll != CX_READY;
    }
    chunk await_resume() const noexcept { return result; }
  };

  awaiter next() noexcept { return {owner_, reader_}; }

private:
  cx_task *owner_;
  cx_reader *reader_;
};

} // namespace chariox
#endif
