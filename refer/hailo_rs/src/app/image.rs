use opencv::{
    core::{ self, Mat, Point, Rect, Scalar, Size, Vector, CV_8UC3 },
    imgcodecs,
    imgproc,
    prelude::*,
};
use tokio::sync::watch;

use crate::hw::{ mpp_jpeg::{ MppJpegEncoder, MppJpegFormat }, rga };
use crate::infer::PostprocessConfig;

use super::state::{ DEFAULT_FRAME_HEIGHT, DEFAULT_FRAME_WIDTH };
use super::types::OcrAnnotation;

pub const DET_INPUT_WIDTH: i32 = 960;
pub const DET_INPUT_HEIGHT: i32 = 544;
pub const OCR_INPUT_WIDTH: i32 = 320;
pub const OCR_INPUT_HEIGHT: i32 = 48;

// ── Frame publishing ──────────────────────────────────────────────────────────

pub fn publish_preview_frame(
    frame: &Mat,
    frame_tx: &watch::Sender<Vec<u8>>,
    mpp_enc: Option<&MppJpegEncoder>
) -> opencv::Result<()> {
    let jpeg = if let Some(enc) = mpp_enc {
        let w = frame.cols();
        let h = frame.rows();
        let bgr_bytes = (w * h * 3) as usize;
        if frame.is_continuous() && enc.width() == w && enc.height() == h {
            let mut yuv = vec![0u8; enc.required_input_len()];
            let dst_fmt = match enc.input_format() {
                MppJpegFormat::Nv12 => rga::fmt::YCB_CR_420_SP,
                MppJpegFormat::Nv24 => rga::fmt::YCB_CR_444_SP,
            };
            let src_slice = unsafe {
                std::slice::from_raw_parts_mut(frame.data() as *mut u8, bgr_bytes)
            };
            let ok = rga::rga_cvt_resize(
                src_slice,
                w,
                h,
                rga::fmt::BGR_888,
                &mut yuv,
                w,
                h,
                dst_fmt
            );
            if ok.is_ok() {
                if let Some(jpeg) = enc.encode(&yuv) { jpeg } else { encode_jpeg(frame)? }
            } else {
                encode_jpeg(frame)?
            }
        } else {
            encode_jpeg(frame)?
        }
    } else {
        encode_jpeg(frame)?
    };
    frame_tx.send_replace(jpeg);
    Ok(())
}

/// OpenCV CPU JPEG 編碼（作為 fallback 或首幀登畫面用）。
pub fn encode_jpeg(frame: &Mat) -> opencv::Result<Vec<u8>> {
    let mut buffer = Vector::<u8>::new();
    let params = Vector::<i32>::from_iter([imgcodecs::IMWRITE_JPEG_QUALITY, 82]);
    imgcodecs::imencode(".jpg", frame, &mut buffer, &params)?;
    Ok(buffer.to_vec())
}

pub fn build_bootstrap_frame() -> opencv::Result<Mat> {
    let mut frame = Mat::new_rows_cols_with_default(
        DEFAULT_FRAME_HEIGHT,
        DEFAULT_FRAME_WIDTH,
        CV_8UC3,
        Scalar::new(10.0, 18.0, 28.0, 0.0)
    )?;
    let _ = imgproc::put_text(
        &mut frame,
        "Hyper Mixed Web API",
        Point::new(86, 210),
        imgproc::FONT_HERSHEY_SIMPLEX,
        1.6,
        Scalar::new(255.0, 255.0, 255.0, 0.0),
        3,
        imgproc::LINE_AA,
        false
    );
    let _ = imgproc::put_text(
        &mut frame,
        "Waiting for camera and OCR pipeline...",
        Point::new(88, 286),
        imgproc::FONT_HERSHEY_SIMPLEX,
        0.9,
        Scalar::new(144.0, 224.0, 230.0, 0.0),
        2,
        imgproc::LINE_AA,
        false
    );
    Ok(frame)
}

// ── Annotation drawing ────────────────────────────────────────────────────────

pub fn draw_annotations(frame: &mut Mat, annotations: &[OcrAnnotation]) {
    for a in annotations {
        draw_result(frame, a.roi, &a.text);
    }
}

