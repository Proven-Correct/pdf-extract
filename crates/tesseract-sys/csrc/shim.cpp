// shim.cpp: the only C/C++ entry points Rust calls (spec 0002 §3.4, D-5).
//
// Tesseract's own C API (capi.h) has no try/catch, so a std::bad_alloc thrown
// inside Recognize would unwind into Rust, which is undefined behaviour. Every
// function here is extern "C", takes and returns only C types, and runs its
// body inside guarded(), which turns any C++ exception into PXT_ERR_EXCEPTION
// plus the exception's what() in the caller's error buffer. Nothing is
// allocated across the boundary: text Tesseract allocates is handed to the
// Rust callback and freed here, inside pxt_collect.
//
// The handle is single-threaded (the Rust Instance is Send, not Sync);
// different handles are independent and may run concurrently.

#include <tesseract/baseapi.h>
#include <tesseract/ocrclass.h>
#include <tesseract/publictypes.h>
#include <tesseract/resultiterator.h>

// Tesseract's parameter registry. Not an installed header, so build.rs puts
// vendor/tesseract/src on the include path for both linkages; see
// pxt_is_global_param for why the shim reads it at all.
#include <ccutil/params.h>

#include <allheaders.h>

#include <algorithm>
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>

// Error codes; mirrored in src/ffi.rs.
#define PXT_OK 0
#define PXT_ERR_INIT 1
#define PXT_ERR_EXCEPTION 2
#define PXT_ERR_RECOGNIZE 3
#define PXT_ERR_TIMEOUT 4
#define PXT_ERR_ARG 5
#define PXT_ERR_CALLBACK 6

// pxt_collect flags: this word starts a block / a text line.
#define PXT_BLOCK_START 1
#define PXT_LINE_START 2

struct PxtHandle {
  tesseract::TessBaseAPI api;
  bool initialized = false;
  bool has_image = false;
};

typedef int (*pxt_word_cb)(void *user, int flags, const char *text, size_t len, int left,
                           int top, int right, int bottom, float confidence);

namespace {

// Copies msg into the caller's buffer and returns code. Takes a C string
// so the catch handlers below never allocate.
int fail(char *err, size_t err_len, int code, const char *msg) noexcept {
  if (err != nullptr && err_len > 0) std::snprintf(err, err_len, "%s", msg);
  return code;
}

// Runs body() and converts every C++ exception to an error code. noexcept,
// so nothing can unwind past this frame into Rust.
template <class F>
int guarded(char *err, size_t err_len, F &&body) noexcept {
  try {
    return body();
  } catch (const std::exception &e) {
    return fail(err, err_len, PXT_ERR_EXCEPTION, e.what());
  } catch (...) {
    return fail(err, err_len, PXT_ERR_EXCEPTION, "unknown C++ exception");
  }
}

void quiet_leptonica(const char *) {}

}  // namespace

