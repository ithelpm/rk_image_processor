pub mod hailo_sys {
    // 告訴 Rust 編譯器：不要對這包 C 語言風格的程式碼碎碎念
    #![allow(non_upper_case_globals)]
    #![allow(non_camel_case_types)]
    #![allow(non_snake_case)]
    #![allow(dead_code)] // 避免沒有用到的 C 函式一直跳警告

    // 把暫存區的綁定程式碼直接貼進這個模組裡
    include!(concat!(env!("OUT_DIR"), "/hailort_bindings.rs"));
}
