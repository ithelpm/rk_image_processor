use opencv::{
    core::{ self, Mat, Point, Rect, Scalar, Size, CV_8UC3 },
    imgproc,
    prelude::*,
    videoio,
};
use std::{ sync::mpsc::sync_channel, thread, time::Duration };

use crate::decoder::PpocrDecoder;
use crate::hw::{ rga, MppJpegEncoder, HailoVDevice };
use crate::hw::rga::fmt as RkFmt;
use crate::infer::{
    extract_rois_from_heatmap,
    HailoHef,
    HailoNetworkGroup,
    HailoVStreams,
    PostprocessConfig,
};

use super::image::{
    build_mock_annotations,
    build_tiles,
    clamp_roi_to_frame,
    draw_annotations,
    draw_manual_rois,
    draw_mock_background,
    map_roi_to_source,
    prepare_ocr_input,
    publish_preview_frame,
    DET_INPUT_HEIGHT,
    DET_INPUT_WIDTH,
    OCR_INPUT_HEIGHT,
    OCR_INPUT_WIDTH,
};
use super::options::CameraSource;
use super::state::{ BackendState, MppJpegState, DEFAULT_FRAME_HEIGHT, DEFAULT_FRAME_WIDTH };
use super::types::OcrAnnotation;

// ── Live runtime ──────────────────────────────────────────────────────────────

pub fn start_live_runtime(state: BackendState) -> Result<(), Box<dyn std::error::Error>> {
    let (ocr_tx, ocr_rx) = sync_channel::<Mat>(1);
    let capture_state = state.clone();
    let ocr_state = state.clone();

    // ── Capture thread: restartable on camera source change ──────────────────
    thread::spawn(move || {
        // Subscribe before the first open — so any changes during open are detected.
        let mut restart_rx = capture_state.camera_restart_tx.subscribe();

        loop {
            let source = capture_state.snapshot_camera_source();
            restart_rx.mark_unchanged(); // baseline for this iteration

            match &source {
                CameraSource::Index(n) => {
                    run_opencv_capture(*n, &source, &capture_state, &ocr_tx, &mut restart_rx);
                }
                CameraSource::Path(p) => {
                    run_v4l_mplane_capture(p, &capture_state, &ocr_tx, &mut restart_rx);
                }
            }
        }
    });

    // ── OCR inference thread ──────────────────────────────────────────────────
    // All Hailo resources are created *inside* this thread so they live for the
    // full duration of the service.  Hailo's C handle types are not Send, so
    // they cannot be moved across thread boundaries; creating them here avoids
    // that constraint entirely and also ensures correct lifetime semantics.
    thread::spawn(move || {
        let decoder = match
            PpocrDecoder::from_content(include_str!("../../../resources/en_dict.txt"))
        {
            Ok(d) => d,
            Err(e) => {
                ocr_state.set_error(format!("字典載入失敗: {e}"));
                return;
            }
        };
        let vdevice = match HailoVDevice::new() {
            Ok(v) => v,
            Err(e) => {
                ocr_state.set_error(format!("Hailo 設備開啟失敗: {e}"));
                return;
            }
        };
        let det_hef = match HailoHef::from_buffer(include_bytes!("../../../resources/ocr_det.hef")) {
            Ok(h) => h,
            Err(e) => {
                ocr_state.set_error(format!("det HEF 載入失敗: {e}"));
                return;
            }
        };
        let rec_hef = match HailoHef::from_buffer(include_bytes!("../../../resources/ocr.hef")) {
            Ok(h) => h,
            Err(e) => {
                ocr_state.set_error(format!("rec HEF 載入失敗: {e}"));
                return;
            }
        };
        let det_network = match HailoNetworkGroup::configure(&vdevice, &det_hef) {
            Ok(n) => n,
            Err(e) => {
                ocr_state.set_error(format!("det 網路配置失敗: {e}"));
                return;
            }
        };
        let rec_network = match HailoNetworkGroup::configure(&vdevice, &rec_hef) {
            Ok(n) => n,
            Err(e) => {
                ocr_state.set_error(format!("rec 網路配置失敗: {e}"));
                return;
            }
        };
        let det_vstreams = match HailoVStreams::create(&det_network) {
            Ok(v) => v,
            Err(e) => {
                ocr_state.set_error(format!("det vstream 建立失敗: {e}"));
                return;
            }
        };
        let rec_vstreams = match HailoVStreams::create(&rec_network) {
            Ok(v) => v,
            Err(e) => {
                ocr_state.set_error(format!("rec vstream 建立失敗: {e}"));
                return;
            }
        };
        let det_input_size = (DET_INPUT_HEIGHT * DET_INPUT_WIDTH * 3) as usize;
        let det_output_size = (DET_INPUT_HEIGHT * DET_INPUT_WIDTH) as usize;
        let rec_input_size = (OCR_INPUT_HEIGHT * OCR_INPUT_WIDTH * 3) as usize;
        let rec_output_size = 40 * 97;

        let mut det_output = vec![0u8; det_output_size];
        let mut rec_output = vec![0u8; rec_output_size];

        while let Ok(frame) = ocr_rx.recv() {
            let config = ocr_state.snapshot_config();
            let manual_rois = ocr_state.snapshot_manual_rois();

            let annotations = if config.manual_roi_mode {
                recognize_manual_rois(
                    &frame,
                    &decoder,
                    &rec_vstreams,
                    &mut rec_output,
                    &manual_rois
                )
            } else {
                recognize_auto_rois(
                    &frame,
                    &decoder,
                    &det_vstreams,
                    &rec_vstreams,
                    &mut det_output,
                    &mut rec_output,
                    &config,
                    det_input_size,
                    rec_input_size
                )
            };

            ocr_state.replace_annotations(annotations);
        }
    });

    Ok(())
}

