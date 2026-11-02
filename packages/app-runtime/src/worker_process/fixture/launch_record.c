#define _POSIX_C_SOURCE 200809L
#include "launcher.h"

#include <errno.h>
#include <limits.h>
#include <poll.h>
#include <stdbool.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

int64_t cx_monotonic_ms(void) {
  struct timespec now;
  if (clock_gettime(CLOCK_MONOTONIC, &now) != 0) return -1;
  return (int64_t)now.tv_sec * 1000 + now.tv_nsec / 1000000;
}

static int transfer(void* bytes, size_t length, bool writing, int64_t deadline) {
  unsigned char* cursor = bytes;
  while (length) {
    int64_t now = cx_monotonic_ms();
    if (now < 0 || now >= deadline) return CX_LAUNCH_TIMEOUT;
    int64_t remaining = deadline - now;
    struct pollfd fd = {CX_APP_CONTROL_FD, writing ? POLLOUT : POLLIN, 0};
    int result = poll(&fd, 1, remaining > INT_MAX ? INT_MAX : (int)remaining);
    if (result < 0 && errno == EINTR) continue;
    if (result == 0) return CX_LAUNCH_TIMEOUT;
    if (result < 0 || !(fd.revents & fd.events)) return CX_LAUNCH_HANDSHAKE;
    ssize_t count = writing ? write(fd.fd, cursor, length) : read(fd.fd, cursor, length);
    if (count < 0 && (errno == EINTR || errno == EAGAIN)) continue;
    if (count <= 0) return CX_LAUNCH_HANDSHAKE;
    cursor += count;
    length -= (size_t)count;
  }
  return 0;
}

static uint32_t be32(const unsigned char* bytes) {
  return ((uint32_t)bytes[0] << 24) | ((uint32_t)bytes[1] << 16) |
         ((uint32_t)bytes[2] << 8) | bytes[3];
}

struct reader { const unsigned char* cursor; size_t remaining; };

static bool valid_utf8(const unsigned char* bytes, size_t length) {
  for (size_t i = 0; i < length;) {
    uint32_t value = bytes[i++];
    if (value < 0x80) continue;
    unsigned int following;
    uint32_t minimum;
    if (value >= 0xc2 && value <= 0xdf) { following = 1; minimum = 0x80; value &= 0x1f; }
    else if (value >= 0xe0 && value <= 0xef) { following = 2; minimum = 0x800; value &= 0x0f; }
    else if (value >= 0xf0 && value <= 0xf4) { following = 3; minimum = 0x10000; value &= 0x07; }
    else return false;
    if (following > length - i) return false;
    for (unsigned int j = 0; j < following; ++j) {
      unsigned char next = bytes[i++];
      if ((next & 0xc0) != 0x80) return false;
      value = (value << 6) | (next & 0x3f);
    }
    if (value < minimum || value > 0x10ffff || (value >= 0xd800 && value <= 0xdfff)) return false;
  }
  return true;
}

static bool take_string(struct reader* reader, char* destination, size_t limit) {
  if (reader->remaining < 4) return false;
  size_t length = be32(reader->cursor);
  if (!length || length > limit || length > reader->remaining - 4) return false;
  reader->cursor += 4;
  reader->remaining -= 4;
  if (!valid_utf8(reader->cursor, length)) return false;
  for (size_t i = 0; i < length; ++i) {
    if (reader->cursor[i] < 0x20 || reader->cursor[i] == 0x7f) return false;
  }
  memcpy(destination, reader->cursor, length);
  destination[length] = '\0';
  reader->cursor += length;
  reader->remaining -= length;
  return true;
}

static bool valid_identity(const struct cx_launch_record* record) {
  size_t length = strlen(record->generation);
  if (!length || length > 19 || (length > 1 && record->generation[0] == '0')) return false;
  uint64_t generation = 0;
  for (size_t i = 0; i < length; ++i) {
    char c = record->generation[i];
    if (c < '0' || c > '9') return false;
    generation = generation * 10 + (uint64_t)(c - '0');
  }
  if (!generation || generation > INT64_MAX) return false;
  for (const char* c = record->installation; *c; ++c) {
    if (!((*c >= 'a' && *c <= 'z') || (*c >= 'A' && *c <= 'Z') ||
          (*c >= '0' && *c <= '9') || *c == '-' || *c == '_' || *c == '.')) return false;
  }
  if (strlen(record->digest) != 64) return false;
  for (const char* c = record->digest; *c; ++c) {
    if (!((*c >= 'a' && *c <= 'f') || (*c >= '0' && *c <= '9'))) return false;
  }
  return true;
}

