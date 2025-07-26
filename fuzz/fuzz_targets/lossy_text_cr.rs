#![no_main]

use ansi_to_tui::{IntoText, LossyFlavor};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    data.to_text_lossy("\r", LossyFlavor::replacement_char())
        .unwrap();
});
