/*
 * hw_shim.c — 薄 C 層，封裝 RGA3 與 MPP (MJPEG 編碼器)。
 *
 * 由 CMake 以靜態庫方式編譯，透過 hw_shim STATIC 目標連結。
 */

#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <dlfcn.h>

/* RGA3 im2d API — 僅用 header 取型別，真正符號在 runtime 以 dlopen 載入。 */
#include <rga/im2d.h>

/* MPP — 只用宣告；真正符號在 runtime 以 dlopen 載入。 */
#include <rockchip/rk_mpi.h>
#include <rockchip/rk_venc_cfg.h>
#include <rockchip/mpp_frame.h>
#include <rockchip/mpp_packet.h>
#include <rockchip/mpp_buffer.h>

#define HW_SHIM_TAG "hw_shim"

static void *open_first_library(const char *const *candidates)
{
    for (size_t i = 0; candidates[i] != NULL; ++i)
    {
        void *handle = dlopen(candidates[i], RTLD_LAZY | RTLD_LOCAL);
        if (handle)
            return handle;
    }
    return NULL;
}

static void *load_symbol(void *handle, const char *name)
{
    return handle ? dlsym(handle, name) : NULL;
}

typedef rga_buffer_t (*rga_wrapbuffer_virtualaddr_t_fn)(void *, int, int, int, int, int);
typedef IM_STATUS (*rga_improcess_fn)(rga_buffer_t, rga_buffer_t, rga_buffer_t,
                                      im_rect, im_rect, im_rect, int);

typedef struct RgaDynApi
{
    int attempted;
    int ready;
    void *handle;
    rga_wrapbuffer_virtualaddr_t_fn wrapbuffer_virtualaddr_t_fn;
    rga_improcess_fn improcess_fn;
} RgaDynApi;

static RgaDynApi g_rga = {0};

static int ensure_rga_api(void)
{
    if (g_rga.ready)
        return 0;
    if (g_rga.attempted)
        return -1;

    g_rga.attempted = 1;

    const char *const candidates[] = {
        "librga.so",
        "librga.so.2",
        "librga.so.2.1.0",
        NULL,
    };

    void *handle = open_first_library(candidates);
    if (!handle)
        return -1;

    g_rga.wrapbuffer_virtualaddr_t_fn = (rga_wrapbuffer_virtualaddr_t_fn)load_symbol(handle, "wrapbuffer_virtualaddr_t");
    g_rga.improcess_fn = (rga_improcess_fn)load_symbol(handle, "improcess");
    if (!g_rga.wrapbuffer_virtualaddr_t_fn || !g_rga.improcess_fn)
    {
        dlclose(handle);
        memset(&g_rga, 0, sizeof(g_rga));
        g_rga.attempted = 1;
        return -1;
    }

    g_rga.handle = handle;
    g_rga.ready = 1;
    return 0;
}

typedef MPP_RET (*mpp_create_fn)(MppCtx *, MppApi **);
typedef MPP_RET (*mpp_init_fn)(MppCtx, MppCtxType, MppCodingType);
typedef MPP_RET (*mpp_destroy_fn)(MppCtx);
typedef MPP_RET (*mpp_enc_cfg_init_fn)(MppEncCfg *);
typedef MPP_RET (*mpp_enc_cfg_deinit_fn)(MppEncCfg);
typedef MPP_RET (*mpp_enc_cfg_set_s32_fn)(MppEncCfg, const char *, RK_S32);
typedef MPP_RET (*mpp_buffer_group_get_fn)(MppBufferGroup *, MppBufferType, MppBufferMode, const char *, const char *);
typedef MPP_RET (*mpp_buffer_group_put_fn)(MppBufferGroup);
typedef MPP_RET (*mpp_buffer_get_with_tag_fn)(MppBufferGroup, MppBuffer *, size_t, const char *, const char *);
typedef void *(*mpp_buffer_get_ptr_with_caller_fn)(MppBuffer, const char *);
typedef MPP_RET (*mpp_buffer_put_with_caller_fn)(MppBuffer, const char *);
typedef MPP_RET (*mpp_frame_init_fn)(MppFrame *);
typedef MPP_RET (*mpp_frame_deinit_fn)(MppFrame *);
typedef void (*mpp_frame_set_width_fn)(MppFrame, RK_U32);
typedef void (*mpp_frame_set_height_fn)(MppFrame, RK_U32);
typedef void (*mpp_frame_set_hor_stride_fn)(MppFrame, RK_U32);
typedef void (*mpp_frame_set_ver_stride_fn)(MppFrame, RK_U32);
typedef void (*mpp_frame_set_fmt_fn)(MppFrame, MppFrameFormat);
typedef void (*mpp_frame_set_buffer_fn)(MppFrame, MppBuffer);
typedef void (*mpp_frame_set_eos_fn)(MppFrame, RK_U32);
typedef size_t (*mpp_packet_get_length_fn)(const MppPacket);
typedef void *(*mpp_packet_get_data_fn)(const MppPacket);
typedef MPP_RET (*mpp_packet_deinit_fn)(MppPacket *);