extern "C" {

const char *pxt_version(void) {
  // A pointer to a static string; cannot throw.
  return tesseract::TessBaseAPI::Version();
}

// Process-wide, called once under a Rust Once before the first Init:
// Leptonica's messages go to a no-op handler and Tesseract's tprintf/tesserr
// output to the null device (debug_file is a process-global parameter, so
// setting it through a throwaway instance sets it for every instance).
int pxt_global_init(const char *debug_null_path, char *err, size_t err_len) {
  return guarded(err, err_len, [&]() -> int {
    if (debug_null_path == nullptr) {
      return fail(err, err_len, PXT_ERR_ARG, "debug_null_path is null");
    }
    leptSetStderrHandler(quiet_leptonica);
    tesseract::TessBaseAPI tmp;
    if (!tmp.SetVariable("debug_file", debug_null_path)) {
      return fail(err, err_len, PXT_ERR_INIT, "Tesseract has no debug_file parameter");
    }
    // Tesseract opens the debug_file lazily, on its first message, through
    // an unsynchronised global FILE*: two instances printing their first
    // message at once would both fopen it (a data race, and a leaked
    // handle). An Init that must fail (the model path runs through the null
    // device, which is not a directory) prints "Error opening data file" here,
    // under the Rust Once, so the handle is open before any instance exists.
    // Its later readers only read the pointer; tesserr's stream buffer still
    // caches its own copy lazily, but that copy is the same value.
    (void)tmp.Init(debug_null_path, "eng", tesseract::OEM_LSTM_ONLY);
    return PXT_OK;
  });
}

// Whether `name` is one of Tesseract's process-global parameters (one
// registered in GlobalParams() rather than in an instance's own vectors):
// 1 if it is, 0 if not, an error code otherwise. Init applies variables
// with SetParam, which looks in the global vectors first, so a global name
// passed to one instance would change every instance in the process
// (debug_file, for one, would undo pxt_global_init's silence). Asking
// Tesseract's own registry keeps the refusal in step with whatever version
// is linked, rather than with a list of names that drifts.
//
// The global vectors are filled by static initialisers before main and
// never change afterwards (a parameter leaves them only in its destructor,
// at exit), so reading them needs no lock.
int pxt_is_global_param(const char *name, char *err, size_t err_len) {
  return guarded(err, err_len, [&]() -> int {
    if (name == nullptr) {
      return fail(err, err_len, PXT_ERR_ARG, "pxt_is_global_param: name is null");
    }
    const tesseract::ParamsVectors *global = tesseract::GlobalParams();
    const auto in = [name](const auto &params) {
      return std::any_of(params.begin(), params.end(), [name](const auto *p) {
        return std::strcmp(p->name_str(), name) == 0;
      });
    };
    const bool found = in(global->int_params) || in(global->bool_params) ||
                       in(global->string_params) || in(global->double_params);
    return found ? 1 : 0;
  });
}

PxtHandle *pxt_create(void) {
  try {
    return new PxtHandle();
  } catch (...) {
    return nullptr;
  }
}

int pxt_init(PxtHandle *h, const char *datapath, const char *langs, int oem,
             const char *const *var_names, const char *const *var_values, size_t n_vars,
             char *err, size_t err_len) {
  return guarded(err, err_len, [&]() -> int {
    if (h == nullptr || datapath == nullptr || langs == nullptr || h->initialized ||
        (n_vars > 0 && (var_names == nullptr || var_values == nullptr))) {
      return fail(err, err_len, PXT_ERR_ARG, "pxt_init: invalid arguments or already initialised");
    }
    if (oem < 0 || oem >= tesseract::OEM_COUNT) {
      return fail(err, err_len, PXT_ERR_ARG, "pxt_init: engine mode out of range");
    }
    std::vector<std::string> names, values;
    for (size_t i = 0; i < n_vars; ++i) {
      names.emplace_back(var_names[i]);
      values.emplace_back(var_values[i]);
    }
    // set_only_non_debug_params = false: a debug variable passed here is
    // honoured rather than silently dropped.
    int rc = h->api.Init(datapath, langs, static_cast<tesseract::OcrEngineMode>(oem), nullptr,
                         0, &names, &values, false);
    if (rc != 0) {
      const std::string msg = std::string("Tesseract could not load language '") + langs +
                              "' from tessdata directory '" + datapath + "'";
      return fail(err, err_len, PXT_ERR_INIT, msg.c_str());
    }
    // Init succeeds when *any* language loaded: a model that fails to load
    // is reported only through tprintf (silenced) and dropped, and a failed
    // first language promotes the next one to primary. So every requested
    // code must appear among the loaded ones. Requested is a subset, not
    // equal: a model's tessedit_load_sublangs may add languages.
    std::vector<std::string> loaded;
    h->api.GetLoadedLanguagesAsVector(&loaded);
    const std::string requested(langs);
    for (size_t start = 0; start <= requested.size();) {
      size_t end = requested.find('+', start);
      if (end == std::string::npos) end = requested.size();
      const std::string code = requested.substr(start, end - start);
      start = end + 1;
      if (code.empty() || std::find(loaded.begin(), loaded.end(), code) != loaded.end()) {
        continue;
      }
      h->api.End();
      const std::string msg = "language \"" + code + "\" did not load";
      return fail(err, err_len, PXT_ERR_INIT, msg.c_str());
    }
    h->initialized = true;
    return PXT_OK;
  });
}

// 8-bit gray only. The pixels are copied into a Leptonica Pix that we create
// and check ourselves: TessBaseAPI::SetImage(const unsigned char*, ...) does
// not check pixCreate's result and would write through a null pointer for an
// image Leptonica refuses (>= 2^31 bytes) or cannot allocate.
int pxt_set_image(PxtHandle *h, const unsigned char *data, int width, int height,
                  int bytes_per_pixel, int bytes_per_line, char *err, size_t err_len) {
  return guarded(err, err_len, [&]() -> int {
    if (h == nullptr || !h->initialized || data == nullptr || bytes_per_pixel != 1 ||
        width <= 0 || height <= 0 || bytes_per_line < width) {
      return fail(err, err_len, PXT_ERR_ARG, "pxt_set_image: invalid arguments or not initialised");
    }
    // Leptonica's own cap (pixCreateHeader, 1.80+), checked here too so an
    // older system Leptonica cannot accept a multi-gigabyte page.
    const uint64_t row_bytes = 4 * ((static_cast<uint64_t>(width) + 3) / 4);
    if (row_bytes * static_cast<uint64_t>(height) >= (uint64_t{1} << 31)) {
      return fail(err, err_len, PXT_ERR_ARG, "pxt_set_image: image over Leptonica's 2^31-byte limit");
    }
    PIX *pix = pixCreate(width, height, 8);
    if (pix == nullptr) {
      return fail(err, err_len, PXT_ERR_ARG, "pxt_set_image: Leptonica refused the image size");
    }
    l_uint32 *line = pixGetData(pix);
    const int wpl = pixGetWpl(pix);
    for (int y = 0; y < height; ++y, line += wpl) {
      const unsigned char *row = data + static_cast<size_t>(y) * bytes_per_line;
      for (int x = 0; x < width; ++x) SET_DATA_BYTE(line, x, row[x]);
    }
    try {
      h->api.SetImage(pix);  // copies (or clones) the Pix
    } catch (...) {
      pixDestroy(&pix);
      throw;
    }
    pixDestroy(&pix);
    h->has_image = true;
    return PXT_OK;
  });
}

int pxt_set_resolution(PxtHandle *h, int ppi, char *err, size_t err_len) {
  return guarded(err, err_len, [&]() -> int {
    if (h == nullptr || !h->has_image) {
      return fail(err, err_len, PXT_ERR_ARG, "pxt_set_resolution: no image set");
    }
    h->api.SetSourceResolution(ppi);
    return PXT_OK;
  });
}

int pxt_set_psm(PxtHandle *h, int psm, char *err, size_t err_len) {
  return guarded(err, err_len, [&]() -> int {
    if (h == nullptr || psm < 0 || psm >= tesseract::PSM_COUNT) {
      return fail(err, err_len, PXT_ERR_ARG, "pxt_set_psm: invalid arguments");
    }
    h->api.SetPageSegMode(static_cast<tesseract::PageSegMode>(psm));
    return PXT_OK;
  });
}

// Recognize under a monitor whose deadline is deadline_ms from now (> 0).
// Tesseract returns -1 for a deadline, a cancel, a missing Init and a layout
// failure alike; it is a timeout only if our own clock is past the deadline
// too (D-11). Our deadline is taken before Tesseract's, so whenever Tesseract
// saw its deadline pass, so do we.
int pxt_recognize(PxtHandle *h, int deadline_ms, char *err, size_t err_len) {
  return guarded(err, err_len, [&]() -> int {
    if (h == nullptr || !h->has_image || deadline_ms <= 0) {
      return fail(err, err_len, PXT_ERR_ARG, "pxt_recognize: no image set or deadline not positive");
    }
    const auto deadline =
        std::chrono::steady_clock::now() + std::chrono::milliseconds(deadline_ms);
    tesseract::ETEXT_DESC monitor;
    monitor.set_deadline_msecs(deadline_ms);
    if (h->api.Recognize(&monitor) == 0) return PXT_OK;
    if (std::chrono::steady_clock::now() >= deadline) {
      const std::string msg =
          "recognition exceeded its deadline of " + std::to_string(deadline_ms) + " ms";
      return fail(err, err_len, PXT_ERR_TIMEOUT, msg.c_str());
    }
    return fail(err, err_len, PXT_ERR_RECOGNIZE, "Tesseract's Recognize failed");
  });
}

// Walks the result in reading order at word level and calls cb once per word
// with a non-null text. Block/line starts are carried in flags; a start seen
// on a word without text is carried to the next emitted word, so the Rust
// side can rebuild block -> line -> word without holding C pointers. A
// non-zero return from cb stops the walk (PXT_ERR_CALLBACK).
int pxt_collect(PxtHandle *h, pxt_word_cb cb, void *user, char *err, size_t err_len) {
  return guarded(err, err_len, [&]() -> int {
    if (h == nullptr || cb == nullptr) {
      return fail(err, err_len, PXT_ERR_ARG, "pxt_collect: invalid arguments");
    }
    std::unique_ptr<tesseract::ResultIterator> it(h->api.GetIterator());
    if (!it || it->Empty(tesseract::RIL_BLOCK)) return PXT_OK;
    int pending = 0;
    do {
      if (it->IsAtBeginningOf(tesseract::RIL_BLOCK)) pending |= PXT_BLOCK_START;
      if (it->IsAtBeginningOf(tesseract::RIL_TEXTLINE)) pending |= PXT_LINE_START;
      std::unique_ptr<char[]> text(it->GetUTF8Text(tesseract::RIL_WORD));
      if (!text) continue;  // to the loop condition, i.e. the next word
      int l = 0, t = 0, r = 0, b = 0;
      if (!it->BoundingBox(tesseract::RIL_WORD, &l, &t, &r, &b)) l = t = r = b = 0;
      const float conf = it->Confidence(tesseract::RIL_WORD);
      if (cb(user, pending, text.get(), std::strlen(text.get()), l, t, r, b, conf) != 0) {
        return fail(err, err_len, PXT_ERR_CALLBACK, "pxt_collect: the callback stopped the walk");
      }
      pending = 0;
    } while (it->Next(tesseract::RIL_WORD));
    return PXT_OK;
  });
}

// Drops the image and results; the models stay loaded for the next page.
void pxt_clear(PxtHandle *h) {
  if (h == nullptr) return;
  try { h->api.Clear(); } catch (...) {}
  h->has_image = false;
}

// Frees the models; the handle can be Init'ed again.
void pxt_end(PxtHandle *h) {
  if (h == nullptr) return;
  try { h->api.End(); } catch (...) {}
  h->initialized = false;
  h->has_image = false;
}

void pxt_delete(PxtHandle *h) {
  try { delete h; } catch (...) {}
}

#ifdef PXT_TEST_HOOKS
// Test-only (debug builds): proves the exception path end to end.
int pxt_test_throw(char *err, size_t err_len) {
  return guarded(err, err_len, []() -> int {
    throw std::runtime_error("pxt_test_throw: deliberate exception");
  });
}
#endif

}  // extern "C"
