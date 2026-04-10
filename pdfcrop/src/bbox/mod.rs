//! Bounding box detection and manipulation
//!
//! This module provides functionality for detecting bounding boxes in PDF documents by:
//! - Parsing content streams to find graphics and text operations
//! - Detecting PDF annotations (links, buttons, etc.)
//! - Combining multiple sources to find the complete content bounds

#[allow(dead_code)]
mod annotations;
mod render;

use crate::error::{Error, Result};
use lopdf::Document;

pub use render::detect_bbox_by_rendering;

/// A bounding box in PDF coordinates (origin at bottom-left)
///
/// Coordinates are in points (1/72 inch)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundingBox {
    /// Left edge (x minimum)
    pub left: f64,
    /// Bottom edge (y minimum)
    pub bottom: f64,
    /// Right edge (x maximum)
    pub right: f64,
    /// Top edge (y maximum)
    pub top: f64,
}

impl BoundingBox {
    /// Create a new bounding box
    pub fn new(left: f64, bottom: f64, right: f64, top: f64) -> Result<Self> {
        if left >= right {
            return Err(Error::InvalidBoundingBox(format!(
                "left ({}) must be less than right ({})",
                left, right
            )));
        }
        if bottom >= top {
            return Err(Error::InvalidBoundingBox(format!(
                "bottom ({}) must be less than top ({})",
                bottom, top
            )));
        }

        Ok(Self {
            left,
            bottom,
            right,
            top,
        })
    }

    /// Parse bounding box from string specification
    ///
    /// Format: "left bottom right top"
    /// Example: "10 20 200 280"
    pub fn from_str(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.split_whitespace().collect();

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

        Self::new(left, bottom, right, top)
    }

    /// Get width of the bounding box
    pub fn width(&self) -> f64 {
        self.right - self.left
    }

    /// Get height of the bounding box
    pub fn height(&self) -> f64 {
        self.top - self.bottom
    }

    /// Expand bounding box by margins
    pub fn with_margins(&self, margins: &crate::margins::Margins) -> Self {
        Self {
            left: self.left - margins.left,
            bottom: self.bottom - margins.bottom,
            right: self.right + margins.right,
            top: self.top + margins.top,
        }
    }

    /// Ensure bounding box doesn't exceed page bounds
    pub fn clamp_to_page(&self, page_width: f64, page_height: f64) -> Self {
        Self {
            left: self.left.max(0.0),
            bottom: self.bottom.max(0.0),
            right: self.right.min(page_width),
            top: self.top.min(page_height),
        }
    }

    /// Union of two bounding boxes (combine to include both)
    pub fn union(&self, other: &BoundingBox) -> Self {
        Self {
            left: self.left.min(other.left),
            bottom: self.bottom.min(other.bottom),
            right: self.right.max(other.right),
            top: self.top.max(other.top),
        }
    }
}

/// Detect bounding box of content on a PDF page
///
/// This function uses rendering-based detection:
/// 1. Renders the PDF page to a bitmap using hayro
/// 2. Scans the bitmap to find the bounding box of non-white pixels
/// 3. Converts pixel coordinates back to PDF points
///
/// This approach is simple, accurate, and handles all PDF features automatically
/// including annotations, transformed graphics, and complex content.
pub fn detect_bbox(doc: &mut Document, page_num: usize) -> Result<BoundingBox> {
    // Save the document to bytes for hayro
    let mut pdf_bytes = Vec::new();
    doc.save_to(&mut pdf_bytes)
        .map_err(|e| Error::PdfParse(format!("failed to serialize PDF: {}", e)))?;

    // Use rendering-based detection at 72 DPI (1:1 scale with PDF points)
    detect_bbox_by_rendering(&pdf_bytes, page_num, Some(72.0))
}

