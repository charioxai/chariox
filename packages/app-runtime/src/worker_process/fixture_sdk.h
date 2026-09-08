/* Fixed test-only SDK shim. This is neither an App nor a Node/sandbox proof.
 * It matches the trusted Rust fixture response serialization, not arbitrary JSON.
 */
#include <poll.h>
#include <time.h>

static int fixture_sdk_mode(const char* mode) {
  return !strcmp(mode, "sdk_ready") || !strcmp(mode, "sdk_wrong_handlers") ||
      !strcmp(mode, "sdk_no_report") || !strcmp(mode, "sdk_broker_call") || !strcmp(mode, "sdk_tool") ||
      !strcmp(mode, "sdk_other_installation") || !strcmp(mode, "sdk_files") ||
      !strcmp(mode, "sdk_health") || !strcmp(mode, "sdk_bad_health") || !strcmp(mode, "sdk_http") || !strcmp(mode, "sdk_http_paused");
}

static int fixture_sdk_io(void* buffer, size_t size, int writing, int64_t deadline) {
  unsigned char* bytes = buffer;
  while (size) {
    int64_t remaining = deadline - cx_monotonic_ms();
    if (remaining <= 0) return -1;
    struct pollfd descriptor = {3, writing ? POLLOUT : POLLIN, 0};
    int status = poll(&descriptor, 1, remaining > 1000 ? 1000 : (int)remaining);
    if (status < 0 && errno == EINTR) continue;
    if (status < 0) return -1;
    if (!status) continue;
    ssize_t count = writing ? write(3, bytes, size) : read(3, bytes, size);
    if (count < 0 && (errno == EINTR || errno == EAGAIN)) continue;
    if (count <= 0) return count == 0 && !writing ? 0 : -1;
    bytes += count; size -= (size_t)count;
  }
  return 1;
}

static int fixture_sdk_send(const char* message) {
  const size_t size = strlen(message);
  if (!size || size > 4096) return -1;
  unsigned char header[4] = {(unsigned char)(size >> 24), (unsigned char)(size >> 16),
      (unsigned char)(size >> 8), (unsigned char)size};
  const int64_t deadline = cx_monotonic_ms() + 5000;
  if (fixture_sdk_io(header, sizeof(header), 1, deadline) != 1) return -1;
  return fixture_sdk_io((void*)message, size, 1, deadline);
}

static int fixture_sdk_receive(char response[8193], int64_t deadline) {
  unsigned char header[4];
  const int status = fixture_sdk_io(header, sizeof(header), 0, deadline);
  if (status != 1) return status;
  const uint32_t size = ((uint32_t)header[0] << 24) | ((uint32_t)header[1] << 16) |
      ((uint32_t)header[2] << 8) | (uint32_t)header[3];
  if (!size || size > 8192) return -1;
  if (fixture_sdk_io(response, size, 0, deadline) != 1 || memchr(response, 0, size)) return -1;
  response[size] = 0;
  return 1;
}

static int fixture_sdk_request(const char* id, const char* method, const char* params) {
  struct timespec now;
  if (clock_gettime(CLOCK_REALTIME, &now) || now.tv_sec < 0) return -1;
  const uint64_t deadline = (uint64_t)now.tv_sec * 1000 + (uint64_t)now.tv_nsec / 1000000 + 5000;
  if (deadline > 9007199254740991ULL) return -1;
  char message[4096];
  const int size = snprintf(message, sizeof(message),
      "{\"kind\":\"request\",\"version\":1,\"generation\":\"1\",\"id\":\"%s\",\"method\":\"%s\",\"params\":%s,\"deadline_ms\":%llu}",
      id, method, params, (unsigned long long)deadline);
  if (size < 0 || (size_t)size >= sizeof(message)) return -1;
  return fixture_sdk_send(message);
}

