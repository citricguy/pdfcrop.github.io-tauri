//! Ghostscript integration for accurate bounding box detection
//!
//! Uses Ghostscript's bbox device to render pages and detect ink bounds.
//! This is the standard method used by the original pdfcrop.

use crate::bbox::BoundingBox;
use crate::error::{Error, Result};
use std::process::Command;

/// Detect bounding box using Ghostscript's bbox device
///
/// This renders the page and tracks where ink appears, providing accurate
/// bounds including text with proper font metrics, images, etc.
pub fn detect_bbox_gs(pdf_data: &[u8], page_num: usize) -> Result<BoundingBox> {
    // Check if Ghostscript is available
    let gs_cmd = find_ghostscript()?;

    // Write PDF to temporary file (Ghostscript needs a file path)
    let temp_dir = std::env::temp_dir();
    let temp_pdf = temp_dir.join(format!("pdfcrop_temp_{}.pdf", std::process::id()));
    std::fs::write(&temp_pdf, pdf_data)
        .map_err(|e| Error::PdfParse(format!("failed to write temp PDF: {}", e)))?;

    // Ensure temp file is cleaned up
    let _cleanup = TempFileCleanup(&temp_pdf);

    // Run Ghostscript with bbox device
    let output = Command::new(&gs_cmd)
        .arg("-sDEVICE=bbox")
        .arg("-dBATCH")
        .arg("-dNOPAUSE")
        .arg("-dSAFER")
        .arg(format!("-dFirstPage={}", page_num + 1))
        .arg(format!("-dLastPage={}", page_num + 1))
        .arg("-q")
        .arg(&temp_pdf)
        .output()
        .map_err(|e| Error::External(format!("failed to run Ghostscript: {}", e)))?;

    if !output.status.success() {
        return Err(Error::External(format!(
            "Ghostscript exited with status: {}",
            output.status
        )));
    }

    // Ghostscript outputs bbox to stderr
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Parse bbox from output
    // Format: %%BoundingBox: left bottom right top
    // or:     %%HiResBoundingBox: left bottom right top
    parse_gs_bbox_output(&stderr)
}

/// Find Ghostscript executable
fn find_ghostscript() -> Result<String> {
    // Try common Ghostscript command names
    let candidates = vec!["gs", "gswin64c", "gswin32c", "gsc"];

    for cmd in candidates {
        if Command::new(cmd)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Ok(cmd.to_string());
        }
    }

    Err(Error::External(
        "Ghostscript not found. Please install Ghostscript:\n\
         - macOS: brew install ghostscript\n\
         - Ubuntu/Debian: sudo apt-get install ghostscript\n\
         - Windows: download from https://www.ghostscript.com/"
            .to_string(),
    ))
}

/// Parse Ghostscript bbox output
fn parse_gs_bbox_output(output: &str) -> Result<BoundingBox> {
    // Look for %%HiResBoundingBox first (more accurate)
    if let Some(line) = output.lines().find(|l| l.contains("%%HiResBoundingBox:")) {
        return parse_bbox_line(line, "%%HiResBoundingBox:");
    }

    // Fall back to %%BoundingBox
    if let Some(line) = output.lines().find(|l| l.contains("%%BoundingBox:")) {
        return parse_bbox_line(line, "%%BoundingBox:");
    }

    Err(Error::EmptyPage(0))
}

/// Parse a single bbox line
fn parse_bbox_line(line: &str, prefix: &str) -> Result<BoundingBox> {
    let bbox_str = line
        .strip_prefix(prefix)
        .ok_or_else(|| Error::InvalidBoundingBox("missing prefix".to_string()))?
        .trim();

    let parts: Vec<&str> = bbox_str.split_whitespace().collect();
    if parts.len() != 4 {
        return Err(Error::InvalidBoundingBox(format!(
            "expected 4 values, got {}",
            parts.len()
        )));
    }

    let left = parts[0]
        .parse::<f64>()
        .map_err(|e| Error::InvalidBoundingBox(format!("invalid left value: {}", e)))?;
    let bottom = parts[1]
        .parse::<f64>()
        .map_err(|e| Error::InvalidBoundingBox(format!("invalid bottom value: {}", e)))?;
    let right = parts[2]
        .parse::<f64>()
        .map_err(|e| Error::InvalidBoundingBox(format!("invalid right value: {}", e)))?;
    let top = parts[3]
        .parse::<f64>()
        .map_err(|e| Error::InvalidBoundingBox(format!("invalid top value: {}", e)))?;

    BoundingBox::new(left, bottom, right, top)
}

/// RAII cleanup for temporary file
struct TempFileCleanup<'a>(&'a std::path::Path);

impl Drop for TempFileCleanup<'_> {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hires_bbox() {
        let output = "%%HiResBoundingBox: 48.96 57.84 560.55 785.30";
        let bbox = parse_gs_bbox_output(output).unwrap();
        assert_eq!(bbox.left, 48.96);
        assert_eq!(bbox.bottom, 57.84);
        assert_eq!(bbox.right, 560.55);
        assert_eq!(bbox.top, 785.30);
    }

    #[test]
    fn test_parse_regular_bbox() {
        let output = "%%BoundingBox: 49 58 561 785";
        let bbox = parse_gs_bbox_output(output).unwrap();
        assert_eq!(bbox.left, 49.0);
        assert_eq!(bbox.bottom, 58.0);
        assert_eq!(bbox.right, 561.0);
        assert_eq!(bbox.top, 785.0);
    }

    #[test]
    fn test_hires_preferred_over_regular() {
        let output = "%%BoundingBox: 49 58 561 785\n%%HiResBoundingBox: 48.96 57.84 560.55 785.30";
        let bbox = parse_gs_bbox_output(output).unwrap();
        // Should use HiRes version
        assert_eq!(bbox.left, 48.96);
    }
}
