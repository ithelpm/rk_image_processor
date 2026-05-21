use std::fs;

pub struct PpocrDecoder {
    characters: Vec<char>,
    blank_idx: usize,
}

impl PpocrDecoder {
    /// 動態讀取字典檔，並自動補齊 PP-OCR 的隱藏字元
    #[allow(dead_code)]
    pub fn new(dict_path: &str) -> Result<Self, String> {
        // 1. 讀取 en_dict.txt
        let content = fs
            ::read_to_string(dict_path)
            .map_err(|e| format!("Unable to read dictionary file: {}", e))?;

        Self::from_content(&content)
    }

    /// 從已載入的字串內容建立解碼器（適合搭配 include_str! 將字典嵌入執行檔）
    pub fn from_content(content: &str) -> Result<Self, String> {
        let mut characters: Vec<char> = vec!['∅'];

        characters.extend(
            content
                .lines()
                .filter(|l| !l.is_empty())
                .map(|l| l.chars().next().unwrap())
        );

        // 2. Fill in extra classes that the model output might reserve
        characters.push('?'); // Extra class / unknown

        // CTC Blank is placed at index 0 according to PP-OCR convention
        let blank_idx = 0;

        Ok(Self { characters, blank_idx })
    }

    /// 執行 CTC 解碼
    pub fn decode(&self, rec_buffer: &[u8], time_steps: usize, num_classes: usize) -> String {
        // 1. Argmax：找出每個時間步長機率最大的 Index
        let mut raw_indices = Vec::with_capacity(time_steps);

        for t in 0..time_steps {
            let mut max_prob = 0u8;
            let mut max_idx = 0_usize;

            for c in 0..num_classes {
                // 模型輸出可能是 uint8，直接比較大小即可
                let prob = rec_buffer[t * num_classes + c];
                if prob > max_prob {
                    max_prob = prob;
                    max_idx = c;
                }
            }
            raw_indices.push(max_idx);
        }

        // 2. CTC 解碼邏輯 (移除連續重複字與 Blank)
        let mut final_text = String::new();
        let mut prev_idx = self.blank_idx;

        for &idx in &raw_indices {
            // 條件：不是 Blank 且 跟前一個不重複
            if idx != self.blank_idx && idx != prev_idx {
                // 安全防護：確保不會超出陣列
                if idx < self.characters.len() {
                    let ch = self.characters[idx];
                    if ch != '?' && ch != '∅' {
                        // 過濾掉用來 Padding 的字元
                        final_text.push(ch);
                    }
                }
            }
            prev_idx = idx;
        }

        final_text
    }
}

#[test]
fn test_decoder() {
    let decoder = PpocrDecoder::from_content(include_str!("../../resources/en_dict.txt")).unwrap();
    assert!(decoder.characters.len() == 97);
    assert!(decoder.blank_idx == 0);
}
