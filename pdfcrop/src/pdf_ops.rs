//! PDF manipulation operations using lopdf

use crate::bbox::BoundingBox;
use crate::error::{Error, Result};
use lopdf::{Document, Object};

/// Apply a bounding box to a PDF page by setting its CropBox
///
/// The CropBox defines the region of the page to be displayed or printed.
/// This is the primary method for "cropping" a PDF page.
///
/// If `clip_content` is true, also adds a clipping path to the content stream
/// to actually remove/hide content outside the bbox.
pub fn apply_cropbox(
    doc: &mut Document,
    page_num: usize,
    bbox: &BoundingBox,
    clip_content: bool,
) -> Result<()> {
    // Get the page ID
    let page_id = doc
        .page_iter()
        .nth(page_num)
        .ok_or_else(|| Error::InvalidPage(format!("page {} not found", page_num)))?;

    // Get the page dictionary
    let page_dict = doc
        .get_object_mut(page_id)
        .map_err(|e| Error::PdfParse(format!("failed to get page {}: {}", page_num, e)))?
        .as_dict_mut()
        .map_err(|e| Error::PdfParse(format!("page {} is not a dictionary: {}", page_num, e)))?;

    // Create CropBox array: [left, bottom, right, top]
    let cropbox = Object::Array(vec![
        Object::Real(bbox.left as f32),
        Object::Real(bbox.bottom as f32),
        Object::Real(bbox.right as f32),
        Object::Real(bbox.top as f32),
    ]);

    // Set the CropBox
    page_dict.set("CropBox", cropbox);

    // If clip_content is enabled, filter page content using component-based approach
    // This removes paths and images that don't overlap with the crop box
    // Text blocks and Form XObjects are kept for safety
    if clip_content {
        filter_page_content(doc, page_id, page_num, bbox)?;
    }

    Ok(())
}

