#include "chariox/app.hpp"
#include <initializer_list>

static void note(const char *message, const char *name) noexcept {
  auto msg=cx_string(message), fields=cx_string(name);
  chariox_app_log_write(CHARIOX_APP_LOG_LEVEL_INFO,&msg,&fields);
}
static bool register_app(chariox_app_app_types_app_error_t *) noexcept {
  for (auto name : {"sdk-stream","sdk-yield","sdk-cancel-start","sdk-held","sdk-before-start"}) {
    auto value=cx_string(name); chariox_app_registry_tool(&value);
  }
  return true;
}
struct lifetime {
  const char *name;
  ~lifetime() noexcept { note("sdk.destroy",name); }
};
static chariox::call run(chariox::context ctx) noexcept {
  auto name=ctx.request().name;
  const char *label=cx_string_is(name,"sdk-yield") ? "\"sdk-yield\"" :
    cx_string_is(name,"sdk-cancel-start") ? "\"sdk-cancel-start\"" :
    cx_string_is(name,"sdk-held") ? "\"sdk-held\"" : "\"sdk-stream\"";
  lifetime guard{label}; note("sdk.start",label);
  if (cx_string_is(name,"sdk-yield")) for (;;) co_await ctx.yield();
  chariox::owned<chariox_app_state_result_option_state_record_app_error_t,
                 chariox_app_state_result_option_state_record_app_error_free> held;
  if (cx_string_is(name,"sdk-cancel-start")) {
    // Force the public C dispatcher's pending-import cleanup before yielding.
    co_await ctx.import([&]() noexcept {
      auto status=chariox_app_state_get(cx_string("must-not-start"),&held.value);
      ctx.fail("EXPECTED","cancel import before yielding");
      return status;
    });
    co_return ctx.fail("UNEXPECTED","cancelled import resumed");
  }
  if (cx_string_is(name,"sdk-held")) {
    co_await ctx.import([&]() noexcept { return chariox_app_state_get(cx_string("held"),&held.value); });
    co_return ctx.ok("null",4);
  }
  chariox_app_http_fetch_args_t request{};
  request.req.url=cx_string("https://example.com/"); request.req.method=cx_string("GET");
  chariox::owned<chariox_app_http_result_tuple2_response_head_stream_u8_app_error_t,
                 chariox_app_http_result_tuple2_response_head_stream_u8_app_error_free> response;
  co_await ctx.import([&]() noexcept { return chariox_app_http_fetch(&request,&response.value); });
  if (response.value.is_err) co_return ctx.error(response.value.val.err);
  auto reader=ctx.stream(response.value.val.ok.f1,4096);
  size_t bytes=0;
  for (;;) {
    auto chunk=co_await reader.next();
    if (chunk.end) break;
    if (chunk.size!=3 || std::memcmp(chunk.data,"abc",3)) co_return ctx.fail("BYTES","wrong final bytes");
    bytes+=chunk.size;
  }
  reader.close(); reader.close();
  if (bytes!=3) co_return ctx.fail("BYTES","lost or duplicated final bytes");
  co_return ctx.ok("{\"bytes\":3}",11);
}
extern "C" const cx_app cx_application=chariox::application<register_app,run>();