pub fn draw_manual_rois(frame: &mut Mat, rois: &[Rect]) {
    let color = Scalar::new(255.0, 255.0, 0.0, 0.0);
    for roi in rois {
        imgproc::rectangle(frame, *roi, color, 2, imgproc::LINE_8, 0).unwrap();
    }
}

pub fn draw_result(frame: &mut Mat, roi: Rect, text: &str) {
    if text.is_empty() {
        return;
    }
    let color_box = Scalar::new(0.0, 255.0, 0.0, 0.0);
    imgproc::rectangle(frame, roi, color_box, 2, imgproc::LINE_8, 0).unwrap();

    let color_text = Scalar::new(0.0, 0.0, 255.0, 0.0);
    let text_origin = Point::new(roi.x, std::cmp::max(roi.y - 5, 10));
    imgproc
        ::put_text(
            frame,
            text,
            text_origin,
            imgproc::FONT_HERSHEY_SIMPLEX,
            0.8,
            color_text,
            2,
            imgproc::LINE_8,
            false
        )
        .unwrap();
}

// ── ROI geometry helpers ──────────────────────────────────────────────────────

pub fn clamp_roi_to_frame(roi: Rect, frame_width: i32, frame_height: i32) -> Option<Rect> {
    if frame_width <= 0 || frame_height <= 0 {
        return None;
    }
    let x = roi.x.clamp(0, frame_width - 1);
    let y = roi.y.clamp(0, frame_height - 1);
    let width = roi.width.max(1).min(frame_width - x);
    let height = roi.height.max(1).min(frame_height - y);
    if width <= 0 || height <= 0 {
        return None;
    }
    Some(Rect::new(x, y, width, height))
}

pub fn build_tiles(frame_width: i32, frame_height: i32) -> Vec<Rect> {
    let mut tiles = Vec::new();
    let mut y = 0;
    while y < frame_height {
        let h = DET_INPUT_HEIGHT.min(frame_height - y);
        let mut x = 0;
        while x < frame_width {
            let w = DET_INPUT_WIDTH.min(frame_width - x);
            tiles.push(Rect::new(x, y, w, h));
            x += DET_INPUT_WIDTH;
        }
        y += DET_INPUT_HEIGHT;
    }
    tiles
}

pub fn map_roi_to_source(roi: Rect, tile: Rect, source_width: i32, source_height: i32) -> Rect {
    if source_width <= 0 || source_height <= 0 {
        return Rect::new(0, 0, 0, 0);
    }
    let scale_x = (tile.width as f64) / (DET_INPUT_WIDTH as f64);
    let scale_y = (tile.height as f64) / (DET_INPUT_HEIGHT as f64);

    let x = (tile.x + (((roi.x as f64) * scale_x).round() as i32)).clamp(0, source_width - 1);
    let y = (tile.y + (((roi.y as f64) * scale_y).round() as i32)).clamp(0, source_height - 1);
    let width = (((roi.width as f64) * scale_x).round() as i32).max(1).min(source_width - x);
    let height = (((roi.height as f64) * scale_y).round() as i32).max(1).min(source_height - y);

    Rect::new(x, y, width, height)
}