/// Filter page content to remove elements outside the crop box
///
/// This analyzes the page's content stream and removes drawing operations
/// that fall completely outside the crop box. This ensures clipped content
/// is actually removed from the PDF file for privacy/security.
fn filter_page_content(
    doc: &mut Document,
    page_id: (u32, u16),
    page_num: usize,
    bbox: &BoundingBox,
) -> Result<()> {
    use crate::content_filter::{
        filter_content_stream, filter_form_xobject, FormFilterTask, TextRenderFallback,
    };

    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsValue;
        web_sys::console::log_1(&JsValue::from_str("[DEBUG] Filtering page content..."));
    }

    // Get the page dictionary and clone needed data to avoid borrow conflicts
    let (contents_ref, resources) = {
        let page = doc
            .get_object(page_id)
            .map_err(|e| Error::PdfParse(format!("failed to get page: {}", e)))?
            .as_dict()
            .map_err(|e| Error::PdfParse(format!("page is not a dictionary: {}", e)))?;

        // Clone the page's Resources (needed for Form XObject lookup)
        let resources = page
            .get(b"Resources")
            .ok()
            .and_then(|obj| obj.as_dict().ok())
            .cloned();

        // Clone the Contents reference
        let contents_ref = match page.get(b"Contents") {
            Ok(obj) => obj.clone(),
            Err(_) => {
                // No existing content, nothing to filter
                return Ok(());
            }
        };

        (contents_ref, resources)
    };

    // Build a per-page render fallback using the current document bytes.
    let mut render_fallback = {
        let mut bytes = Vec::new();
        if let Err(e) = doc.save_to(&mut bytes) {
            #[cfg(not(target_arch = "wasm32"))]
            eprintln!("[DEBUG] Could not serialize PDF for render fallback: {}", e);
            None
        } else {
            TextRenderFallback::new(bytes, page_num).ok()
        }
    };

    // Collect all Form XObjects to filter
    let mut all_form_xobjects: Vec<FormFilterTask> = vec![];
    const IDENTITY_CTM: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

    // Handle both single stream and array of streams
    match contents_ref {
        Object::Reference(ref_id) => {
            // Single content stream - filter it
            let stream = doc
                .get_object(ref_id)
                .map_err(|e| Error::PdfParse(format!("failed to get stream: {}", e)))?
                .as_stream()
                .map_err(|e| Error::PdfParse(format!("object is not a stream: {}", e)))?;

            // Filter the content stream (collects Form XObjects for second pass)
            let (filtered_content, form_xobjects) = filter_content_stream(
                doc,
                stream,
                resources.as_ref(),
                bbox,
                &IDENTITY_CTM,
                &mut render_fallback,
                false,
            )?;
            all_form_xobjects.extend(form_xobjects);

            // Update the stream with filtered content
            let stream_mut = doc
                .get_object_mut(ref_id)
                .map_err(|e| Error::PdfParse(format!("failed to get stream mut: {}", e)))?
                .as_stream_mut()
                .map_err(|e| Error::PdfParse(format!("object is not a stream: {}", e)))?;

            stream_mut.set_plain_content(filtered_content);
        }
        Object::Array(ref streams) => {
            // Multiple content streams - MUST concatenate before filtering!
            // PDF spec says these streams are concatenated, and operations can span
            // stream boundaries (e.g., one stream ends with [(text) and next has ] TJ)
            #[cfg(target_arch = "wasm32")]
            {
                use wasm_bindgen::JsValue;
                web_sys::console::log_1(&JsValue::from_str(&format!(
                    "[DEBUG] Page has {} content streams - concatenating before filter",
                    streams.len()
                )));
            }

            #[cfg(debug_assertions)]
            eprintln!(
                "[DEBUG] Page has {} content streams (array) - concatenating",
                streams.len()
            );

            // Collect all stream references and concatenate their decoded content
            let mut stream_refs: Vec<lopdf::ObjectId> = Vec::new();
            let mut concatenated_bytes: Vec<u8> = Vec::new();

            for stream_ref in streams.iter() {
                if let Object::Reference(ref_id) = stream_ref {
                    stream_refs.push(*ref_id);

                    let stream = doc
                        .get_object(*ref_id)
                        .map_err(|e| Error::PdfParse(format!("failed to get stream: {}", e)))?
                        .as_stream()
                        .map_err(|e| Error::PdfParse(format!("object is not a stream: {}", e)))?;

                    let decoded = stream
                        .decompressed_content()
                        .map_err(|e| Error::PdfParse(format!("Failed to decode stream: {}", e)))?;

                    // Add newline separator between streams to ensure operators don't merge
                    if !concatenated_bytes.is_empty() {
                        concatenated_bytes.push(b'\n');
                    }
                    concatenated_bytes.extend_from_slice(&decoded);
                }
            }

            if stream_refs.is_empty() {
                return Err(Error::PdfParse("No valid stream references in array".to_string()));
            }

            // Create a temporary stream object for filtering
            // The bytes are already decoded, so we create an uncompressed stream
            let mut temp_dict = lopdf::Dictionary::new();
            temp_dict.set("Length", Object::Integer(concatenated_bytes.len() as i64));
            let mut temp_stream = lopdf::Stream::new(temp_dict, concatenated_bytes);
            // Mark as already decompressed by setting allows_compression = false
            temp_stream.allows_compression = false;

            // Filter the concatenated content
            let (filtered_content, form_xobjects) = filter_content_stream(
                doc,
                &temp_stream,
                resources.as_ref(),
                bbox,
                &IDENTITY_CTM,
                &mut render_fallback,
                false,
            )?;
            all_form_xobjects.extend(form_xobjects);

            // Put all filtered content in the first stream, clear others
            for (idx, ref_id) in stream_refs.iter().enumerate() {
                let stream_mut = doc
                    .get_object_mut(*ref_id)
                    .map_err(|e| Error::PdfParse(format!("failed to get stream mut: {}", e)))?
                    .as_stream_mut()
                    .map_err(|e| Error::PdfParse(format!("object is not a stream: {}", e)))?;

                if idx == 0 {
                    // First stream gets all the filtered content
                    stream_mut.set_plain_content(filtered_content.clone());
                } else {
                    // Other streams are cleared (they'll be empty but still valid)
                    stream_mut.set_plain_content(Vec::new());
                }
            }
        }
        _ => {
            return Err(Error::PdfParse(
                "Contents is not a reference or array".to_string(),
            ));
        }
    }

    // Second pass: Recursively filter all collected Form XObjects
    while let Some(task) = all_form_xobjects.pop() {
        match filter_form_xobject(doc, task, bbox, &mut render_fallback) {
            Ok(nested_xobjects) => {
                all_form_xobjects.extend(nested_xobjects);
            }
            Err(e) => {
                #[cfg(target_arch = "wasm32")]
                {
                    use wasm_bindgen::JsValue;
                    web_sys::console::log_1(&JsValue::from_str(&format!(
                        "[DEBUG] Could not filter Form XObject: {}",
                        e
                    )));
                }
                #[cfg(not(target_arch = "wasm32"))]
                eprintln!("[DEBUG] Could not filter Form XObject: {}", e);
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsValue;
        web_sys::console::log_1(&JsValue::from_str("[DEBUG] Content filtering complete"));
    }

    Ok(())
}

/// Get the MediaBox dimensions of a page
///
/// MediaBox defines the boundaries of the physical medium
pub fn get_page_dimensions(doc: &Document, page_num: usize) -> Result<(f64, f64)> {
    let page_id = doc
        .page_iter()
        .nth(page_num)
        .ok_or_else(|| Error::InvalidPage(format!("page {} not found", page_num)))?;

    let page = doc
        .get_object(page_id)
        .map_err(|e| Error::PdfParse(format!("failed to get page {}: {}", page_num, e)))?
        .as_dict()
        .map_err(|e| Error::PdfParse(format!("page {} is not a dictionary: {}", page_num, e)))?;

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

    let width = right - left;
    let height = top - bottom;

    Ok((width, height))
}

/// Get the number of pages in a PDF document
pub fn get_page_count(doc: &Document) -> usize {
    doc.get_pages().len()
}