// ── Mock runtime ──────────────────────────────────────────────────────────────

pub fn spawn_mock_runtime(state: BackendState) {
    thread::spawn(move || {
        let mut frame_index: i32 = 0;
        loop {
            let mut frame = Mat::new_rows_cols_with_default(
                DEFAULT_FRAME_HEIGHT,
                DEFAULT_FRAME_WIDTH,
                CV_8UC3,
                Scalar::new(12.0, 18.0, 28.0, 0.0)
            ).unwrap_or_default();

            draw_mock_background(&mut frame, frame_index);

            let config = state.snapshot_config();
            let manual_rois = state.snapshot_manual_rois();
            let annotations = build_mock_annotations(frame_index, &config, &manual_rois);
            state.replace_annotations(annotations.clone());
            state.set_frame_size(DEFAULT_FRAME_WIDTH, DEFAULT_FRAME_HEIGHT);

            draw_annotations(&mut frame, &annotations);
            if !manual_rois.is_empty() {
                draw_manual_rois(&mut frame, &manual_rois);
            }

            let label = format!(
                "MOCK MODE | threshold {:.0} | interval {} | manual {}",
                config.threshold_value,
                config.ocr_frame_interval,
                if config.manual_roi_mode {
                    "on"
                } else {
                    "off"
                }
            );
            let _ = imgproc::put_text(
                &mut frame,
                &label,
                Point::new(24, 44),
                imgproc::FONT_HERSHEY_SIMPLEX,
                0.8,
                Scalar::new(255.0, 255.0, 255.0, 0.0),
                2,
                imgproc::LINE_AA,
                false
            );

            if let Err(error) = publish_preview_frame(&frame, &state.frame_tx, None) {
                state.set_error(format!("無法編碼 mock frame: {error}"));
            }

            thread::sleep(Duration::from_millis(120));
            frame_index = frame_index.wrapping_add(1);
        }
    });
}

// ── Capture loop: OpenCV VideoCapture (integer index / webcam) ────────────────

