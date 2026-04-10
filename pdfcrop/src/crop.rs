//! Main PDF cropping logic

use crate::bbox::{detect_bbox, BoundingBox};
use crate::error::{Error, Result};
use crate::pdf_ops::{apply_cropbox, get_page_count, get_page_dimensions};
use crate::CropOptions;
use lopdf::Document;

/// Crop a PDF file according to the specified options
///
/// This function:
/// 1. Loads the PDF from bytes
/// 2. For each page:
///    - Detects or uses the specified bounding box
///    - Applies margins
///    - Sets the CropBox
/// 3. Returns the cropped PDF as bytes
///
/// # Example
///
/// ```no_run
/// use pdfcrop::{crop_pdf, CropOptions, Margins};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let pdf_data = std::fs::read("input.pdf")?;
/// let options = CropOptions {
///     margins: Margins::uniform(10.0),
///     ..Default::default()
/// };
/// let cropped = crop_pdf(&pdf_data, options)?;
/// std::fs::write("output.pdf", cropped)?;
/// # Ok(())
/// # }
/// ```
pub fn crop_pdf(pdf_data: &[u8], options: CropOptions) -> Result<Vec<u8>> {
    // Debug logging at the very start
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsValue;
        web_sys::console::log_1(&JsValue::from_str("[DEBUG] crop_pdf called"));
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "[DEBUG] page_range: {:?}",
            options.page_range
        )));
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "[DEBUG] shrink_to_content: {}",
            options.shrink_to_content
        )));
    }

    // Load the PDF document
    let mut doc = Document::load_mem(pdf_data)?;

    let page_count = get_page_count(&doc);

    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsValue;
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "[DEBUG] Total pages in document: {}",
            page_count
        )));
    }

    // Determine which pages to process based on page_range option
    let pages_to_process: Vec<usize> = if let Some(ref range) = options.page_range {
        range.to_page_list(page_count)
    } else {
        (0..page_count).collect()
    };

    if options.verbose {
        if pages_to_process.len() == page_count {
            eprintln!("Processing all {} pages", page_count);
        } else {
            eprintln!(
                "Processing {} of {} pages",
                pages_to_process.len(),
                page_count
            );
        }
    }

    // Phase 1: Detect all bboxes in parallel (read-only operations on pdf_data)
    // This is the expensive part (rendering), so parallelizing gives major speedup
    #[cfg(feature = "parallel")]
    let bbox_results: Vec<_> = {
        use rayon::prelude::*;
        pages_to_process
            .par_iter() // Rayon parallel iterator
            .map(|&page_num| {
                (
                    page_num,
                    bbox_detection_task(pdf_data, &doc, page_num, &options),
                )
            })
            .collect::<Vec<_>>()
    };

    // Sequential fallback when parallel feature is disabled (e.g., WASM without wasm-bindgen-rayon)
    #[cfg(not(feature = "parallel"))]
    let bbox_results: Vec<_> = pages_to_process
        .iter()
        .map(|&page_num| {
            (
                page_num,
                bbox_detection_task(pdf_data, &doc, page_num, &options),
            )
        })
        .collect::<Vec<_>>();

    // Phase 2: Apply cropboxes sequentially (mutates document, must be sequential)
    for (page_num, bbox_result) in bbox_results.iter() {
        // Extract bbox from result (propagate errors)
        let (final_bbox, is_manual) = bbox_result.as_ref().map_err(|e| {
            Error::PdfParse(format!(
                "Failed to detect bbox for page {}: {}",
                page_num + 1,
                e
            ))
        })?;

        // Apply content filtering if enabled and bbox was manually specified
        // Now supports recursive Form XObject filtering
        let should_clip = options.clip_content && *is_manual;

        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::JsValue;
            if options.clip_content {
                if *is_manual {
                    web_sys::console::log_1(&JsValue::from_str(&format!(
                        "[DEBUG] Page {}: Applying content clipping (manual bbox)",
                        page_num + 1
                    )));
                } else {
                    web_sys::console::log_1(&JsValue::from_str(&format!(
                        "[DEBUG] Page {}: Skipping clipping (auto-detected bbox - already tight)",
                        page_num + 1
                    )));
                }
            }
        }

        if options.verbose && options.clip_content && !is_manual {
            eprintln!("  Skipping clipping (auto-detected bbox - fast track)");
        }

        // Apply the crop box (with optional content clipping)
        apply_cropbox(&mut doc, *page_num, final_bbox, should_clip)?;
    }

    // Phase 3: Remove pages that weren't in the page range (if page range was specified)
    if let Some(ref _range) = options.page_range {
        // Get all page numbers (0-indexed)
        let all_pages: Vec<usize> = (0..page_count).collect();

        // Find pages to remove (pages NOT in the range, 0-indexed)
        let pages_to_remove: Vec<u32> = all_pages
            .iter()
            .filter(|&&p| !pages_to_process.contains(&p))
            .map(|&p| (p + 1) as u32) // Convert to 1-indexed for lopdf
            .collect();

        if !pages_to_remove.is_empty() {
            // Always log for debugging (even without verbose)
            #[cfg(target_arch = "wasm32")]
            {
                use wasm_bindgen::JsValue;
                web_sys::console::log_1(&JsValue::from_str(&format!(
                    "[DEBUG] Removing {} pages not in range: {:?}",
                    pages_to_remove.len(),
                    pages_to_remove
                )));
                web_sys::console::log_1(&JsValue::from_str(&format!(
                    "[DEBUG] Pages to keep (0-indexed): {:?}",
                    pages_to_process
                )));
                web_sys::console::log_1(&JsValue::from_str(&format!(
                    "[DEBUG] Page count before deletion: {}",
                    page_count
                )));
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                eprintln!(
                    "[DEBUG] Removing {} pages not in range: {:?}",
                    pages_to_remove.len(),
                    pages_to_remove
                );
                eprintln!("[DEBUG] Pages to keep (0-indexed): {:?}", pages_to_process);
                eprintln!("[DEBUG] Page count before deletion: {}", page_count);
            }

            // Delete all pages not in range in one call (uses 1-indexed page numbers)
            doc.delete_pages(&pages_to_remove);

            let new_count = get_page_count(&doc);

            #[cfg(target_arch = "wasm32")]
            {
                use wasm_bindgen::JsValue;
                web_sys::console::log_1(&JsValue::from_str(&format!(
                    "[DEBUG] Page count after deletion: {}",
                    new_count
                )));
                web_sys::console::log_1(&JsValue::from_str(
                    "[DEBUG] Cleaning up unused objects...",
                ));
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                eprintln!("[DEBUG] Page count after deletion: {}", new_count);
                eprintln!("[DEBUG] Cleaning up unused objects...");
            }

            // Clean up unused objects to reduce file size
            doc.delete_zero_length_streams(); // Remove empty streams
            doc.prune_objects(); // Remove unused objects
            doc.renumber_objects(); // Reorganize object IDs
            doc.compress(); // Compress stream objects

            #[cfg(target_arch = "wasm32")]
            {
                use wasm_bindgen::JsValue;
                web_sys::console::log_1(&JsValue::from_str("[DEBUG] Cleanup complete"));
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                eprintln!("[DEBUG] Cleanup complete");
            }
        }
    }

    // Save the document to bytes
    let mut output = Vec::new();
    doc.save_to(&mut output)?;

    Ok(output)
}

