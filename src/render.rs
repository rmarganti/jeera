use crate::error::AppError;
use serde::Serialize;
use std::io::Write;

pub fn render_json(mut writer: impl Write, value: &impl Serialize) -> Result<(), AppError> {
    serde_json::to_writer_pretty(&mut writer, value)
        .map_err(|source| AppError::EncodeJsonOutput { source })?;
    writeln!(writer).map_err(|source| AppError::RenderOutput { source })?;
    Ok(())
}

pub mod ansi {
    pub const BOLD: &str = "\x1b[1m";
    pub const DIM: &str = "\x1b[2m";
    pub const CYAN: &str = "\x1b[36m";
    pub const GREEN: &str = "\x1b[32m";
    pub const YELLOW: &str = "\x1b[33m";
    pub const RESET: &str = "\x1b[0m";
}
