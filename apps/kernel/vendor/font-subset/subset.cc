// The Browser mirror's font subsetter: HarfBuzz hb-subset plus the WOFF2 codec in one wasm module.
// `font_subset` keeps the glyphs of the given code points and returns the font as WOFF2.
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>

#include <hb.h>
#include <hb-subset.h>
#include <woff2/decode.h>
#include <woff2/encode.h>

extern "C" {

uint8_t* font_alloc(size_t length) { return static_cast<uint8_t*>(malloc(length ? length : 1)); }
void font_free(void* pointer) { free(pointer); }

// Returns a font_alloc'd WOFF2 file (the caller font_free's it) or null; the input is a WOFF2, TrueType or OpenType file.
uint8_t* font_subset(const uint8_t* data, size_t length, const uint32_t* code_points, size_t count, int quality, size_t* out_length) {
  *out_length = 0;
  std::vector<uint8_t> sfnt;
  if (length >= 4 && memcmp(data, "wOF2", 4) == 0) {
    size_t size = woff2::ComputeWOFF2FinalSize(data, length);
    if (size == 0 || size > (64u << 20)) return nullptr;
    sfnt.resize(size);
    if (!woff2::ConvertWOFF2ToTTF(sfnt.data(), size, data, length)) return nullptr;
  } else {
    sfnt.assign(data, data + length);
  }

  hb_blob_t* blob = hb_blob_create(reinterpret_cast<const char*>(sfnt.data()), static_cast<unsigned>(sfnt.size()), HB_MEMORY_MODE_READONLY, nullptr, nullptr);
  hb_face_t* face = hb_face_create(blob, 0);
  hb_blob_destroy(blob);
  hb_subset_input_t* input = hb_subset_input_create_or_fail();
  if (!input) { hb_face_destroy(face); return nullptr; }
  hb_set_t* wanted = hb_subset_input_unicode_set(input);
  for (size_t at = 0; at < count; at++) hb_set_add(wanted, code_points[at]);
  // An icon font names its glyphs by private-use code points that only a stylesheet's `content` mentions: a face most of
  // whose characters are private-use keeps them all. A text face with a few private-use glyphs keeps those the page uses.
  hb_set_t* mapped = hb_set_create();
  hb_face_collect_unicodes(face, mapped);
  unsigned total = hb_set_get_population(mapped), private_use = 0;
  const hb_codepoint_t areas[3][2] = {{0xE000u, 0xF8FFu}, {0xF0000u, 0xFFFFDu}, {0x100000u, 0x10FFFDu}};
  for (const auto& area : areas)
    for (hb_codepoint_t at = area[0] - 1; hb_set_next(mapped, &at) && at <= area[1];) private_use++;
  if (private_use * 2 >= total)
    for (const auto& area : areas)
      for (hb_codepoint_t at = area[0] - 1; hb_set_next(mapped, &at) && at <= area[1];) hb_set_add(wanted, at);
  hb_set_destroy(mapped);
  hb_face_t* subset = hb_subset_or_fail(face, input);
  hb_subset_input_destroy(input);
  hb_face_destroy(face);
  if (!subset) return nullptr;
  hb_blob_t* subset_blob = hb_face_reference_blob(subset);
  unsigned subset_length = 0;
  const char* subset_data = hb_blob_get_data(subset_blob, &subset_length);

  woff2::WOFF2Params params;
  params.brotli_quality = quality;
  size_t capacity = woff2::MaxWOFF2CompressedSize(reinterpret_cast<const uint8_t*>(subset_data), subset_length);
  uint8_t* out = font_alloc(capacity);
  size_t written = capacity;
  bool ok = out && woff2::ConvertTTFToWOFF2(reinterpret_cast<const uint8_t*>(subset_data), subset_length, out, &written, params);
  hb_blob_destroy(subset_blob);
  hb_face_destroy(subset);
  if (!ok) { font_free(out); return nullptr; }
  *out_length = written;
  return out;
}

}