/// Bbox detection task (extracted for reuse in parallel and sequential modes)
fn bbox_detection_task(
    pdf_data: &[u8],
    doc: &Document,
    page_num: usize,
    options: &CropOptions,
) -> Result<(BoundingBox, bool)> {
    let page_count = get_page_count(doc);

    if options.verbose {
        eprintln!("Processing page {}/{}", page_num + 1, page_count);
    }

    // Determine which bounding box to use and whether it was manually specified
    // This uses pdf_data directly without re-serializing, and can run in parallel
    let (bbox, is_manual) = determine_bbox_with_source_parallel(
        pdf_data, doc, // Read-only access for page dimensions
        page_num, options,
    )?;

    if options.verbose {
        eprintln!(
            "  Detected bbox: ({:.2}, {:.2}, {:.2}, {:.2})",
            bbox.left, bbox.bottom, bbox.right, bbox.top
        );
        eprintln!("  Size: {:.2} x {:.2} pts", bbox.width(), bbox.height());
    }

    // Apply margins
    let bbox_with_margins = bbox.with_margins(&options.margins);

    if options.verbose {
        eprintln!(
            "  With margins: ({:.2}, {:.2}, {:.2}, {:.2})",
            bbox_with_margins.left,
            bbox_with_margins.bottom,
            bbox_with_margins.right,
            bbox_with_margins.top
        );
    }

    // Clamp to page dimensions (read-only operation)
    let (page_width, page_height) = get_page_dimensions(doc, page_num)?;
    let final_bbox = bbox_with_margins.clamp_to_page(page_width, page_height);

    if options.verbose {
        eprintln!(
            "  Final bbox: ({:.2}, {:.2}, {:.2}, {:.2})",
            final_bbox.left, final_bbox.bottom, final_bbox.right, final_bbox.top
        );
    }

    Ok((final_bbox, is_manual))
}