fn run_opencv_capture(
    index: i32,
    source: &CameraSource,
    state: &super::state::BackendState,
    ocr_tx: &std::sync::mpsc::SyncSender<Mat>,
    restart_rx: &mut tokio::sync::watch::Receiver<u64>
) {
    let mut camera = match videoio::VideoCapture::new(index, videoio::CAP_V4L2) {
        Ok(cam) => cam,
        Err(e) => {
            state.set_error(format!("無法開啟攝影機 {index}: {e}"));
            thread::sleep(Duration::from_secs(2));
            return;
        }
    };
    match camera.is_opened() {
        Ok(true) => state.clear_error(),
        _ => {
            state.set_error(format!("攝影機 {index} 開啟失敗"));
            thread::sleep(Duration::from_secs(2));
            return;
        }
    }
    let mut frame = Mat::default();
    let mut frame_index: i64 = 0;
    loop {
        if restart_rx.has_changed().unwrap_or(false) {
            return;
        }
        match camera.read(&mut frame) {
            Ok(true) if !frame.empty() => {}
            Ok(_) => {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(e) => {
                state.set_error(format!("攝影機讀取失敗: {e}"));
                thread::sleep(Duration::from_millis(50));
                continue;
            }
        }
        // Webcams mounted upside-down — flip vertically.
        let mut working_frame = Mat::default();
        if core::flip(&frame, &mut working_frame, 0).is_err() {
            continue;
        }
        publish_and_send(&working_frame, state, ocr_tx, source, frame_index);
        frame_index += 1;
    }
}

// ── Capture loop: native V4L2 MPLANE (device path, e.g. rk_hdmirx HDMI IN) ──

fn run_v4l_mplane_capture(
    path: &str,
    state: &super::state::BackendState,
    ocr_tx: &std::sync::mpsc::SyncSender<Mat>,
    restart_rx: &mut tokio::sync::watch::Receiver<u64>
) {
    use std::os::unix::io::AsRawFd;

    // Query format via raw ioctl (V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE = 9).
    let (width, height, fourcc, _stride) = match query_mplane_format(path) {
        Ok(t) => t,
        Err(e) => {
            state.set_error(format!("無法查詢 V4L2 格式 ({path}): {e}"));
            thread::sleep(Duration::from_secs(2));
            return;
        }
    };
    let width = width as i32;
    let height = height as i32;

    // Open the device file for direct ioctl access.
    let file = match std::fs::OpenOptions::new().read(true).write(true).open(path) {
        Ok(f) => f,
        Err(e) => {
            state.set_error(format!("無法開啟 V4L2 設備 {path}: {e}"));
            thread::sleep(Duration::from_secs(2));
            return;
        }
    };
    let fd = file.as_raw_fd();

    // VIDIOC_REQBUFS — allocate kernel-side buffers.
    let n_bufs = match mplane_reqbufs(fd, 4) {
        Ok(n) if n > 0 => n,
        Ok(_) => {
            state.set_error(format!("MPLANE REQBUFS 傳回 0 個緩衝區 ({path})"));
            thread::sleep(Duration::from_secs(2));
            return;
        }
        Err(e) => {
            state.set_error(format!("MPLANE REQBUFS 失敗 ({path}): {e}"));
            thread::sleep(Duration::from_secs(2));
            return;
        }
    };

    // VIDIOC_QUERYBUF + mmap each buffer.
    let mut mappings: Vec<(*mut libc::c_void, usize)> = Vec::with_capacity(n_bufs as usize);
    for i in 0..n_bufs {
        match mplane_querybuf_mmap(fd, i) {
            Ok(m) => mappings.push(m),
            Err(e) => {
                for (ptr, len) in &mappings {
                    unsafe {
                        libc::munmap(*ptr, *len);
                    }
                }
                state.set_error(format!("MPLANE QUERYBUF/mmap 失敗 ({path}, buf {i}): {e}"));
                thread::sleep(Duration::from_secs(2));
                return;
            }
        }
    }

    // VIDIOC_QBUF — enqueue all buffers.
    for i in 0..n_bufs {
        if let Err(e) = mplane_qbuf(fd, i) {
            for (ptr, len) in &mappings {
                unsafe {
                    libc::munmap(*ptr, *len);
                }
            }
            state.set_error(format!("MPLANE QBUF 失敗 ({path}, buf {i}): {e}"));
            thread::sleep(Duration::from_secs(2));
            return;
        }
    }

    // VIDIOC_STREAMON
    if let Err(e) = mplane_streamon(fd) {
        for (ptr, len) in &mappings {
            unsafe {
                libc::munmap(*ptr, *len);
            }
        }
        state.set_error(format!("MPLANE STREAMON 失敗 ({path}): {e}"));
        thread::sleep(Duration::from_secs(2));
        return;
    }

    state.clear_error();

    let mut frame_index: i64 = 0;
    let mut fatal_stream_error = false;
    loop {
        if restart_rx.has_changed().unwrap_or(false) {
            break;
        }

        // poll with a short timeout so restart_rx is checked periodically.
        let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        let ready = unsafe { libc::poll(&mut pfd, 1, 200) };
        if ready == 0 {
            continue; // timeout — re-check restart_rx
        }
        if ready < 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            state.set_error(format!("MPLANE poll 失敗 ({path}): {e}"));
            fatal_stream_error = true;
            break;
        }

        // VIDIOC_DQBUF — dequeue one filled buffer.
        let buf_idx = match mplane_dqbuf(fd) {
            Ok(i) => i,
            Err(e) => {
                if is_retryable_mplane_error(&e) {
                    continue;
                }
                state.set_error(format!("MPLANE DQBUF 失敗 ({path}): {e}"));
                fatal_stream_error = true;
                break;
            }
        };

        let (ptr, len) = mappings[buf_idx as usize];
        // SAFETY: ptr is a valid mmap'd buffer of `len` bytes; kernel will not
        // write to it again until we QBUF it back below.
        let slice = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) };

        let frame = match raw_to_bgr_mat(slice, width, height, fourcc) {
            Ok(f) => f,
            Err(e) => {
                state.set_error(format!("像素格式轉換失敗: {e}"));
                let _ = mplane_qbuf(fd, buf_idx);
                fatal_stream_error = true;
                break;
            }
        };

        // Return the buffer to the driver before heavy processing.
        if let Err(e) = mplane_qbuf(fd, buf_idx) {
            state.set_error(format!("MPLANE QBUF 失敗 ({path}, buf {buf_idx}): {e}"));
            fatal_stream_error = true;
            break;
        }

        let source = CameraSource::Path(path.to_string());
        publish_and_send(&frame, state, ocr_tx, &source, frame_index);
        frame_index += 1;
    }

    // ── Cleanup ───────────────────────────────────────────────────────────────
    let _ = mplane_streamoff(fd);
    for (ptr, len) in mappings {
        unsafe {
            libc::munmap(ptr, len);
        }
    }

    if fatal_stream_error {
        thread::sleep(Duration::from_secs(2));
    }
}

fn is_retryable_mplane_error(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(code) if code == libc::EAGAIN || code == libc::EINTR)
}

// ── Raw MPLANE ioctl helpers ──────────────────────────────────────────────────
//
// The v4l 0.14 mmap Stream calls VIDIOC_QUERYBUF without setting m.planes,
// causing EINVAL on strict MPLANE-only drivers (e.g. rk_hdmirx on RK3588).
// The helpers below bypass the v4l crate entirely.
//
// All struct layouts are verified for aarch64 / 64-bit Linux.

const V4L2_MEMORY_MMAP: u32 = 1;
const V4L2_BUF_TYPE_MPLANE: u32 = 9; // V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE

