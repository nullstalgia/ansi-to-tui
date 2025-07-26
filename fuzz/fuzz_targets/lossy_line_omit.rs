#![no_main]

use ansi_to_tui::{IntoText, LossyFlavor};
use libfuzzer_sys::fuzz_target;
use tui::style::Style;

fuzz_target!(|data: &[u8]| {
    data.to_line_lossy(Style::new(), LossyFlavor::omitted())
        .unwrap();
});