/// Determine which bounding box to use for a given page, and whether it was manually specified
/// (Parallel-safe version that uses pdf_data directly without re-serialization)
///
/// Returns (bbox, is_manual) where is_manual is true if the bbox came from a manual override
fn determine_bbox_with_source_parallel(
    pdf_data: &[u8],
    doc: &Document, // Read-only reference (thread-safe)
    page_num: usize,
    options: &CropOptions,
) -> Result<(BoundingBox, bool)> {
    // Check for per-page bbox override first (highest priority)
    if let Some(ref page_bboxes) = options.page_bboxes {
        if let Some(&bbox) = page_bboxes.get(&page_num) {
            #[cfg(target_arch = "wasm32")]
            {
                use wasm_bindgen::JsValue;
                web_sys::console::log_1(&JsValue::from_str(&format!(
                    "[DEBUG] Per-page bbox for page {}: ({:.2}, {:.2}, {:.2}, {:.2}), shrink_to_content: {}",
                    page_num + 1, bbox.left, bbox.bottom, bbox.right, bbox.top, options.shrink_to_content
                )));
            }

            if options.verbose {
                eprintln!("  Using per-page bbox for page {}", page_num + 1);
            }

            // Apply shrink_to_content if enabled for per-page bboxes too
            if options.shrink_to_content {
                #[cfg(target_arch = "wasm32")]
                {
                    use wasm_bindgen::JsValue;
                    web_sys::console::log_1(&JsValue::from_str(
                        "[DEBUG] Shrinking per-page bbox to actual content...",
                    ));
                }

                match detect_bbox_within_region(pdf_data, page_num, &bbox, options.verbose) {
                    Ok(detected_bbox) => {
                        #[cfg(target_arch = "wasm32")]
                        {
                            use wasm_bindgen::JsValue;
                            web_sys::console::log_1(&JsValue::from_str(&format!(
                                "[DEBUG] Shrunk to: ({:.2}, {:.2}, {:.2}, {:.2})",
                                detected_bbox.left,
                                detected_bbox.bottom,
                                detected_bbox.right,
                                detected_bbox.top
                            )));
                        }
                        // Still treat as manual bbox even after shrinking, so content clipping will be applied
                        return Ok((detected_bbox, true));
                    }
                    Err(_e) => {
                        #[cfg(target_arch = "wasm32")]
                        {
                            use wasm_bindgen::JsValue;
                            web_sys::console::log_1(&JsValue::from_str(&format!(
                                "[DEBUG] Could not shrink: {}, using manual bbox",
                                _e
                            )));
                        }
                        // Return manual bbox with is_manual=true
                        return Ok((bbox, true));
                    }
                }
            }

            return Ok((bbox, true));
        }
    }

    // Check for page-specific override (odd/even)
    let page_number = page_num + 1; // 1-indexed for odd/even check

    let manual_bbox = if page_number % 2 == 1 {
        // Odd page
        options.bbox_odd
    } else {
        // Even page
        options.bbox_even
    }
    .or(options.bbox_override); // Fall back to global override

    // If we have a manual bbox and shrink_to_content is enabled, detect content within it
    if let Some(bbox) = manual_bbox {
        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::JsValue;
            web_sys::console::log_1(&JsValue::from_str(&format!(
                "[DEBUG] Manual bbox for page {}: ({:.2}, {:.2}, {:.2}, {:.2}), shrink_to_content: {}",
                page_num + 1, bbox.left, bbox.bottom, bbox.right, bbox.top, options.shrink_to_content
            )));
        }

        if options.shrink_to_content {
            if options.verbose {
                eprintln!(
                    "  Manual bbox: ({:.2}, {:.2}, {:.2}, {:.2})",
                    bbox.left, bbox.bottom, bbox.right, bbox.top
                );
                eprintln!("  Detecting actual content within manual bbox...");
            }

            #[cfg(target_arch = "wasm32")]
            {
                use wasm_bindgen::JsValue;
                web_sys::console::log_1(&JsValue::from_str(
                    "[DEBUG] Shrinking manual bbox to actual content...",
                ));
            }

            // Detect content within the manual bbox region
            // Even though we shrink it, it's still based on a manual specification
            match detect_bbox_within_region(pdf_data, page_num, &bbox, options.verbose) {
                Ok(detected_bbox) => {
                    #[cfg(target_arch = "wasm32")]
                    {
                        use wasm_bindgen::JsValue;
                        web_sys::console::log_1(&JsValue::from_str(&format!(
                            "[DEBUG] Shrunk to: ({:.2}, {:.2}, {:.2}, {:.2})",
                            detected_bbox.left,
                            detected_bbox.bottom,
                            detected_bbox.right,
                            detected_bbox.top
                        )));
                    }

                    if options.verbose {
                        eprintln!(
                            "  Shrunk to actual content: ({:.2}, {:.2}, {:.2}, {:.2})",
                            detected_bbox.left,
                            detected_bbox.bottom,
                            detected_bbox.right,
                            detected_bbox.top
                        );
                    }
                    // Still treat as manual bbox even after shrinking, so content clipping will be applied
                    return Ok((detected_bbox, true));
                }
                Err(e) => {
                    #[cfg(target_arch = "wasm32")]
                    {
                        use wasm_bindgen::JsValue;
                        web_sys::console::log_1(&JsValue::from_str(&format!(
                            "[DEBUG] Could not shrink bbox: {}, using manual bbox",
                            e
                        )));
                    }

                    if options.verbose {
                        eprintln!("  Warning: Could not detect content within bbox ({}), using manual bbox", e);
                    }
                    // Return manual bbox with is_manual=true
                    return Ok((bbox, true));
                }
            }
        } else {
            #[cfg(target_arch = "wasm32")]
            {
                use wasm_bindgen::JsValue;
                web_sys::console::log_1(&JsValue::from_str(
                    "[DEBUG] Using manual bbox without shrinking",
                ));
            }

            // Manual bbox without shrinking - return with is_manual=true
            return Ok((bbox, true));
        }
    }

    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsValue;
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "[DEBUG] No manual bbox for page {}, auto-detecting content",
            page_num + 1
        )));
    }

    // Auto-detect bbox using specified method - use pdf_data directly for efficiency
    let bbox = detect_bbox_with_method_parallel(
        pdf_data,
        doc,
        page_num,
        options.bbox_method,
        options.verbose,
    )?;
    Ok((bbox, false))
}