typedef struct MppDynApi
{
    int attempted;
    int ready;
    void *handle;
    mpp_create_fn mpp_create;
    mpp_init_fn mpp_init;
    mpp_destroy_fn mpp_destroy;
    mpp_enc_cfg_init_fn mpp_enc_cfg_init;
    mpp_enc_cfg_deinit_fn mpp_enc_cfg_deinit;
    mpp_enc_cfg_set_s32_fn mpp_enc_cfg_set_s32;
    mpp_buffer_group_get_fn mpp_buffer_group_get;
    mpp_buffer_group_put_fn mpp_buffer_group_put;
    mpp_buffer_get_with_tag_fn mpp_buffer_get_with_tag;
    mpp_buffer_get_ptr_with_caller_fn mpp_buffer_get_ptr_with_caller;
    mpp_buffer_put_with_caller_fn mpp_buffer_put_with_caller;
    mpp_frame_init_fn mpp_frame_init;
    mpp_frame_deinit_fn mpp_frame_deinit;
    mpp_frame_set_width_fn mpp_frame_set_width;
    mpp_frame_set_height_fn mpp_frame_set_height;
    mpp_frame_set_hor_stride_fn mpp_frame_set_hor_stride;
    mpp_frame_set_ver_stride_fn mpp_frame_set_ver_stride;
    mpp_frame_set_fmt_fn mpp_frame_set_fmt;
    mpp_frame_set_buffer_fn mpp_frame_set_buffer;
    mpp_frame_set_eos_fn mpp_frame_set_eos;
    mpp_packet_get_length_fn mpp_packet_get_length;
    mpp_packet_get_data_fn mpp_packet_get_data;
    mpp_packet_deinit_fn mpp_packet_deinit;
} MppDynApi;

static MppDynApi g_mpp = {0};