int cx_read_record(struct cx_launch_record* record, int64_t deadline) {
  unsigned char header[12];
  int status = transfer(header, sizeof(header), false, deadline);
  if (status) return status;
  size_t length = be32(header + 8);
  if (memcmp(header, "CXAWL001", 8) || length < 24 || length > CX_APP_RECORD_MAX)
    return CX_LAUNCH_INVALID;
  unsigned char* bytes = malloc(length);
  if (!bytes) return CX_LAUNCH_INVALID;
  status = transfer(bytes, length, false, deadline);
  if (status) { free(bytes); return status; }
  record->nofile = be32(bytes);
  record->cpu_seconds = be32(bytes + 4);
  record->heap_mib = be32(bytes + 8);
  record->v8_threads = be32(bytes + 12);
  record->max_file_bytes = ((uint64_t)be32(bytes + 16) << 32) | be32(bytes + 20);
  struct reader reader = {bytes + 24, length - 24};
  bool valid = record->nofile >= 32 && record->nofile <= 4096 &&
      record->cpu_seconds >= 1 && record->cpu_seconds <= 86400 &&
      record->heap_mib >= 16 && record->heap_mib <= 65536 &&
      record->v8_threads >= 1 && record->v8_threads <= 4 &&
      record->max_file_bytes >= 1 && record->max_file_bytes <= (UINT64_C(1) << 40) &&
      take_string(&reader, record->generation, 128) &&
      take_string(&reader, record->installation, 128) &&
      take_string(&reader, record->digest, 64) && valid_identity(record);
  for (size_t i = 0; valid && i < CX_ROOT_COUNT; ++i)
    valid = take_string(&reader, record->roots[i], CX_APP_PATH_MAX);
  // Bootstrap is source code: unlike metadata, newline/tab are valid. Only an
  // exact, bounded UTF-8 artifact without embedded NUL may reach the embedder.
  if (valid && reader.remaining >= 4) {
    record->bootstrap_length = be32(reader.cursor);
    valid = record->bootstrap_length > 0 &&
        record->bootstrap_length <= CX_APP_BOOTSTRAP_MAX &&
        record->bootstrap_length == reader.remaining - 4 &&
        valid_utf8(reader.cursor + 4, record->bootstrap_length) &&
        !memchr(reader.cursor + 4, 0, record->bootstrap_length);
    if (valid) {
      record->bootstrap = malloc(record->bootstrap_length + 1);
      valid = record->bootstrap != NULL;
      if (valid) {
        memcpy(record->bootstrap, reader.cursor + 4, record->bootstrap_length);
        record->bootstrap[record->bootstrap_length] = '\0';
      }
    }
  } else valid = false;
  free(bytes);
  return valid ? 0 : CX_LAUNCH_INVALID;
}

static size_t append_string(unsigned char* bytes, const char* value) {
  size_t length = strlen(value);
  bytes[0] = (unsigned char)(length >> 24);
  bytes[1] = (unsigned char)(length >> 16);
  bytes[2] = (unsigned char)(length >> 8);
  bytes[3] = (unsigned char)length;
  memcpy(bytes + 4, value, length);
  return length + 4;
}

int cx_ready_and_wait(const struct cx_launch_record* record, int64_t deadline) {
  unsigned char ready[8 + 3 * 4 + 128 + 128 + 64];
  memcpy(ready, "CXAWR001", 8);
  size_t size = 8;
  size += append_string(ready + size, record->generation);
  size += append_string(ready + size, record->installation);
  size += append_string(ready + size, record->digest);
  int status = transfer(ready, size, true, deadline);
  if (status) return status;
  char reply[8];
  status = transfer(reply, sizeof(reply), false, deadline);
  if (status) return status;
  return memcmp(reply, "CXAWGO01", 8) ? CX_LAUNCH_HANDSHAKE : 0;
}
