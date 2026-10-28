#include "chariox/app.hpp"
#include <initializer_list>

static bool register_app(chariox_app_app_types_app_error_t *) noexcept {
  for (const char *tool : {"echo", "counter", "wait"}) {
    auto name = cx_string(tool);
    chariox_app_registry_tool(&name);
  }
  return true;
}

static chariox::call run(chariox::context ctx) noexcept {
  const auto &request = ctx.request();
  if (request.kind == CX_LIFECYCLE) co_return ctx.ok();
  if (request.kind != CX_TOOL) co_return ctx.fail("NOT_REGISTERED", "no event or wake handler registered");
  if (cx_string_is(request.name, "echo"))
    co_return ctx.ok(reinterpret_cast<const char *>(request.input.ptr), request.input.len);
  if (cx_string_is(request.name, "wait")) {
    uint64_t ms;
    if (!cx_json_u64(cx_json_get(ctx.json(), cx_json_root(ctx.json()), "ms"), &ms) || ms > 30000)
      co_return ctx.fail("INVALID_INPUT", "ms must be an integer from 0 through 30000");
    co_await ctx.wait_for(ms * UINT64_C(1000000));
    co_return ctx.ok("{\"waited\":true}", 15);
  }
  if (!cx_string_is(request.name, "counter")) co_return ctx.fail("TOOL_NOT_FOUND", "unknown tool");

  chariox::owned<chariox_app_state_result_option_state_record_app_error_t,
                 chariox_app_state_result_option_state_record_app_error_free> got;
  co_await ctx.import([&]() noexcept { return chariox_app_state_get(cx_string("count"), &got.value); });
  if (got.value.is_err) co_return ctx.error(got.value.val.err);
  uint64_t count = 0;
  if (got.value.val.ok.is_some) {
    cx_json_doc *stored = nullptr;
    auto value = got.value.val.ok.val.value;
    auto parsed = cx_json_parse(reinterpret_cast<const char *>(value.ptr), value.len, &stored);
    bool valid = parsed == CX_JSON_OK && cx_json_u64(cx_json_root(stored), &count);
    cx_json_free(stored);
    if (!valid) co_return ctx.fail("INVALID_STATE", "count must be an interoperable nonnegative integer");
  }
  if (count == CX_JSON_MAX_SAFE_INTEGER) co_return ctx.fail("INVALID_STATE", "counter reached the JSON integer limit");
  uint64_t next = count + 1;
  char value[32];
  cx_json_write_u64(value, sizeof value, next);
  chariox_app_state_check_t check{cx_string("count"), {got.value.val.ok.is_some, got.value.val.ok.val.version}};
  chariox_app_state_write_t write{};
  write.tag = CHARIOX_APP_STATE_WRITE_PUT;
  write.val.put = {cx_string("count"), cx_string(value)};
  chariox_app_state_transaction_t tx{};
  tx.schema_version = 1;
  tx.checks = {&check, 1};
  tx.writes = {&write, 1};
  chariox::owned<chariox_app_state_result_committed_app_error_t,
                 chariox_app_state_result_committed_app_error_free> committed;
  // Conflicts are returned to the caller; accepted writes are never replayed.
  co_await ctx.import([&]() noexcept { return chariox_app_state_transact(&tx, &committed.value); });
  if (committed.value.is_err) co_return ctx.error(committed.value.val.err);
  if (committed.value.val.ok.revision > CX_JSON_MAX_SAFE_INTEGER)
    co_return ctx.fail("INVALID_STATE", "revision exceeds JSON integer range");
  char result[128];
  std::memcpy(result, "{\"count\":", 9);
  auto len = 9 + cx_json_write_u64(result + 9, sizeof result - 9, next);
  std::memcpy(result + len, ",\"revision\":", 12); len += 12;
  len += cx_json_write_u64(result + len, sizeof result - len, committed.value.val.ok.revision);
  result[len++] = '}';
  co_return ctx.ok(result, len);
}

extern "C" const cx_app cx_application = chariox::application<register_app, run>();