static int ensure_mpp_api(void)
{
    if (g_mpp.ready)
        return 0;
    if (g_mpp.attempted)
        return -1;

    g_mpp.attempted = 1;

    const char *const candidates[] = {
        "librockchip_mpp.so",
        "librockchip_mpp.so.1",
        "librockchip_mpp.so.0",
        NULL,
    };

    void *handle = open_first_library(candidates);
    if (!handle)
        return -1;

    g_mpp.mpp_create = (mpp_create_fn)load_symbol(handle, "mpp_create");
    g_mpp.mpp_init = (mpp_init_fn)load_symbol(handle, "mpp_init");
    g_mpp.mpp_destroy = (mpp_destroy_fn)load_symbol(handle, "mpp_destroy");
    g_mpp.mpp_enc_cfg_init = (mpp_enc_cfg_init_fn)load_symbol(handle, "mpp_enc_cfg_init");
    g_mpp.mpp_enc_cfg_deinit = (mpp_enc_cfg_deinit_fn)load_symbol(handle, "mpp_enc_cfg_deinit");
    g_mpp.mpp_enc_cfg_set_s32 = (mpp_enc_cfg_set_s32_fn)load_symbol(handle, "mpp_enc_cfg_set_s32");
    g_mpp.mpp_buffer_group_get = (mpp_buffer_group_get_fn)load_symbol(handle, "mpp_buffer_group_get");
    g_mpp.mpp_buffer_group_put = (mpp_buffer_group_put_fn)load_symbol(handle, "mpp_buffer_group_put");
    g_mpp.mpp_buffer_get_with_tag = (mpp_buffer_get_with_tag_fn)load_symbol(handle, "mpp_buffer_get_with_tag");
    g_mpp.mpp_buffer_get_ptr_with_caller = (mpp_buffer_get_ptr_with_caller_fn)load_symbol(handle, "mpp_buffer_get_ptr_with_caller");
    g_mpp.mpp_buffer_put_with_caller = (mpp_buffer_put_with_caller_fn)load_symbol(handle, "mpp_buffer_put_with_caller");
    g_mpp.mpp_frame_init = (mpp_frame_init_fn)load_symbol(handle, "mpp_frame_init");
    g_mpp.mpp_frame_deinit = (mpp_frame_deinit_fn)load_symbol(handle, "mpp_frame_deinit");
    g_mpp.mpp_frame_set_width = (mpp_frame_set_width_fn)load_symbol(handle, "mpp_frame_set_width");
    g_mpp.mpp_frame_set_height = (mpp_frame_set_height_fn)load_symbol(handle, "mpp_frame_set_height");
    g_mpp.mpp_frame_set_hor_stride = (mpp_frame_set_hor_stride_fn)load_symbol(handle, "mpp_frame_set_hor_stride");
    g_mpp.mpp_frame_set_ver_stride = (mpp_frame_set_ver_stride_fn)load_symbol(handle, "mpp_frame_set_ver_stride");
    g_mpp.mpp_frame_set_fmt = (mpp_frame_set_fmt_fn)load_symbol(handle, "mpp_frame_set_fmt");
    g_mpp.mpp_frame_set_buffer = (mpp_frame_set_buffer_fn)load_symbol(handle, "mpp_frame_set_buffer");
    g_mpp.mpp_frame_set_eos = (mpp_frame_set_eos_fn)load_symbol(handle, "mpp_frame_set_eos");
    g_mpp.mpp_packet_get_length = (mpp_packet_get_length_fn)load_symbol(handle, "mpp_packet_get_length");
    g_mpp.mpp_packet_get_data = (mpp_packet_get_data_fn)load_symbol(handle, "mpp_packet_get_data");
    g_mpp.mpp_packet_deinit = (mpp_packet_deinit_fn)load_symbol(handle, "mpp_packet_deinit");

    if (!g_mpp.mpp_create || !g_mpp.mpp_init || !g_mpp.mpp_destroy ||
        !g_mpp.mpp_enc_cfg_init || !g_mpp.mpp_enc_cfg_deinit || !g_mpp.mpp_enc_cfg_set_s32 ||
        !g_mpp.mpp_buffer_group_get || !g_mpp.mpp_buffer_group_put || !g_mpp.mpp_buffer_get_with_tag ||
        !g_mpp.mpp_buffer_get_ptr_with_caller || !g_mpp.mpp_buffer_put_with_caller ||
        !g_mpp.mpp_frame_init || !g_mpp.mpp_frame_deinit || !g_mpp.mpp_frame_set_width ||
        !g_mpp.mpp_frame_set_height || !g_mpp.mpp_frame_set_hor_stride ||
        !g_mpp.mpp_frame_set_ver_stride || !g_mpp.mpp_frame_set_fmt ||
        !g_mpp.mpp_frame_set_buffer || !g_mpp.mpp_frame_set_eos ||
        !g_mpp.mpp_packet_get_length || !g_mpp.mpp_packet_get_data || !g_mpp.mpp_packet_deinit)
    {
        dlclose(handle);
        memset(&g_mpp, 0, sizeof(g_mpp));
        g_mpp.attempted = 1;
        return -1;
    }

    g_mpp.handle = handle;
    g_mpp.ready = 1;
    return 0;
}

/* ── RGA: colour-convert + scale ─────────────────────────────────────────────
 *
 * 將 src_va 指向的 src_fmt 格式影像轉換（並可縮放）為 dst_fmt 格式，
 * 寫入 dst_va。兩個緩衝區都在 process 虛擬記憶體中即可；
 * RGA driver 會自行處理 DMA 映射。
 *
 * RK_FORMAT_* 常數定義於 <rga/rga.h>：
 *   RK_FORMAT_YCbCr_420_SP  = 0xa00  (NV12)
 *   RK_FORMAT_YCbCr_422_SP  = 0x800  (NV16)
 *   RK_FORMAT_BGR_888       = 0x700  (BGR24)
 *   RK_FORMAT_RGB_888       = 0x200  (RGB24)
 *
 * 回傳值：>= 0 成功（IM_STATUS_SUCCESS = 1），< 0 失敗。
 */