pub fn prepare_ocr_input(cropped: &Mat) -> opencv::Result<Mat> {
    let crop_width = cropped.cols();
    let crop_height = cropped.rows();
    if crop_width <= 0 || crop_height <= 0 {
        return Err(opencv::Error::new(core::StsBadArg, "OCR crop is empty".to_owned()));
    }

    let resized_width = (
        (((crop_width as f64) * (OCR_INPUT_HEIGHT as f64)) / (crop_height as f64)).round() as i32
    ).clamp(1, OCR_INPUT_WIDTH);

    // 嘗試用 RGA3 在單一 pass 完成縮放 + BGR→RGB。
    // 若 RGA 失敗（非 aarch64 或 driver 錯誤）則 fallback 至 OpenCV。
    if cropped.is_continuous() {
        let src_bytes = (crop_width * crop_height * 3) as usize;
        let dst_bytes = (OCR_INPUT_WIDTH * OCR_INPUT_HEIGHT * 3) as usize;
        let mut src_buf = unsafe {
            std::slice::from_raw_parts_mut(cropped.data() as *mut u8, src_bytes)
        };
        let mut dst_vec = vec![0u8; dst_bytes];
        let ok = rga::rga_cvt_resize(
            &mut src_buf,
            crop_width,
            crop_height,
            rga::fmt::BGR_888,
            &mut dst_vec,
            resized_width,
            OCR_INPUT_HEIGHT,
            rga::fmt::RGB_888
        );
        if ok.is_ok() {
            // RGA 輸出已經是 RGB，將結果包裝成 320×48×3 Mat（延續用點 zero-pad目的）。
            // 實際寬度為 resized_width，導層只讀 [0, resized_width) 行。
            let mat = (unsafe {
                Mat::new_rows_cols_with_data_unsafe(
                    OCR_INPUT_HEIGHT,
                    OCR_INPUT_WIDTH,
                    opencv::core::CV_8UC3,
                    dst_vec.as_ptr() as *mut _,
                    opencv::core::Mat_AUTO_STEP as usize
                )
            })?;
            let mut out = Mat::default();
            mat.copy_to(&mut out)?;
            return Ok(out);
        }
    }

    // ── OpenCV fallback ───────────────────────────────────────────────────
    let mut resized = Mat::default();
    imgproc::resize(
        cropped,
        &mut resized,
        Size::new(resized_width, OCR_INPUT_HEIGHT),
        0.0,
        0.0,
        imgproc::INTER_LINEAR
    )?;
    let mut padded = Mat::new_rows_cols_with_default(
        OCR_INPUT_HEIGHT,
        OCR_INPUT_WIDTH,
        resized.typ(),
        Scalar::all(0.0)
    )?;
    {
        let dst_roi = Rect::new(0, 0, resized_width, OCR_INPUT_HEIGHT);
        let mut dst = Mat::roi_mut(&mut padded, dst_roi)?;
        resized.copy_to(&mut dst)?;
    }
    let mut rgb = Mat::default();
    opencv::opencv_has_inherent_feature_algorithm_hint! {
        {
            imgproc::cvt_color(
                &padded, &mut rgb,
                imgproc::COLOR_BGR2RGB, 0,
                core::AlgorithmHint::ALGO_HINT_DEFAULT,
            )?;
        } else {
            imgproc::cvt_color(&padded, &mut rgb, imgproc::COLOR_BGR2RGB, 0)?;
        }
    }
    Ok(rgb)
}

// ── Mock animation helpers ────────────────────────────────────────────────────

pub fn draw_mock_background(frame: &mut Mat, frame_index: i32) {
    let stripe_offset = frame_index.rem_euclid(320);
    for stripe in 0..8_i32 {
        let x = (stripe * 180 + stripe_offset) % DEFAULT_FRAME_WIDTH;
        let color = Scalar::new(
            24.0 + (stripe as f64) * 8.0,
            56.0,
            76.0 + (stripe as f64) * 6.0,
            0.0
        );
        let _ = imgproc::rectangle(
            frame,
            Rect::new(x, 0, 92, DEFAULT_FRAME_HEIGHT),
            color,
            -1,
            imgproc::LINE_8,
            0
        );
    }
    for row in (0..DEFAULT_FRAME_HEIGHT).step_by(72) {
        let _ = imgproc::line(
            frame,
            Point::new(0, row),
            Point::new(DEFAULT_FRAME_WIDTH, row),
            Scalar::new(42.0, 86.0, 98.0, 0.0),
            1,
            imgproc::LINE_AA,
            0
        );
    }
}

pub fn build_mock_annotations(
    frame_index: i32,
    config: &PostprocessConfig,
    manual_rois: &[Rect]
) -> Vec<OcrAnnotation> {
    if !manual_rois.is_empty() {
        return manual_rois
            .iter()
            .enumerate()
            .map(|(i, roi)| OcrAnnotation {
                roi: *roi,
                text: format!("MANUAL-{:02}", i + 1),
            })
            .collect();
    }
    let sweep = frame_index.rem_euclid(220);
    vec![
        OcrAnnotation {
            roi: Rect::new(120 + sweep, 160, 280, 74),
            text: format!("OCR {:.0}", config.threshold_value),
        },
        OcrAnnotation {
            roi: Rect::new(540, 338 + sweep / 6, 320, 68),
            text: format!("ROI {}x{}", config.min_roi_width, config.min_roi_height),
        }
    ]
}
