#pragma once

#include <stddef.h>  /* size_t */

#ifdef __cplusplus
extern "C" {
#endif

/* ── RGA ──────────────────────────────────────────────────────── */
int rk_rga_cvt_resize(
    void *src_va, int src_w, int src_h, int src_fmt,
    void *dst_va, int dst_w, int dst_h, int dst_fmt
);

/* ── MPP JPEG encoder ─────────────────────────────────────────── */
typedef struct RkMppJpeg RkMppJpeg;  /* 不透明指標，內部細節留在 .c */

RkMppJpeg *rk_mpp_jpeg_create(int width, int height, int quality, int format);
void        rk_mpp_jpeg_destroy(RkMppJpeg *enc);
int         rk_mpp_jpeg_encode(RkMppJpeg *enc,
                               const void *nv12_data, size_t nv12_size,
                               void *out_buf,          size_t out_cap);

#ifdef __cplusplus
}  /* extern "C" */
#endif