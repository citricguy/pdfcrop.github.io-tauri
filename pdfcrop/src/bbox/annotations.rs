//! PDF Annotation bounding box detection
//!
//! This module detects the bounding boxes of PDF annotations including:
//! - Links (/Link)
//! - Buttons and widgets (/Widget)
//! - Text annotations (/Text, /FreeText)
//! - Other interactive elements
//!
//! Annotations are stored separately from the page content stream but are
//! rendered as part of the page, so they must be included in bbox detection.

use crate::bbox::BoundingBox;
use crate::error::{Error, Result};
use lopdf::{Document, Object, ObjectId};

/// Detect bounding box of all annotations on a PDF page
///
/// Returns:
/// - `Ok(Some(bbox))` if annotations are found
/// - `Ok(None)` if no annotations exist on the page
/// - `Err(...)` if there's an error reading annotations
pub(crate) fn detect_annotation_bbox(
    doc: &Document,
    page: &lopdf::Dictionary,
    _page_id: ObjectId,
) -> Result<Option<BoundingBox>> {
    // Try to get the Annots array from the page
    let annots = match page.get(b"Annots") {
        Ok(obj) => obj,
        Err(_) => {
            // No annotations on this page
            return Ok(None);
        }
    };

    // Annots can be either a direct Array or a Reference to an Array
    let annots_array = match annots {
        Object::Array(arr) => arr,
        Object::Reference(ref_id) => {
            // Dereference to get the actual array
            match doc.get_object(*ref_id) {
                Ok(Object::Array(arr)) => arr,
                Ok(_) => return Err(Error::PdfParse("Annots reference is not an array".into())),
                Err(e) => {
                    return Err(Error::PdfParse(format!(
                        "Failed to dereference Annots: {}",
                        e
                    )))
                }
            }
        }
        _ => {
            return Err(Error::PdfParse(
                "Annots is not an array or reference".into(),
            ))
        }
    };

    if annots_array.is_empty() {
        return Ok(None);
    }

    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut found_any = false;

    // Process each annotation
    for annot_ref in annots_array {
        // Each annotation should be a reference to a dictionary
        let annot_id = match annot_ref {
            Object::Reference(id) => id,
            _ => continue, // Skip non-reference annotations
        };

        // Get the annotation dictionary
        let annot_dict = match doc.get_object(*annot_id) {
            Ok(Object::Dictionary(dict)) => dict,
            _ => continue, // Skip if not a dictionary
        };

        // Get the /Rect entry which defines the annotation's bounding box
        // Format: [x1 y1 x2 y2] where (x1,y1) is bottom-left, (x2,y2) is top-right
        let rect = match annot_dict.get(b"Rect") {
            Ok(Object::Array(arr)) => arr,
            Ok(Object::Reference(ref_id)) => match doc.get_object(*ref_id) {
                Ok(Object::Array(arr)) => arr,
                _ => continue,
            },
            _ => continue, // No Rect, skip this annotation
        };

        if rect.len() != 4 {
            continue; // Invalid Rect
        }

        // Extract coordinates from Rect array
        let x1 = extract_number(&rect[0]).unwrap_or(0.0);
        let y1 = extract_number(&rect[1]).unwrap_or(0.0);
        let x2 = extract_number(&rect[2]).unwrap_or(0.0);
        let y2 = extract_number(&rect[3]).unwrap_or(0.0);

        // Normalize rectangle (in case coordinates are reversed)
        let left = x1.min(x2);
        let right = x1.max(x2);
        let bottom = y1.min(y2);
        let top = y1.max(y2);

        // Update bounding box
        min_x = min_x.min(left);
        min_y = min_y.min(bottom);
        max_x = max_x.max(right);
        max_y = max_y.max(top);
        found_any = true;
    }

    if !found_any {
        return Ok(None);
    }

    // Create and return the bounding box
    BoundingBox::new(min_x, min_y, max_x, max_y).map(Some)
}

/// Extract a numeric value from a PDF Object
///
/// Handles both Integer and Real number types
fn extract_number(obj: &Object) -> Option<f64> {
    match obj {
        Object::Integer(i) => Some(*i as f64),
        Object::Real(f) => Some(*f as f64),
        _ => None,
    }
}

// Tests removed - this module is unused (legacy code kept for reference)
// The rendering-based detection approach doesn't require annotation detection