// VIDIOC_REQBUFS  = _IOWR('V',  8, struct v4l2_requestbuffers[20])  = 0xC014_5608
const VIDIOC_REQBUFS: libc::c_ulong = 0xc014_5608;
// VIDIOC_QUERYBUF = _IOWR('V',  9, struct v4l2_buffer[88])          = 0xC058_5609
const VIDIOC_QUERYBUF: libc::c_ulong = 0xc058_5609;
// VIDIOC_QBUF     = _IOWR('V', 15, struct v4l2_buffer[88])          = 0xC058_560F
const VIDIOC_QBUF: libc::c_ulong = 0xc058_560f;
// VIDIOC_DQBUF    = _IOWR('V', 17, struct v4l2_buffer[88])          = 0xC058_5611
const VIDIOC_DQBUF: libc::c_ulong = 0xc058_5611;
// VIDIOC_STREAMON  = _IOW('V', 18, __u32[4])                        = 0x4004_5612
const VIDIOC_STREAMON: libc::c_ulong = 0x4004_5612;
// VIDIOC_STREAMOFF = _IOW('V', 19, __u32[4])                        = 0x4004_5613
const VIDIOC_STREAMOFF: libc::c_ulong = 0x4004_5613;

/// `struct v4l2_requestbuffers` — sizeof = 20 (aarch64, kernel ≥ 5.2)
#[repr(C)]
struct V4l2ReqBuffers {
    count: u32, // [0..4]
    buffer_type: u32, // [4..8]
    memory: u32, // [8..12]
    capabilities: u32, // [12..16]
    flags: u8, // [16]
    reserved: [u8; 3], // [17..20]
}

/// `struct v4l2_plane` — sizeof = 64 (aarch64, 64-bit)
///
/// The `m` union contains `unsigned long userptr` (8 bytes on 64-bit), so the
/// union sits at offset 8 (already 8-byte aligned after two u32 fields) and
/// occupies bytes [8..16].  `mem_offset` is a u32 at the start of that union.
#[repr(C)]
struct V4l2Plane {
    bytesused: u32, // [0..4]
    length: u32, // [4..8]
    m_mem_offset: u32, // [8..12]  — mem_offset (low 32 bits of 8-byte union)
    _m_pad: u32, // [12..16] — high 32 bits (unsigned long padding)
    data_offset: u32, // [16..20]
    _reserved: [u32; 11], // [20..64]
}

/// `struct v4l2_buffer` — sizeof = 88 (aarch64, 64-bit)
///
/// `struct timeval` uses `__kernel_long_t` (= i64 on 64-bit), so it requires
/// 8-byte alignment; 4 bytes of padding appear at [20..24].
/// The `m` union holds a pointer (`struct v4l2_plane *`) which is 8 bytes,
/// aligned at offset 64 (naturally 8-byte aligned).
#[repr(C)]
struct V4l2Buffer {
    index: u32, // [0..4]
    buffer_type: u32, // [4..8]
    bytesused: u32, // [8..12]
    flags: u32, // [12..16]
    field: u32, // [16..20]
    _pad0: u32, // [20..24] implicit padding before timeval
    ts_sec: i64, // [24..32]
    ts_usec: i64, // [32..40]
    tc_type: u32, // [40..44]  ┐
    tc_flags: u32, // [44..48]  │ struct v4l2_timecode (16 bytes)
    tc_fsmh: u32, // [48..52]  │ frames+seconds+minutes+hours
    tc_userbits: u32, // [52..56]  ┘
    sequence: u32, // [56..60]
    memory: u32, // [60..64]
    m_planes_ptr: u64, // [64..72]  *mut V4l2Plane (8-byte pointer on aarch64)
    length: u32, // [72..76]  for MPLANE: number of planes
    reserved2: u32, // [76..80]
    request_fd: i32, // [80..84]
    _pad1: u32, // [84..88]
}

impl V4l2Buffer {
    fn new_mplane(index: u32, planes_ptr: *mut V4l2Plane) -> Self {
        V4l2Buffer {
            index,
            buffer_type: V4L2_BUF_TYPE_MPLANE,
            memory: V4L2_MEMORY_MMAP,
            length: 1, // 1 plane
            m_planes_ptr: planes_ptr as u64,
            bytesused: 0,
            flags: 0,
            field: 0,
            _pad0: 0,
            ts_sec: 0,
            ts_usec: 0,
            tc_type: 0,
            tc_flags: 0,
            tc_fsmh: 0,
            tc_userbits: 0,
            sequence: 0,
            reserved2: 0,
            request_fd: 0,
            _pad1: 0,
        }
    }
}

fn mplane_reqbufs(fd: libc::c_int, count: u32) -> std::io::Result<u32> {
    let mut req = V4l2ReqBuffers {
        count,
        buffer_type: V4L2_BUF_TYPE_MPLANE,
        memory: V4L2_MEMORY_MMAP,
        capabilities: 0,
        flags: 0,
        reserved: [0; 3],
    };
    // SAFETY: req is a valid V4l2ReqBuffers; fd is open.
    let ret = unsafe { libc::ioctl(fd, VIDIOC_REQBUFS, &mut req as *mut _) };
    if ret < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(req.count)
    }
}

