use opencv::{ core::{ self, Mat, Point, Rect, Size, CV_8UC1 }, imgproc };
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct PostprocessConfig {
    pub threshold_value: f32,
    pub dilation_kernel_width: i32,
    pub dilation_kernel_height: i32,
    pub dilation_iterations: i32,
    pub min_roi_width: i32,
    pub min_roi_height: i32,
    pub min_roi_area: i32,
    pub ocr_frame_interval: i32,
    pub manual_roi_mode: bool,
}

impl Default for PostprocessConfig {
    fn default() -> Self {
        Self {
            threshold_value: 32.0,
            dilation_kernel_width: 4,
            dilation_kernel_height: 6,
            dilation_iterations: 1,
            min_roi_width: 8,
            min_roi_height: 8,
            min_roi_area: 100,
            ocr_frame_interval: 4,
            manual_roi_mode: false,
        }
    }
}

/// 將模型的輸出 Buffer (機率圖) 轉換成一組 OpenCV 的矩形框
pub fn extract_rois_from_heatmap(
    heatmap_buffer: &[u8],
    width: i32,
    height: i32,
    config: &PostprocessConfig,
) -> Vec<Rect> {
    let mut rois = Vec::new();

    // ==========================================
    // 1. 將一維 Buffer 轉為 OpenCV 二維矩陣 (Zero-copy)
    // ==========================================
    // CV_8UC1 代表單通道、8-bit 的灰階圖。
    // 使用 unsafe 是因為我們直接拿外部記憶體指標來建立 Mat，避免浪費時間複製。
    let heatmap_mat = unsafe {
        Mat::new_rows_cols_with_data_unsafe(
            height,
            width,
            CV_8UC1,
            heatmap_buffer.as_ptr() as *mut std::ffi::c_void,
            core::Mat_AUTO_STEP
        ).expect("無法建立熱力圖 Mat")
    };

    // ==========================================
    // 2. 二值化 (Binarization)
    // ==========================================
    // 將機率高於閾值（32）的像素變成全白 (255)，其餘變全黑 (0)
    let mut binary_mat = Mat::default();
    imgproc
        ::threshold(
            &heatmap_mat,
            &mut binary_mat,
            config.threshold_value as f64,
            255.0,
            imgproc::THRESH_BINARY,
        )
        .unwrap();

    // ==========================================
    // 3. 膨脹 (Dilation) - 關鍵步驟！
    // ==========================================
    // 模型預測出來的文字區塊有時候會斷裂 (例如 'i' 的上面那一點)。
    // 透過膨脹操作，把距離很近的白色像素「融合」成一個完整的大區塊。
    let mut dilated_mat = Mat::default();
    let kernel_width = config.dilation_kernel_width.max(1);
    let kernel_height = config.dilation_kernel_height.max(1);
    let dilation_iterations = config.dilation_iterations.max(1);
    let element = imgproc
        ::get_structuring_element(
            imgproc::MORPH_RECT,
            Size::new(kernel_width, kernel_height),
            Point::new(-1, -1)
        )
        .unwrap();

    imgproc
        ::dilate(
            &binary_mat,
            &mut dilated_mat,
            &element,
            Point::new(-1, -1),
            dilation_iterations,
            core::BORDER_CONSTANT,
            core::Scalar::default()
        )
        .unwrap();

    // ==========================================
    // 4. 尋找輪廓 (Find Contours)
    // ==========================================
    // 找出所有白色區塊的邊界
    let mut contours = core::Vector::<core::Vector<Point>>::new();
    imgproc
        ::find_contours(
            &dilated_mat,
            &mut contours,
            imgproc::RETR_EXTERNAL, // 我們只需要最外圍的輪廓，不需要中空的內部輪廓
            imgproc::CHAIN_APPROX_SIMPLE,
            Point::new(0, 0)
        )
        .unwrap();

    // ==========================================
    // 5. 轉換為 Bounding Box 並過濾雜訊
    // ==========================================
    for i in 0..contours.len() {
        let contour = contours.get(i).unwrap();
        let rect = imgproc::bounding_rect(&contour).unwrap();

        if rect.width >= config.min_roi_width
            && rect.height >= config.min_roi_height
            && rect.area() >= config.min_roi_area
        {
            rois.push(rect);
        }
    }

    rois
}