/// Determine which bounding box to use for a given page, and whether it was manually specified
/// (Legacy version for compatibility - kept for reference)
///
/// Returns (bbox, is_manual) where is_manual is true if the bbox came from a manual override
#[allow(dead_code)]
fn determine_bbox_with_source(
    pdf_data: &[u8],
    doc: &mut Document,
    page_num: usize,
    options: &CropOptions,
) -> Result<(BoundingBox, bool)> {
    // Check for per-page bbox override first (highest priority)
    if let Some(ref page_bboxes) = options.page_bboxes {
        if let Some(&bbox) = page_bboxes.get(&page_num) {
            if options.verbose {
                eprintln!("  Using per-page bbox for page {}", page_num + 1);
            }
            return Ok((bbox, true));
        }
    }

    // Check for page-specific override (odd/even)
    let page_number = page_num + 1; // 1-indexed for odd/even check

    let manual_bbox = if page_number % 2 == 1 {
        // Odd page
        options.bbox_odd
    } else {
        // Even page
        options.bbox_even
    }
    .or(options.bbox_override); // Fall back to global override

    // If we have a manual bbox and shrink_to_content is enabled, detect content within it
    if let Some(bbox) = manual_bbox {
        if options.shrink_to_content {
            if options.verbose {
                eprintln!(
                    "  Manual bbox: ({:.2}, {:.2}, {:.2}, {:.2})",
                    bbox.left, bbox.bottom, bbox.right, bbox.top
                );
                eprintln!("  Detecting actual content within manual bbox...");
            }

            // Detect content within the manual bbox region
            // Even though we shrink it, it's still based on a manual specification
            match detect_bbox_within_region(pdf_data, page_num, &bbox, options.verbose) {
                Ok(detected_bbox) => {
                    if options.verbose {
                        eprintln!(
                            "  Shrunk to actual content: ({:.2}, {:.2}, {:.2}, {:.2})",
                            detected_bbox.left,
                            detected_bbox.bottom,
                            detected_bbox.right,
                            detected_bbox.top
                        );
                    }
                    // Return with is_manual=false since we detected actual content
                    return Ok((detected_bbox, false));
                }
                Err(e) => {
                    if options.verbose {
                        eprintln!("  Warning: Could not detect content within bbox ({}), using manual bbox", e);
                    }
                    // Return manual bbox with is_manual=true
                    return Ok((bbox, true));
                }
            }
        } else {
            // Manual bbox without shrinking - return with is_manual=true
            return Ok((bbox, true));
        }
    }

    // Auto-detect bbox using specified method - return with is_manual=false
    let bbox = detect_bbox_with_method(
        pdf_data,
        doc,
        page_num,
        options.bbox_method,
        options.verbose,
    )?;
    Ok((bbox, false))
}

