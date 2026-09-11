/* The whole C side of sabiruby-compiler: one call from source to RITE binary.
   It does what the reference mrbc (mrbgems/mruby-bin-mrbc/tools/mrbc/mrbc.c)
   does, reading from memory instead of files, so that Rust never touches the
   compiler's structs (mrc_ccontext has bit fields). */

#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "mrc_irep.h"
#include "mrc_ccontext.h"
#include "mrc_dump.h"
#include "mrc_compile.h"
#include "mrc_diagnostic.h"

/* Flags (keep in sync with src/ffi.rs). */
#define SHIM_DEBUG_INFO  1u /* mrbc -g */
#define SHIM_REMOVE_LV   2u /* mrbc --remove-lv */
#define SHIM_NO_EXT_OPS  4u /* mrbc --no-ext-ops */
#define SHIM_NO_OPTIMIZE 8u /* mrbc --no-optimize */

/* Results. */
#define SHIM_OK            0
#define SHIM_COMPILE_ERROR 1 /* diagnostics in *diag */
#define SHIM_DUMP_ERROR    2
#define SHIM_NO_MEMORY     3

/* The standalone compiler still references these two mruby functions; the
   reference mrbc defines the same dummies (end of mrbc.c). */
mrc_sym
mrb_intern(mrb_state *mrb, const char *str, size_t len)
{
  (void)mrb; (void)str; (void)len;
  return 0;
}

const char*
mrb_sym_name(mrb_state *mrb, mrc_sym sym)
{
  (void)mrb; (void)sym;
  return NULL;
}

/* Growable text buffer for the diagnostics. */
typedef struct { char *p; size_t len, cap; int oom; } sbuf;

static void
sbuf_put(sbuf *b, const char *s, size_t n)
{
  if (b->oom) return;
  if (b->len + n + 1 > b->cap) {
    size_t cap = b->cap ? b->cap * 2 : 256;
    while (cap < b->len + n + 1) cap *= 2;
    char *q = (char *)realloc(b->p, cap);
    if (!q) { b->oom = 1; return; }
    b->p = q; b->cap = cap;
  }
  memcpy(b->p + b->len, s, n);
  b->len += n;
  b->p[b->len] = '\0';
}

static void
sbuf_str(sbuf *b, const char *s)
{
  sbuf_put(b, s ? s : "", s ? strlen(s) : 0);
}

static void
sbuf_u32(sbuf *b, uint32_t v)
{
  char tmp[12];
  int i = (int)sizeof(tmp);
  tmp[--i] = '\0';
  do { tmp[--i] = (char)('0' + v % 10); v /= 10; } while (v && i > 0);
  sbuf_str(b, tmp + i);
}

/* Diagnostics as records separated by 0x1e, fields by 0x1f:
   code, line, column, filename, message (the message may contain tabs and newlines). */
static char *
collect_diagnostics(mrc_ccontext *c)
{
  sbuf b = { NULL, 0, 0, 0 };
  for (mrc_diagnostic_list *d = c->diagnostic_list; d; d = d->next) {
    const char *filename = d->filename ? d->filename
                         : (c->filename_table ? c->filename_table[0].filename : "-");
    sbuf_u32(&b, (uint32_t)d->code); sbuf_put(&b, "\x1f", 1);
    sbuf_u32(&b, d->line);           sbuf_put(&b, "\x1f", 1);
    sbuf_u32(&b, d->column);         sbuf_put(&b, "\x1f", 1);
    sbuf_str(&b, filename);          sbuf_put(&b, "\x1f", 1);
    sbuf_str(&b, d->message);        sbuf_put(&b, "\x1e", 1);
  }
  if (b.oom) { free(b.p); return NULL; }
  return b.p;
}

int
sabiruby_mrc_compile(const uint8_t *src, size_t len, const char *filename, unsigned flags,
                     uint8_t **out, size_t *out_len, char **diag)
{
  *out = NULL; *out_len = 0; *diag = NULL;
  mrc_ccontext *c = mrc_ccontext_new(NULL);
  if (!c) return SHIM_NO_MEMORY;
  if (filename && !mrc_ccontext_filename(c, filename)) { mrc_ccontext_free(c); return SHIM_NO_MEMORY; }
  c->no_exec = 1; /* as mrbc's load_file */
  c->no_ext_ops = (flags & SHIM_NO_EXT_OPS) ? 1 : 0;
  c->no_optimize = (flags & SHIM_NO_OPTIMIZE) ? 1 : 0;

  /* NUL-terminated copy, like the buffer mrbc reads a file into; the parser
     keeps pointing into it until the irep is freed */
  uint8_t *buf = (uint8_t *)malloc(len + 1);
  if (!buf) { mrc_ccontext_free(c); return SHIM_NO_MEMORY; }
  if (len) memcpy(buf, src, len);
  buf[len] = '\0';
  const uint8_t *source = buf;

  int result;
  mrc_irep *irep = mrc_load_string_cxt(c, &source, len);
  *diag = collect_diagnostics(c);
  if (!irep) {
    result = SHIM_COMPILE_ERROR;
  }
  else {
    if (flags & SHIM_REMOVE_LV) mrc_irep_remove_lv(c, irep);
    uint8_t *bin = NULL;
    size_t size = 0;
    int n = mrc_dump_irep(c, irep, (flags & SHIM_DEBUG_INFO) ? MRC_DUMP_DEBUG_INFO : 0, &bin, &size);
    if (n == MRC_DUMP_OK) {
      *out = bin; *out_len = size;
      result = SHIM_OK;
    }
    else {
      free(bin);
      result = SHIM_DUMP_ERROR;
    }
    mrc_irep_free(c, irep);
  }
  mrc_ccontext_free(c);
  free(buf);
  return result;
}

void
sabiruby_mrc_free(void *p)
{
  free(p);
}

const char *
sabiruby_mrc_version(void)
{
  return "mruby 4.1.0-rc (3cf73ee), Prism " PRISM_VERSION;
}
