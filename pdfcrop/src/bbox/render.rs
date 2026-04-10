//! Render-based bounding box detection
//!
//! This module uses actual PDF rendering (via hayro) to detect content bounds
//! by finding non-white pixels in the rendered output. This approach:
//! - Eliminates all heuristics (no corner detection, edge thresholds, font filtering)
//! - Matches Ghostscript's rendering-based approach
//! - Automatically handles annotations, transformed graphics, and edge cases
//! - Simple, accurate, and handles all PDF features
//!
//! ## Approach
//! 1. Render the PDF page to a bitmap using hayro
//! 2. Scan pixels to find the bounding box of non-white content
//! 3. Convert pixel coordinates back to PDF points

use crate::bbox::BoundingBox;
use crate::error::{Error, Result};

use hayro::{render, InterpreterSettings, Pdf as HayroPdf, RenderSettings};
use std::sync::Arc;

/// Detect bounding box by rendering the page and finding non-white pixels
///
/// This function:
/// 1. Renders the PDF page at a given DPI (default: 72 DPI = 1:1 scale)
/// 2. Scans the rendered bitmap to find the bounding box of content
/// 3. Returns the bounding box in PDF coordinate space (points)
///
/// # Parameters
/// - `pdf_bytes`: The raw PDF file data
/// - `page_num`: Zero-indexed page number
/// - `dpi`: Dots per inch for rendering (higher = more accurate but slower)
///          Default is 72 DPI (1:1 scale with PDF points)
///
/// # Returns
/// The bounding box of non-white content in PDF points
pub fn detect_bbox_by_rendering(
    pdf_bytes: &[u8],
    page_num: usize,
    dpi: Option<f32>,
) -> Result<BoundingBox> {
    let dpi = dpi.unwrap_or(72.0); // Default: 72 DPI = 1:1 with PDF points
    let scale = dpi / 72.0; // PDF uses 72 points per inch

    // Load PDF with hayro
    let data = Arc::new(pdf_bytes.to_vec());
    let pdf = HayroPdf::new(data)
        .map_err(|e| Error::PdfParse(format!("hayro failed to load PDF: {:?}", e)))?;

    // Get the requested page
    let page = pdf
        .pages()
        .get(page_num)
        .ok_or_else(|| Error::InvalidPage(format!("page {} not found", page_num)))?;

    // Render the page
    let interpreter_settings = InterpreterSettings::default();
    let render_settings = RenderSettings {
        x_scale: scale,
        y_scale: scale,
        ..Default::default()
    };

    let pixmap = render(page, &interpreter_settings, &render_settings);

    // Scan the pixmap to find bounding box of non-white pixels
    let bbox = scan_pixmap_for_content(&pixmap, scale)?;

    Ok(bbox)
}

/// Scan a rendered pixmap to find the bounding box of non-white pixels
///
/// This function scans the pixel buffer to find the minimum and maximum
/// x/y coordinates of pixels that are not white (allowing for slight anti-aliasing).
///
/// # Parameters
/// - `pixmap`: The rendered page as RGBA pixels
/// - `scale`: The scale factor used for rendering (to convert back to PDF points)
///
/// # Returns
/// The bounding box in PDF coordinate space (bottom-left origin)
fn scan_pixmap_for_content(pixmap: &hayro::Pixmap, scale: f32) -> Result<BoundingBox> {
    let width = pixmap.width() as usize;
    let height = pixmap.height() as usize;
    let pixels = pixmap.data_as_u8_slice();

    // PDF uses bottom-left origin, but pixmaps use top-left
    // We'll scan in pixmap coordinates first, then convert

    let mut min_x = width;
    let mut max_x = 0;
    let mut min_y = height;
    let mut max_y = 0;

    // Threshold for "non-white" (allows for slight anti-aliasing)
    // RGB values below this are considered content
    const WHITE_THRESHOLD: u8 = 250;

    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) * 4; // RGBA = 4 bytes per pixel

            if idx + 2 < pixels.len() {
                let r = pixels[idx];
                let g = pixels[idx + 1];
                let b = pixels[idx + 2];
                // Alpha is pixels[idx + 3] but we don't check it

                // If pixel is not white (accounting for anti-aliasing)
                if r < WHITE_THRESHOLD || g < WHITE_THRESHOLD || b < WHITE_THRESHOLD {
                    min_x = min_x.min(x);
                    max_x = max_x.max(x);
                    min_y = min_y.min(y);
                    max_y = max_y.max(y);
                }
            }
        }
    }

    // Check if we found any content
    if min_x > max_x || min_y > max_y {
        return Err(Error::EmptyPage(0)); // No content found
    }

    // Convert pixmap coordinates to PDF points
    // Pixmap: top-left origin, y increases downward
    // PDF: bottom-left origin, y increases upward

    // Scale back from pixels to PDF points
    let left = (min_x as f32) / scale;
    let right = (max_x as f32 + 1.0) / scale; // +1 because max_x is inclusive

    // Flip y-axis: pixmap y=0 is PDF y=height
    let pdf_height = (height as f32) / scale;
    let bottom = pdf_height - ((max_y as f32 + 1.0) / scale);
    let top = pdf_height - ((min_y as f32) / scale);

    BoundingBox::new(left as f64, bottom as f64, right as f64, top as f64)
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_render_bbox_basic() {
        // Basic test would go here
        // Requires actual PDF test data
    }
}