/// Detect actual content bounding box within a specified region
///
/// This renders the page and detects content only within the given bbox region.
/// Useful for shrinking a manual bbox to the actual content.
fn detect_bbox_within_region(
    pdf_data: &[u8],
    page_num: usize,
    region: &BoundingBox,
    _verbose: bool,
) -> Result<BoundingBox> {
    use crate::error::Error;
    use hayro::{render, InterpreterSettings, Pdf as HayroPdf, RenderSettings};
    use std::sync::Arc;

    let scale = 1.0f64; // 72 DPI = 1:1 scale

    // Load PDF with hayro
    let data = Arc::new(pdf_data.to_vec());
    let pdf = HayroPdf::new(data)
        .map_err(|e| Error::PdfParse(format!("hayro failed to load PDF: {:?}", e)))?;

    let page = pdf
        .pages()
        .get(page_num)
        .ok_or_else(|| Error::InvalidPage(format!("page {} not found", page_num)))?;

    // Render the page
    let interpreter_settings = InterpreterSettings::default();
    let render_settings = RenderSettings {
        x_scale: scale as f32,
        y_scale: scale as f32,
        ..Default::default()
    };

    let pixmap = render(page, &interpreter_settings, &render_settings);

    // Get pixmap dimensions
    let width = pixmap.width() as usize;
    let height = pixmap.height() as usize;
    let pixels = pixmap.data_as_u8_slice();

    // Convert region bbox to pixel coordinates (PDF coords -> pixmap coords)
    let region_left_px = (region.left * scale).max(0.0) as usize;
    let region_right_px = ((region.right * scale).ceil() as usize).min(width);
    let pdf_height = (height as f64) / scale;
    let region_bottom_px = ((pdf_height - region.top) * scale).max(0.0) as usize;
    let region_top_px = (((pdf_height - region.bottom) * scale).ceil() as usize).min(height);

    // Scan only within the region for content
    let mut min_x = region_right_px;
    let mut max_x = region_left_px;
    let mut min_y = region_top_px;
    let mut max_y = region_bottom_px;

    const WHITE_THRESHOLD: u8 = 250;

    for y in region_bottom_px..region_top_px {
        for x in region_left_px..region_right_px {
            let idx = (y * width + x) * 4; // RGBA

            if idx + 2 < pixels.len() {
                let r = pixels[idx];
                let g = pixels[idx + 1];
                let b = pixels[idx + 2];

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
        // No content found in region, return the region itself
        return Ok(*region);
    }

    // Convert pixel coordinates back to PDF points
    let left = (min_x as f64) / scale;
    let right = (max_x as f64 + 1.0) / scale;
    let bottom = pdf_height - ((max_y as f64 + 1.0) / scale);
    let top = pdf_height - ((min_y as f64) / scale);

    BoundingBox::new(left, bottom, right, top)
}

/// Detect bounding box using the specified method (parallel-safe version using pdf_data directly)
fn detect_bbox_with_method_parallel(
    pdf_data: &[u8],
    _doc: &Document, // Not used, but kept for potential future use
    page_num: usize,
    method: crate::BBoxMethod,
    verbose: bool,
) -> Result<BoundingBox> {
    use crate::bbox::detect_bbox_by_rendering;
    use crate::BBoxMethod;

    match method {
        #[cfg(not(target_arch = "wasm32"))]
        BBoxMethod::Ghostscript => crate::ghostscript::detect_bbox_gs(pdf_data, page_num),
        #[cfg(target_arch = "wasm32")]
        BBoxMethod::Ghostscript => {
            // Ghostscript not available in WASM, fall back to rendering
            if verbose {
                eprintln!("  Ghostscript not available in WASM, using rendering-based detection");
            }
            detect_bbox_by_rendering(pdf_data, page_num, Some(72.0))
        }
        BBoxMethod::ContentStream => {
            // Use rendering-based detection directly on pdf_data (no re-serialization needed!)
            detect_bbox_by_rendering(pdf_data, page_num, Some(72.0))
        }
        #[cfg(not(target_arch = "wasm32"))]
        BBoxMethod::Auto => {
            // Try Ghostscript first
            match crate::ghostscript::detect_bbox_gs(pdf_data, page_num) {
                Ok(bbox) => {
                    if verbose {
                        eprintln!("  BBox method: Ghostscript");
                    }
                    Ok(bbox)
                }
                Err(e) => {
                    if verbose {
                        eprintln!(
                            "  Ghostscript unavailable ({}), using rendering-based detection",
                            e
                        );
                    }
                    // Use rendering-based detection directly on pdf_data
                    detect_bbox_by_rendering(pdf_data, page_num, Some(72.0))
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        BBoxMethod::Auto => {
            // In WASM, Auto just uses rendering (no Ghostscript available)
            detect_bbox_by_rendering(pdf_data, page_num, Some(72.0))
        }
    }
}

/// Detect bounding box using the specified method (legacy version for compatibility)
#[allow(dead_code)]
#[allow(unused_variables)] // pdf_data unused in WASM builds (no Ghostscript)
fn detect_bbox_with_method(
    pdf_data: &[u8],
    doc: &mut Document,
    page_num: usize,
    method: crate::BBoxMethod,
    verbose: bool,
) -> Result<BoundingBox> {
    use crate::BBoxMethod;

    match method {
        #[cfg(not(target_arch = "wasm32"))]
        BBoxMethod::Ghostscript => crate::ghostscript::detect_bbox_gs(pdf_data, page_num),
        #[cfg(target_arch = "wasm32")]
        BBoxMethod::Ghostscript => {
            // Ghostscript not available in WASM
            if verbose {
                eprintln!("  Ghostscript not available in WASM, using content stream parsing");
            }
            detect_bbox(doc, page_num)
        }
        BBoxMethod::ContentStream => detect_bbox(doc, page_num),
        #[cfg(not(target_arch = "wasm32"))]
        BBoxMethod::Auto => {
            // Try Ghostscript first
            match crate::ghostscript::detect_bbox_gs(pdf_data, page_num) {
                Ok(bbox) => {
                    if verbose {
                        eprintln!("  BBox method: Ghostscript");
                    }
                    Ok(bbox)
                }
                Err(e) => {
                    if verbose {
                        eprintln!(
                            "  Ghostscript unavailable ({}), using content stream parsing",
                            e
                        );
                    }
                    detect_bbox(doc, page_num)
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        BBoxMethod::Auto => {
            // In WASM, Auto just uses content stream
            detect_bbox(doc, page_num)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::margins::Margins;

    // Note: These tests would require actual PDF files to test properly.
    // For now, we'll just test that the API compiles and basic types work.

    #[test]
    fn test_crop_options_default() {
        let options = CropOptions::default();
        assert_eq!(options.margins, Margins::none());
        assert!(options.bbox_override.is_none());
        assert!(!options.verbose);
    }

    #[test]
    fn test_crop_options_with_margins() {
        let options = CropOptions {
            margins: Margins::uniform(10.0),
            ..Default::default()
        };
        assert_eq!(options.margins.left, 10.0);
    }
}