fn mplane_querybuf_mmap(
    fd: libc::c_int,
    index: u32
) -> std::io::Result<(*mut libc::c_void, usize)> {
    // SAFETY: V4l2Plane is a plain-old-data C struct; zero-initialisation is valid.
    let mut plane = unsafe { std::mem::zeroed::<V4l2Plane>() };
    let mut buf = V4l2Buffer::new_mplane(index, &mut plane);
    // SAFETY: buf and plane are valid stack memory; fd is open.
    let ret = unsafe { libc::ioctl(fd, VIDIOC_QUERYBUF, &mut buf as *mut _) };
    if ret < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let offset = plane.m_mem_offset as libc::off_t;
    let length = plane.length as usize;
    // SAFETY: standard V4L2 mmap pattern.
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            length,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            offset
        )
    };
    if ptr == libc::MAP_FAILED {
        return Err(std::io::Error::last_os_error());
    }
    Ok((ptr, length))
}

fn mplane_qbuf(fd: libc::c_int, index: u32) -> std::io::Result<()> {
    // SAFETY: V4l2Plane is a plain-old-data C struct; zero-initialisation is valid.
    let mut plane = unsafe { std::mem::zeroed::<V4l2Plane>() };
    let mut buf = V4l2Buffer::new_mplane(index, &mut plane);
    let ret = unsafe { libc::ioctl(fd, VIDIOC_QBUF, &mut buf as *mut _) };
    if ret < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn mplane_dqbuf(fd: libc::c_int) -> std::io::Result<u32> {
    // SAFETY: V4l2Plane is a plain-old-data C struct; zero-initialisation is valid.
    let mut plane = unsafe { std::mem::zeroed::<V4l2Plane>() };
    let mut buf = V4l2Buffer::new_mplane(0, &mut plane);
    let ret = unsafe { libc::ioctl(fd, VIDIOC_DQBUF, &mut buf as *mut _) };
    if ret < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(buf.index)
    }
}

