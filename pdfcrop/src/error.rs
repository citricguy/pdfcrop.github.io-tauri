//! Error types for pdfcrop operations

use thiserror::Error;

/// Result type for pdfcrop operations
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur during PDF cropping operations
#[derive(Error, Debug)]
pub enum Error {
    /// Error reading or parsing PDF file
    #[error("PDF parsing error: {0}")]
    PdfParse(String),

    /// Error writing PDF file
    #[error("PDF writing error: {0}")]
    PdfWrite(String),

    /// Invalid page number or page not found
    #[error("Invalid page: {0}")]
    InvalidPage(String),

    /// Invalid bounding box coordinates
    #[error("Invalid bounding box: {0}")]
    InvalidBoundingBox(String),

    /// Error parsing PDF content stream
    #[error("Content stream parsing error: {0}")]
    ContentStreamParse(String),

    /// No content found on page (empty bbox)
    #[error("No content found on page {0}")]
    EmptyPage(usize),

    /// External tool error (e.g., Ghostscript)
    #[error("External tool error: {0}")]
    External(String),

    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// lopdf library error
    #[error("PDF library error: {0}")]
    Lopdf(#[from] lopdf::Error),
}