/// Get the MediaBox of a page
///
/// The MediaBox defines the boundaries of the physical medium on which
/// the page is to be printed. It's the largest possible page size.
#[allow(dead_code)]
pub(crate) fn get_media_box(page: &lopdf::Dictionary) -> Result<BoundingBox> {
    let media_box = page
        .get(b"MediaBox")
        .map_err(|e| Error::PdfParse(format!("MediaBox not found: {}", e)))?
        .as_array()
        .map_err(|e| Error::PdfParse(format!("MediaBox is not an array: {}", e)))?;

    if media_box.len() != 4 {
        return Err(Error::PdfParse(format!(
            "MediaBox has wrong length: {}",
            media_box.len()
        )));
    }

    // MediaBox values can be either Integer or Real
    let left = media_box[0]
        .as_f32()
        .map(|f| f as f64)
        .or_else(|_| media_box[0].as_i64().map(|i| i as f64))
        .map_err(|e| Error::PdfParse(format!("invalid MediaBox left: {}", e)))?;
    let bottom = media_box[1]
        .as_f32()
        .map(|f| f as f64)
        .or_else(|_| media_box[1].as_i64().map(|i| i as f64))
        .map_err(|e| Error::PdfParse(format!("invalid MediaBox bottom: {}", e)))?;
    let right = media_box[2]
        .as_f32()
        .map(|f| f as f64)
        .or_else(|_| media_box[2].as_i64().map(|i| i as f64))
        .map_err(|e| Error::PdfParse(format!("invalid MediaBox right: {}", e)))?;
    let top = media_box[3]
        .as_f32()
        .map(|f| f as f64)
        .or_else(|_| media_box[3].as_i64().map(|i| i as f64))
        .map_err(|e| Error::PdfParse(format!("invalid MediaBox top: {}", e)))?;

    BoundingBox::new(left, bottom, right, top)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bbox_new() {
        let bbox = BoundingBox::new(10.0, 20.0, 100.0, 200.0).unwrap();
        assert_eq!(bbox.left, 10.0);
        assert_eq!(bbox.bottom, 20.0);
        assert_eq!(bbox.right, 100.0);
        assert_eq!(bbox.top, 200.0);
    }

    #[test]
    fn test_bbox_invalid() {
        assert!(BoundingBox::new(100.0, 20.0, 10.0, 200.0).is_err());
        assert!(BoundingBox::new(10.0, 200.0, 100.0, 20.0).is_err());
    }

    #[test]
    fn test_bbox_dimensions() {
        let bbox = BoundingBox::new(10.0, 20.0, 110.0, 220.0).unwrap();
        assert_eq!(bbox.width(), 100.0);
        assert_eq!(bbox.height(), 200.0);
    }

    #[test]
    fn test_bbox_from_str() {
        let bbox = BoundingBox::from_str("10 20 100 200").unwrap();
        assert_eq!(bbox.left, 10.0);
        assert_eq!(bbox.bottom, 20.0);
        assert_eq!(bbox.right, 100.0);
        assert_eq!(bbox.top, 200.0);
    }

    #[test]
    fn test_bbox_with_margins() {
        use crate::margins::Margins;
        let bbox = BoundingBox::new(10.0, 20.0, 100.0, 200.0).unwrap();
        let margins = Margins::uniform(5.0);
        let expanded = bbox.with_margins(&margins);

        assert_eq!(expanded.left, 5.0);
        assert_eq!(expanded.bottom, 15.0);
        assert_eq!(expanded.right, 105.0);
        assert_eq!(expanded.top, 205.0);
    }

    #[test]
    fn test_bbox_union() {
        let bbox1 = BoundingBox::new(10.0, 20.0, 100.0, 200.0).unwrap();
        let bbox2 = BoundingBox::new(5.0, 30.0, 90.0, 210.0).unwrap();
        let union = bbox1.union(&bbox2);

        assert_eq!(union.left, 5.0); // min of 10 and 5
        assert_eq!(union.bottom, 20.0); // min of 20 and 30
        assert_eq!(union.right, 100.0); // max of 100 and 90
        assert_eq!(union.top, 210.0); // max of 200 and 210
    }
}