fn mplane_streamon(fd: libc::c_int) -> std::io::Result<()> {
    let buf_type = V4L2_BUF_TYPE_MPLANE;
    let ret = unsafe { libc::ioctl(fd, VIDIOC_STREAMON, &buf_type as *const u32) };
    if ret < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn mplane_streamoff(fd: libc::c_int) -> std::io::Result<()> {
    let buf_type = V4L2_BUF_TYPE_MPLANE;
    let ret = unsafe { libc::ioctl(fd, VIDIOC_STREAMOFF, &buf_type as *const u32) };
    if ret < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Query width, height, fourcc, and bytesperline of a V4L2 MPLANE capture device.
///
/// The `v4l::video::Capture` trait issues `VIDIOC_G_FMT` with
/// `V4L2_BUF_TYPE_VIDEO_CAPTURE` (= 1).  Strict MPLANE-only drivers (e.g.
/// `rk_hdmirx` on RK3588) reject that with `EINVAL`.  This function uses a
/// raw ioctl with `V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE` (= 9) instead.
///
/// Layout of `struct v4l2_format` on 64-bit Linux (aarch64/x86_64):
/// ```text
///   [  0..  4]  __u32  type
///   [  4..  8]  padding (pointer in v4l2_window forces union to 8-byte align)
///   [  8.. 12]  pix_mp.width
///   [ 12.. 16]  pix_mp.height
///   [ 16.. 20]  pix_mp.pixelformat  (fourcc)
///   [ 20.. 24]  pix_mp.field
///   [ 24.. 28]  pix_mp.colorspace
///   [ 28.. 32]  pix_mp.plane_fmt[0].sizeimage
///   [ 32.. 36]  pix_mp.plane_fmt[0].bytesperline
/// ```
/// `sizeof(v4l2_format)` = 208 = 0xD0, so `VIDIOC_G_FMT` = 0xC0D0_5604.
fn query_mplane_format(path: &str) -> std::io::Result<(u32, u32, v4l::FourCC, u32)> {
    use std::os::unix::io::AsRawFd;

    // VIDIOC_G_FMT = _IOWR('V', 4, struct v4l2_format) = 0xC0D0_5604
    const VIDIOC_G_FMT: libc::c_ulong = 0xc0d0_5604;
    const V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE: u32 = 9;

    // Open a plain File so we have a raw fd without fighting v4l::Device's API.
    let file = std::fs::OpenOptions::new().read(true).write(true).open(path)?;
    let fd = file.as_raw_fd();

    let mut buf = [0u8; 208];
    buf[0..4].copy_from_slice(&V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE.to_ne_bytes());

    // SAFETY: `buf` is a 208-byte zero-initialised v4l2_format; fd is valid and open.
    let ret = unsafe { libc::ioctl(fd, VIDIOC_G_FMT, buf.as_mut_ptr()) };
    if ret < 0 {
        return Err(std::io::Error::last_os_error());
    }

    let base = 8usize; // pix_mp starts after type(4) + padding(4)
    let width = u32::from_ne_bytes(buf[base..base + 4].try_into().unwrap());
    let height = u32::from_ne_bytes(buf[base + 4..base + 8].try_into().unwrap());
    let fcc_bytes: [u8; 4] = buf[base + 8..base + 12].try_into().unwrap();
    let fourcc = v4l::FourCC::new(&fcc_bytes);
    let bytesperline = u32::from_ne_bytes(buf[base + 24..base + 28].try_into().unwrap());

    Ok((width, height, fourcc, bytesperline))
}

fn clamp_to_u8(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

fn decode_nv24_to_bgr(buf: &[u8], width: i32, height: i32) -> opencv::Result<Mat> {
    let y_size = (width * height) as usize;
    let uv_size = y_size * 2;
    let total = y_size + uv_size;
    if buf.len() < total {
        return Err(
            opencv::Error::new(
                opencv::core::StsError,
                format!("NV24 資料不足: 需要 {total} B，實際 {} B", buf.len())
            )
        );
    }

    let y_plane = &buf[..y_size];
    let uv_plane = &buf[y_size..y_size + uv_size];
    let mut dst = vec![0u8; y_size * 3];

    for index in 0..y_size {
        let y = y_plane[index] as i32;
        let u = (uv_plane[index * 2] as i32) - 128;
        let v = (uv_plane[index * 2 + 1] as i32) - 128;

        let r = y + ((359 * v) >> 8);
        let g = y - ((88 * u + 183 * v) >> 8);
        let b = y + ((454 * u) >> 8);

        let dst_index = index * 3;
        dst[dst_index] = clamp_to_u8(b);
        dst[dst_index + 1] = clamp_to_u8(g);
        dst[dst_index + 2] = clamp_to_u8(r);
    }

    let bytes_mat = Mat::from_slice(&dst)?;
    let frame_ref = bytes_mat.reshape(3, height)?;
    let mut out = Mat::default();
    frame_ref.copy_to(&mut out)?;
    Ok(out)
}

/// Convert a raw V4L2 plane-0 buffer to an OpenCV CV_8UC3 BGR Mat.
/// 導入時優先使用 RGA3 硬體色彩轉換；若 RGA 不可用則 fallback 至 OpenCV CPU 路徑。
fn raw_to_bgr_mat(buf: &[u8], width: i32, height: i32, fourcc: v4l::FourCC) -> opencv::Result<Mat> {
    let fcc = fourcc.str().unwrap_or_default();
    match fcc.trim_end() {
        "BGR3" => {
            // Packed BGR24 — maps directly to CV_8UC3，不需要色彩轉換。
            let expected = (width * height * 3) as usize;
            let bytes_mat = Mat::from_slice(&buf[..expected.min(buf.len())])?;
            let frame_ref = bytes_mat.reshape(3, height)?;
            let mut out = Mat::default();
            frame_ref.copy_to(&mut out)?;
            Ok(out)
        }
        "NV16" => {
            // Semi-planar YUV 4:2:2。嘗試用 RGA 直接轉成 BGR24。
            let y_size = (width * height) as usize;
            let uv_size = y_size;
            let total = y_size + uv_size;
            if buf.len() < total {
                return Err(
                    opencv::Error::new(
                        opencv::core::StsError,
                        format!("NV16 資料不足: 需要 {total} B，實際 {} B", buf.len())
                    )
                );
            }

            let dst_bytes = (width * height * 3) as usize;
            let mut dst_vec = vec![0u8; dst_bytes];
            // 註：NV16 = RK_FORMAT_YCB_CR_422_SP
            let ok = rga::rga_cvt_resize(
                &mut buf[..total].to_vec(), // RGA 需要 mutable ref，際上只讀取
                width,
                height,
                RkFmt::YCB_CR_422_SP,
                &mut dst_vec,
                width,
                height,
                RkFmt::BGR_888
            );
            if ok.is_ok() {
                let bytes_mat = Mat::from_slice(&dst_vec)?;
                let frame_ref = bytes_mat.reshape(3, height)?;
                let mut out = Mat::default();
                frame_ref.copy_to(&mut out)?;
                return Ok(out);
            }

            // OpenCV fallback：调除每一列 UV 以得到近似 NV12
            let uv_src = &buf[y_size..y_size + uv_size];
            let uv_row_bytes = (width as usize) * 2;
            let mut nv12 = Vec::with_capacity((y_size * 3) / 2);
            nv12.extend_from_slice(&buf[..y_size]);
            for row in (0..height as usize).step_by(2) {
                nv12.extend_from_slice(&uv_src[row * uv_row_bytes..(row + 1) * uv_row_bytes]);
            }
            let bytes_mat = Mat::from_slice(&nv12)?;
            let nv12_mat = bytes_mat.reshape(1, (height * 3) / 2)?;
            let mut bgr = Mat::default();
            #[cfg(opencv_algorithm_hint)]
            imgproc::cvt_color(
                &nv12_mat,
                &mut bgr,
                imgproc::COLOR_YUV2BGR_NV12,
                0,
                opencv::core::AlgorithmHint::ALGO_HINT_DEFAULT
            )?;
            #[cfg(not(opencv_algorithm_hint))]
            imgproc::cvt_color(&nv12_mat, &mut bgr, imgproc::COLOR_YUV2BGR_NV12, 0)?;
            Ok(bgr)
        }
        "NV12" => {
            // NV12。嘗試用 RGA 轉換成 BGR24。
            let total = ((width * height * 3) / 2) as usize;
            if buf.len() >= total {
                let dst_bytes = (width * height * 3) as usize;
                let mut dst_vec = vec![0u8; dst_bytes];
                let ok = rga::rga_cvt_resize(
                    &mut buf[..total].to_vec(),
                    width,
                    height,
                    RkFmt::YCB_CR_420_SP,
                    &mut dst_vec,
                    width,
                    height,
                    RkFmt::BGR_888
                );
                if ok.is_ok() {
                    let bytes_mat = Mat::from_slice(&dst_vec)?;
                    let frame_ref = bytes_mat.reshape(3, height)?;
                    let mut out = Mat::default();
                    frame_ref.copy_to(&mut out)?;
                    return Ok(out);
                }
            }

            // OpenCV fallback
            let actual_total = total.min(buf.len());
            let bytes_mat = Mat::from_slice(&buf[..actual_total])?;
            let nv12_mat = bytes_mat.reshape(1, (height * 3) / 2)?;
            let mut bgr = Mat::default();
            #[cfg(opencv_algorithm_hint)]
            imgproc::cvt_color(
                &nv12_mat,
                &mut bgr,
                imgproc::COLOR_YUV2BGR_NV12,
                0,
                opencv::core::AlgorithmHint::ALGO_HINT_DEFAULT
            )?;
            #[cfg(not(opencv_algorithm_hint))]
            imgproc::cvt_color(&nv12_mat, &mut bgr, imgproc::COLOR_YUV2BGR_NV12, 0)?;
            Ok(bgr)
        }
        "NV24" => {
            let total = (width * height * 3) as usize;
            if buf.len() >= total {
                let dst_bytes = (width * height * 3) as usize;
                let mut dst_vec = vec![0u8; dst_bytes];
                let ok = rga::rga_cvt_resize(
                    &mut buf[..total].to_vec(),
                    width,
                    height,
                    RkFmt::YCB_CR_444_SP,
                    &mut dst_vec,
                    width,
                    height,
                    RkFmt::BGR_888
                );
                if ok.is_ok() {
                    let bytes_mat = Mat::from_slice(&dst_vec)?;
                    let frame_ref = bytes_mat.reshape(3, height)?;
                    let mut out = Mat::default();
                    frame_ref.copy_to(&mut out)?;
                    return Ok(out);
                }
            }

            decode_nv24_to_bgr(buf, width, height)
        }
        other =>
            Err(
                opencv::Error::new(
                    opencv::core::StsNotImplemented,
                    format!("不支援的像素格式: {other}")
                )
            ),
    }
}

// ── Shared: publish preview frame and enqueue OCR frame ──────────────────────

fn publish_and_send(
    frame: &Mat,
    state: &super::state::BackendState,
    ocr_tx: &std::sync::mpsc::SyncSender<Mat>,
    source: &CameraSource,
    frame_index: i64
) {
    state.set_frame_size(frame.cols(), frame.rows());

    let annotations = state.snapshot_annotations();
    let manual_rois = state.snapshot_manual_rois();
    let mut preview = frame.clone();
    draw_annotations(&mut preview, &annotations);
    if !manual_rois.is_empty() {
        draw_manual_rois(&mut preview, &manual_rois);
    }

    let w = frame.cols();
    let h = frame.rows();
    let mut enc_guard = state.mpp_jpeg.lock().unwrap();
    let needs_init = match &*enc_guard {
        MppJpegState::Uninitialized => true,
        MppJpegState::Ready(enc) => enc.width() != w || enc.height() != h,
        MppJpegState::Disabled => false,
    };
    if needs_init {
        *enc_guard = match MppJpegEncoder::new(w, h, 82) {
            Some(enc) => MppJpegState::Ready(enc),
            None => MppJpegState::Disabled,
        };
    }

    if let Err(e) = publish_preview_frame(&preview, &state.frame_tx, enc_guard.encoder()) {
        state.set_error(format!("無法編碼預覽影像: {e}"));
    }
    drop(enc_guard);

    let cfg = state.snapshot_config();
    let interval = cfg.ocr_frame_interval.max(1) as i64;
    if frame_index % interval == 0 {
        let _ = ocr_tx.try_send(frame.clone());
    }
    let _ = source;
}

// ── OCR recognition helpers ───────────────────────────────────────────────────

fn recognize_auto_rois(
    frame: &Mat,
    decoder: &PpocrDecoder,
    det_vstreams: &HailoVStreams,
    rec_vstreams: &HailoVStreams,
    det_output_buffer: &mut [u8],
    rec_output_buffer: &mut [u8],
    config: &PostprocessConfig,
    det_input_size: usize,
    rec_input_size: usize
) -> Vec<OcrAnnotation> {
    let mut annotations = Vec::new();

    for tile in build_tiles(frame.cols(), frame.rows()) {
        let tile_view = core::Mat::roi(frame, tile).unwrap();

        // 嘗試用 RGA3 在單一 pass 完成 縮放(tile→960×544) + BGR→RGB 轉換。
        // 若 tile_view 不連續（ROI sub-mat），先 clone 為連續陣列。
        let det_input: Mat = {
            let src = if tile_view.is_continuous() {
                tile_view.try_clone().unwrap()
            } else {
                let mut c = Mat::default();
                tile_view.copy_to(&mut c).unwrap();
                c
            };
            let src_w = src.cols();
            let src_h = src.rows();
            let src_bytes = (src_w * src_h * 3) as usize;
            let dst_bytes = (DET_INPUT_WIDTH * DET_INPUT_HEIGHT * 3) as usize;
            let mut dst_vec = vec![0u8; dst_bytes];
            let rga_ok = rga::rga_cvt_resize(
                // SAFETY: Mat 資料在此 scope 內有效，RGA 僅讀取 src。
                unsafe {
                    std::slice::from_raw_parts_mut(src.data() as *mut u8, src_bytes)
                },
                src_w,
                src_h,
                RkFmt::BGR_888,
                &mut dst_vec,
                DET_INPUT_WIDTH,
                DET_INPUT_HEIGHT,
                RkFmt::RGB_888
            );
            if rga_ok.is_ok() {
                let m = (
                    unsafe {
                        Mat::new_rows_cols_with_data_unsafe(
                            DET_INPUT_HEIGHT,
                            DET_INPUT_WIDTH,
                            opencv::core::CV_8UC3,
                            dst_vec.as_ptr() as *mut _,
                            opencv::core::Mat_AUTO_STEP as usize
                        )
                    }
                ).unwrap();
                let mut out = Mat::default();
                m.copy_to(&mut out).unwrap();
                out
            } else {
                // OpenCV fallback
                let mut resized = Mat::default();
                imgproc
                    ::resize(
                        &src,
                        &mut resized,
                        Size::new(DET_INPUT_WIDTH, DET_INPUT_HEIGHT),
                        0.0,
                        0.0,
                        imgproc::INTER_LINEAR
                    )
                    .unwrap();
                let mut rgb = Mat::default();
                opencv::opencv_has_inherent_feature_algorithm_hint! {
                    {
                        imgproc::cvt_color(
                            &resized, &mut rgb,
                            imgproc::COLOR_BGR2RGB, 0,
                            core::AlgorithmHint::ALGO_HINT_DEFAULT,
                        ).unwrap();
                    } else {
                        imgproc::cvt_color(
                            &resized, &mut rgb,
                            imgproc::COLOR_BGR2RGB, 0,
                        ).unwrap();
                    }
                }
                rgb
            }
        };

        let det_input_cont = if det_input.is_continuous() { det_input } else { det_input.clone() };
        det_vstreams.write_input(det_input_cont.data(), det_input_size).unwrap();
        det_vstreams.read_output(det_output_buffer.as_mut_ptr(), det_output_buffer.len()).unwrap();

        let rois: Vec<Rect> = extract_rois_from_heatmap(
            det_output_buffer,
            DET_INPUT_WIDTH,
            DET_INPUT_HEIGHT,
            config
        );

        for roi in rois {
            let source_roi = map_roi_to_source(roi, tile, frame.cols(), frame.rows());
            if let Some(clamped) = clamp_roi_to_frame(source_roi, frame.cols(), frame.rows()) {
                let cropped = core::Mat::roi(frame, clamped).unwrap().try_clone().unwrap();
                let rec_input = prepare_ocr_input(&cropped).unwrap();
                let rec_cont = if rec_input.is_continuous() {
                    rec_input
                } else {
                    rec_input.clone()
                };
                rec_vstreams.write_input(rec_cont.data(), rec_input_size).unwrap();
                rec_vstreams
                    .read_output(rec_output_buffer.as_mut_ptr(), rec_output_buffer.len())
                    .unwrap();
                let text = decoder.decode(rec_output_buffer, 40, 97);
                if !text.is_empty() {
                    annotations.push(OcrAnnotation { roi: clamped, text });
                }
            }
        }
    }

    annotations
}

fn recognize_manual_rois(
    frame: &Mat,
    decoder: &PpocrDecoder,
    rec_vstreams: &HailoVStreams,
    rec_output_buffer: &mut [u8],
    manual_rois: &[Rect]
) -> Vec<OcrAnnotation> {
    let mut annotations = Vec::new();
    let rec_input_size = (OCR_INPUT_HEIGHT * OCR_INPUT_WIDTH * 3) as usize;

    for roi in manual_rois {
        if let Some(clamped) = clamp_roi_to_frame(*roi, frame.cols(), frame.rows()) {
            let cropped = core::Mat::roi(frame, clamped).unwrap().try_clone().unwrap();
            let rec_input = prepare_ocr_input(&cropped).unwrap();
            let rec_cont = if rec_input.is_continuous() { rec_input } else { rec_input.clone() };
            rec_vstreams.write_input(rec_cont.data(), rec_input_size).unwrap();
            rec_vstreams
                .read_output(rec_output_buffer.as_mut_ptr(), rec_output_buffer.len())
                .unwrap();
            let text = decoder.decode(rec_output_buffer, 40, 97);
            if !text.is_empty() {
                annotations.push(OcrAnnotation { roi: clamped, text });
            }
        }
    }

    annotations
}