int rk_rga_cvt_resize(
    void *src_va, int src_w, int src_h, int src_fmt,
    void *dst_va, int dst_w, int dst_h, int dst_fmt)
{
    if (ensure_rga_api() != 0)
        return -1;

    rga_buffer_t src = g_rga.wrapbuffer_virtualaddr_t_fn(
        src_va, src_w, src_h, src_w, src_h, src_fmt);
    rga_buffer_t dst = g_rga.wrapbuffer_virtualaddr_t_fn(
        dst_va, dst_w, dst_h, dst_w, dst_h, dst_fmt);
    rga_buffer_t pat;
    memset(&pat, 0, sizeof(pat));

    /* IM_C_API improcess(src, dst, pat, srect, drect, prect, usage)
     * 傳遞完整的 src/dst 矩形，RGA 會自動進行格式轉換和縮放。
     * IM_SYNC (= 1<<19) 確保呼叫同步完成。 */
    im_rect srect = {0, 0, src_w, src_h};
    im_rect drect = {0, 0, dst_w, dst_h};
    im_rect prect = {0, 0, 0, 0};
    return (int)g_rga.improcess_fn(src, dst, pat, srect, drect, prect, IM_SYNC);
}

/* ── MPP JPEG encoder ────────────────────────────────────────────────────────
 *
 * 一個 RkMppJpeg 實例對應一個固定解析度的 NV12 → JPEG 編碼器。
 * 請在同一個解析度的場景重複使用同一個實例以避免反覆初始化開銷。
 */
typedef struct RkMppJpeg
{
    MppCtx ctx;
    MppApi *mpi;
    int width;
    int height;
    MppFrameFormat format;
    MppBufferGroup grp; /* 預先分配，避免每幀重建 IOMMU 映射 */
    MppBuffer in_buf;
    void *in_ptr;
    size_t in_size;
} RkMppJpeg;

/*
 * rk_mpp_jpeg_create — 建立編碼器。
 *   quality: JPEG 量化參數 (1–99，愈大品質愈好)。
 * 成功回傳非 NULL；失敗回傳 NULL。
 * 呼叫者必須在不需要時呼叫 rk_mpp_jpeg_destroy() 釋放資源。
 */
RkMppJpeg *rk_mpp_jpeg_create(int width, int height, int quality, int format)
{
    if (ensure_mpp_api() != 0)
        return NULL;

    RkMppJpeg *enc = (RkMppJpeg *)calloc(1, sizeof(RkMppJpeg));
    if (!enc)
        return NULL;

    if (g_mpp.mpp_create(&enc->ctx, &enc->mpi) != MPP_OK)
        goto fail;
    if (g_mpp.mpp_init(enc->ctx, MPP_CTX_ENC, MPP_VIDEO_CodingMJPEG) != MPP_OK)
        goto fail;

    MppEncCfg cfg = NULL;
    if (g_mpp.mpp_enc_cfg_init(&cfg) != MPP_OK)
        goto fail;

    g_mpp.mpp_enc_cfg_set_s32(cfg, "prep:width", width);
    g_mpp.mpp_enc_cfg_set_s32(cfg, "prep:height", height);
    g_mpp.mpp_enc_cfg_set_s32(cfg, "prep:hor_stride", width);
    g_mpp.mpp_enc_cfg_set_s32(cfg, "prep:ver_stride", height);
    g_mpp.mpp_enc_cfg_set_s32(cfg, "prep:format", format);
    g_mpp.mpp_enc_cfg_set_s32(cfg, "codec:type", MPP_VIDEO_CodingMJPEG);
    g_mpp.mpp_enc_cfg_set_s32(cfg, "jpeg:quant", quality);

    enc->mpi->control(enc->ctx, MPP_ENC_SET_CFG, cfg);
    g_mpp.mpp_enc_cfg_deinit(cfg);

    enc->width = width;
    enc->height = height;
    enc->format = (MppFrameFormat)format;

    /* 預先分配輸入 ION buffer，避免每幀重建 IOMMU 映射 */
    size_t in_size = (size_t)width * height * 3; /* NV12: *1.5; NV24: *3; 用 *3 覆蓋兩者 */
    if (g_mpp.mpp_buffer_group_get(&enc->grp, MPP_BUFFER_TYPE_ION, MPP_BUFFER_INTERNAL,
                                   HW_SHIM_TAG, __FUNCTION__) != MPP_OK)
        goto fail;
    if (g_mpp.mpp_buffer_get_with_tag(enc->grp, &enc->in_buf, in_size,
                                      HW_SHIM_TAG, __FUNCTION__) != MPP_OK)
        goto fail;
    enc->in_ptr = g_mpp.mpp_buffer_get_ptr_with_caller(enc->in_buf, __FUNCTION__);
    if (!enc->in_ptr)
        goto fail;
    enc->in_size = in_size;
    return enc;

fail:
    if (enc->in_buf)
        g_mpp.mpp_buffer_put_with_caller(enc->in_buf, __FUNCTION__);
    if (enc->grp)
        g_mpp.mpp_buffer_group_put(enc->grp);
    if (enc->ctx)
        g_mpp.mpp_destroy(enc->ctx);
    free(enc);
    return NULL;
}