static int fixture_sdk_event(const char* name) {
  char message[256];
  const int size = snprintf(message, sizeof(message),
      "{\"kind\":\"event\",\"version\":1,\"generation\":\"1\",\"name\":\"%s\",\"data\":{}}", name);
  if (size < 0 || (size_t)size >= sizeof(message)) return -1;
  return fixture_sdk_send(message);
}

static int fixture_sdk_tool(const struct cx_launch_record* record) {
  // Fixed trusted-serializer fixture, deliberately not a general JSON parser.
  const int64_t deadline = cx_monotonic_ms() + 30000;
  for (unsigned int calls = 0; calls < 32; ++calls) {
    char request[8193];
    const int received = fixture_sdk_receive(request, deadline);
    if (received == 0) return 0;
    if (received != 1 || strncmp(request, "{\"kind\":\"request\",", 18) ||
        !strstr(request, "\"method\":\"tools.invoke\"") || !strstr(request, "\"name\":\"echo\"")) return 109;
    const char* id = strstr(request, "\"id\":\"");
    if (!id) return 110;
    id += 6;
    const char* end = strchr(id, '"');
    if (!end || end == id || end - id > 128) return 111;
    for (const char* p = id; p != end; ++p)
      if (!((*p >= 'a' && *p <= 'z') || (*p >= 'A' && *p <= 'Z') ||
            (*p >= '0' && *p <= '9') || *p == '-' || *p == '_')) return 112;
    char marker[1200];
    const int size = snprintf(marker, sizeof(marker), "%s/tool-effects", record->roots[CX_DATA]);
    if (size < 0 || (size_t)size >= sizeof(marker)) return 113;
    const int file = open(marker, O_WRONLY | O_CREAT | O_APPEND, 0600);
    if (file < 0) return 114;
    const int failed = send_all(file, "1\n", 2) || fsync(file);
    if (close(file) || failed) return 115;
    char response[512];
    const int count = snprintf(response, sizeof(response),
        "{\"kind\":\"response\",\"version\":1,\"generation\":\"1\",\"id\":\"%.*s\",\"result\":{\"ok\":true}}",
        (int)(end-id), id);
    if (count < 0 || (size_t)count >= sizeof(response) || fixture_sdk_send(response) != 1) return 116;
  }
  return 117;
}

#include "fixture_health.h"
#include "fixture_http.h"