/* 銷毀並釋放編碼器資源。enc 為 NULL 時為 no-op。 */
void rk_mpp_jpeg_destroy(RkMppJpeg *enc)
{
    if (!enc)
        return;
    if (ensure_mpp_api() == 0)
    {
        if (enc->in_buf)
            g_mpp.mpp_buffer_put_with_caller(enc->in_buf, __FUNCTION__);
        if (enc->grp)
            g_mpp.mpp_buffer_group_put(enc->grp);
        g_mpp.mpp_destroy(enc->ctx);
    }
    free(enc);
}

/*
 * rk_mpp_jpeg_encode — 將一幀 NV12 / NV24 資料編碼為 JPEG。
 *   nv12_data / nv12_size : 輸入 YUV 緩衝區（大小取決於 enc->format）。
 *   out_buf   / out_cap   : 輸出緩衝區（呼叫者分配，建議 2× NV12 大小）。
 * 回傳值：> 0 為實際寫入 out_buf 的位元組數；-1 表示失敗。
 */
int rk_mpp_jpeg_encode(RkMppJpeg *enc,
                       const void *nv12_data, size_t nv12_size,
                       void *out_buf, size_t out_cap)
{
    if (!enc || ensure_mpp_api() != 0)
        return -1;
    if (!enc->in_ptr || nv12_size > enc->in_size)
        return -1;

    /* 直接複製到預先分配的 ION buffer（IOMMU 映射在 create 時已建立） */
    memcpy(enc->in_ptr, nv12_data, nv12_size);

    MppFrame frame = NULL;
    g_mpp.mpp_frame_init(&frame);
    g_mpp.mpp_frame_set_width(frame, (RK_U32)enc->width);
    g_mpp.mpp_frame_set_height(frame, (RK_U32)enc->height);
    g_mpp.mpp_frame_set_hor_stride(frame, (RK_U32)enc->width);
    g_mpp.mpp_frame_set_ver_stride(frame, (RK_U32)enc->height);
    g_mpp.mpp_frame_set_fmt(frame, enc->format);
    g_mpp.mpp_frame_set_buffer(frame, enc->in_buf);
    g_mpp.mpp_frame_set_eos(frame, 1);

    int ret = -1;
    if (enc->mpi->encode_put_frame(enc->ctx, frame) == MPP_OK)
    {
        MppPacket pkt = NULL;
        if (enc->mpi->encode_get_packet(enc->ctx, &pkt) == MPP_OK && pkt)
        {
            size_t len = g_mpp.mpp_packet_get_length(pkt);
            if (len > 0 && len <= out_cap)
            {
                memcpy(out_buf, g_mpp.mpp_packet_get_data(pkt), len);
                ret = (int)len;
            }
            g_mpp.mpp_packet_deinit(&pkt);
        }
    }

    g_mpp.mpp_frame_deinit(&frame);
    return ret;
}