static int fixture_sdk_run(const struct cx_launch_record* record, const char* mode) {
  const int flags = fcntl(3, F_GETFL);
  if (flags < 0 || fcntl(3, F_SETFL, flags | O_NONBLOCK)) return 94;
  char response[8193];
  if (fixture_sdk_request("before-ready", "state.get", "{\"key\":\"fixture\"}") != 1 ||
      fixture_sdk_receive(response, cx_monotonic_ms() + 5000) != 1) return 95;
  static const char rejected[] = "{\"kind\":\"response\",\"version\":1,\"generation\":\"1\",\"id\":\"before-ready\",\"error\":{\"code\":\"APP_NOT_READY\",";
  if (strncmp(response, rejected, sizeof(rejected) - 1)) return 96;
  if (fixture_sdk_event("worker.fixture.before_ready_rejected") != 1) return 97;
  if (!strcmp(mode, "sdk_no_report")) {
    /* Parent owns the short activation timeout. EOF/kill ends this fixed shim. */
    return fixture_sdk_receive(response, cx_monotonic_ms() + 30000) == 0 ? 0 : 98;
  }
  const char* report = !strcmp(mode, "sdk_wrong_handlers") ?
      "{\"tools\":[\"undeclared\"],\"events\":[],\"lifecycle\":[]}" :
      fixture_health_mode(mode) ? "{\"tools\":[],\"events\":[],\"lifecycle\":[\"health_check\",\"startup\"]}" :
      !strcmp(mode, "sdk_tool") ? "{\"tools\":[\"echo\"],\"events\":[],\"lifecycle\":[]}" :
      "{\"tools\":[],\"events\":[],\"lifecycle\":[]}";
  if (fixture_sdk_request("ready", "worker.ready", report) != 1) return 99;
  if (fixture_health_mode(mode)) {
    if (fixture_health_before(mode, response) != 1) return 121;
    if (!strcmp(mode, "sdk_bad_health")) return fixture_sdk_receive(response, cx_monotonic_ms()+5000)==0 ? 0 : 122;
  }
  char startup[8193] = {0};
  int received = fixture_sdk_receive(response, cx_monotonic_ms() + 5000);
  if (received==1 && fixture_health_mode(mode) && strstr(response,"\"method\":\"lifecycle.dispatch\"")) {
    char id[129];
    if (fixture_health_parse(response,"startup",id)!=1) return 124;
    // Activation's ready ACK and outgoing startup request share one actor but
    // can become writable in either order. Preserve the already validated call.
    memcpy(startup,response,strlen(response)+1);
    received=fixture_sdk_receive(response,cx_monotonic_ms()+5000);
  }
  if (received == 0) return 0;
  if (received != 1) return 100;
  static const char accepted[] = "{\"kind\":\"response\",\"version\":1,\"generation\":\"1\",\"id\":\"ready\",\"result\":null}";
  if (strcmp(response, accepted)) return 101;
  char marker[1200];
  const int size = snprintf(marker, sizeof(marker), "%s/ready-ack", record->roots[CX_DATA]);
  if (size < 0 || (size_t)size >= sizeof(marker)) return 102;
  const int file = open(marker, O_WRONLY | O_CREAT | O_EXCL, 0600);
  if (file < 0) return 103;
  if (fsync(file) || close(file)) return 104;
  if (fixture_sdk_event("worker.fixture.ready_ack") != 1) return 105;
  if (fixture_health_mode(mode)) return fixture_health_after(response,startup)==1 ? 0 : 123;
  if (!strcmp(mode, "sdk_http") || !strcmp(mode, "sdk_http_paused")) { int result=fixture_http_run(response,!strcmp(mode, "sdk_http_paused")); if (result) return result; }
  if (!strcmp(mode, "sdk_files")) {
    /* Exercise both namespaces on the exact inherited production peer. */
    if (fixture_sdk_request("files-state", "state.get", "{\"key\":\"fixture\"}") != 1 ||
        fixture_sdk_receive(response, cx_monotonic_ms() + 5000) != 1 ||
        strcmp(response, "{\"kind\":\"response\",\"version\":1,\"generation\":\"1\",\"id\":\"files-state\",\"result\":null}")) return 118;
    if (fixture_sdk_request("files-write", "files.atomic_replace",
        "{\"path\":\"fixture-file\",\"contentsBase64\":\"AP9maWxl\"}") != 1 ||
        fixture_sdk_receive(response, cx_monotonic_ms() + 5000) != 1 ||
        strcmp(response, "{\"kind\":\"response\",\"version\":1,\"generation\":\"1\",\"id\":\"files-write\",\"result\":{\"bytesWritten\":6}}")) return 119;
    if (fixture_sdk_event("worker.fixture.files_complete") != 1) return 120;
  }
  if (!strcmp(mode, "sdk_tool")) return fixture_sdk_tool(record);
  if (!strcmp(mode, "sdk_broker_call")) {
    if (fixture_sdk_request("after-ready", "state.get", "{\"key\":\"fixture\"}") != 1) return 106;
    const int reply = fixture_sdk_receive(response, cx_monotonic_ms() + 5000);
    if (reply == 0) return 0;
    static const char prefix[] = "{\"kind\":\"response\",\"version\":1,\"generation\":\"1\",\"id\":\"after-ready\",";
    if (reply != 1 || strncmp(response, prefix, sizeof(prefix) - 1)) return 107;
  }
  // Valid fixture stays alive until its one inherited channel closes. No
  // additional transport, subprocess or runtime loop is introduced.
  return fixture_sdk_receive(response, cx_monotonic_ms() + 30000) == 0 ? 0 : 108;
}
