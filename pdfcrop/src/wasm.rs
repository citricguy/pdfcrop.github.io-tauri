//! WASM bindings for pdfcrop
//!
//! This module provides JavaScript-accessible functions for PDF cropping in web browsers.

use js_sys::{Array, Object};
use std::collections::HashMap;
use wasm_bindgen::prelude::*;

use crate::{crop_pdf, BBoxMethod, BoundingBox, CropOptions, Margins, PageRange};

/// Initialize panic hook for better error messages in browser console
#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
    // Only show logs from pdfcrop, not from dependencies like hayro (which spams thousands of debug logs)
    wasm_logger::init(
        wasm_logger::Config::default().module_prefix("pdfcrop"), // Only log from pdfcrop modules
    );
}

/// WASM-friendly bounding box structure
#[wasm_bindgen]
#[derive(Debug, Clone, Copy)]
pub struct WasmBoundingBox {
    pub left: f64,
    pub bottom: f64,
    pub right: f64,
    pub top: f64,
}

#[wasm_bindgen]
impl WasmBoundingBox {
    #[wasm_bindgen(constructor)]
    pub fn new(left: f64, bottom: f64, right: f64, top: f64) -> Self {
        WasmBoundingBox {
            left,
            bottom,
            right,
            top,
        }
    }

    #[wasm_bindgen(getter)]
    pub fn width(&self) -> f64 {
        self.right - self.left
    }

    #[wasm_bindgen(getter)]
    pub fn height(&self) -> f64 {
        self.top - self.bottom
    }
}

impl From<WasmBoundingBox> for BoundingBox {
    fn from(bbox: WasmBoundingBox) -> Self {
        BoundingBox::new(bbox.left, bbox.bottom, bbox.right, bbox.top)
            .expect("Invalid bounding box coordinates")
    }
}

impl From<BoundingBox> for WasmBoundingBox {
    fn from(bbox: BoundingBox) -> Self {
        WasmBoundingBox {
            left: bbox.left,
            bottom: bbox.bottom,
            right: bbox.right,
            top: bbox.top,
        }
    }
}

/// WASM-friendly crop options
#[wasm_bindgen]
pub struct WasmCropOptions {
    margins_left: f64,
    margins_top: f64,
    margins_right: f64,
    margins_bottom: f64,
    verbose: bool,
    clip_content: bool,
    shrink_to_content: bool,
}

#[wasm_bindgen]
impl WasmCropOptions {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        WasmCropOptions {
            margins_left: 0.0,
            margins_top: 0.0,
            margins_right: 0.0,
            margins_bottom: 0.0,
            verbose: false,
            clip_content: false,
            shrink_to_content: false,
        }
    }

    /// Set uniform margins (all sides)
    #[wasm_bindgen(js_name = setUniformMargins)]
    pub fn set_uniform_margins(&mut self, margin: f64) {
        self.margins_left = margin;
        self.margins_top = margin;
        self.margins_right = margin;
        self.margins_bottom = margin;
    }

    /// Set individual margins
    #[wasm_bindgen(js_name = setMargins)]
    pub fn set_margins(&mut self, left: f64, top: f64, right: f64, bottom: f64) {
        self.margins_left = left;
        self.margins_top = top;
        self.margins_right = right;
        self.margins_bottom = bottom;
    }

    #[wasm_bindgen(js_name = setVerbose)]
    pub fn set_verbose(&mut self, verbose: bool) {
        self.verbose = verbose;
    }

    #[wasm_bindgen(js_name = setClipContent)]
    pub fn set_clip_content(&mut self, clip: bool) {
        self.clip_content = clip;
    }

    #[wasm_bindgen(js_name = setShrinkToContent)]
    pub fn set_shrink_to_content(&mut self, shrink: bool) {
        self.shrink_to_content = shrink;
    }
}

/// Crop a PDF with the given options
///
/// # Parameters
/// - `pdf_bytes`: PDF file as Uint8Array
/// - `options`: WasmCropOptions configuration
/// - `page_bboxes`: Optional JS Map of page number → WasmBoundingBox
/// - `page_range`: Optional array of page numbers to crop (0-indexed)
///
/// # Returns
/// Cropped PDF as Uint8Array
#[wasm_bindgen(js_name = cropPdf)]
pub fn crop_pdf_wasm(
    pdf_bytes: &[u8],
    options: &WasmCropOptions,
    page_bboxes: Option<Object>,
    page_range: Option<Array>,
) -> Result<Vec<u8>, JsValue> {
    // Convert page_bboxes from JS Map to HashMap
    let page_bboxes_map = if let Some(obj) = page_bboxes {
        let mut map = HashMap::new();
        let entries = js_sys::Object::entries(&obj);
        for i in 0..entries.length() {
            let entry = entries.get(i);
            let pair = js_sys::Array::from(&entry);
            // Parse page number from string key (Object keys are always strings in JS)
            let page_num_str = pair.get(0).as_string().ok_or("Invalid page number")?;
            let page_num = page_num_str
                .parse::<usize>()
                .map_err(|_| JsValue::from_str("Invalid page number"))?;
            let bbox_obj = pair.get(1);

            // Extract bbox fields from JS object
            let left = js_sys::Reflect::get(&bbox_obj, &"left".into())?
                .as_f64()
                .ok_or("Invalid bbox.left")?;
            let bottom = js_sys::Reflect::get(&bbox_obj, &"bottom".into())?
                .as_f64()
                .ok_or("Invalid bbox.bottom")?;
            let right = js_sys::Reflect::get(&bbox_obj, &"right".into())?
                .as_f64()
                .ok_or("Invalid bbox.right")?;
            let top = js_sys::Reflect::get(&bbox_obj, &"top".into())?
                .as_f64()
                .ok_or("Invalid bbox.top")?;

            let bbox = BoundingBox::new(left, bottom, right, top)
                .map_err(|e| JsValue::from_str(&e.to_string()))?;
            map.insert(page_num, bbox);
        }
        Some(map)
    } else {
        None
    };

    // Convert page_range from JS Array to Vec<usize>
    let page_range_opt = if let Some(arr) = page_range {
        let mut pages = Vec::new();
        for i in 0..arr.length() {
            let page = arr.get(i).as_f64().ok_or("Invalid page number in range")? as usize;
            pages.push(page);
        }
        Some(PageRange::List(pages))
    } else {
        None
    };

    // Build CropOptions
    let crop_options = CropOptions {
        margins: Margins::new(
            options.margins_left,
            options.margins_top,
            options.margins_right,
            options.margins_bottom,
        ),
        bbox_override: None,
        bbox_odd: None,
        bbox_even: None,
        page_bboxes: page_bboxes_map,
        page_range: page_range_opt,
        bbox_method: BBoxMethod::ContentStream, // Always use rendering in WASM
        verbose: options.verbose,
        clip_content: options.clip_content,
        shrink_to_content: options.shrink_to_content,
    };

    // Perform the crop
    crop_pdf(pdf_bytes, crop_options).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Get the number of pages in a PDF
#[wasm_bindgen(js_name = getPageCount)]
pub fn get_page_count_wasm(pdf_bytes: &[u8]) -> Result<usize, JsValue> {
    use lopdf::Document;

    let doc = Document::load_mem(pdf_bytes)
        .map_err(|e| JsValue::from_str(&format!("Failed to load PDF: {}", e)))?;

    Ok(crate::pdf_ops::get_page_count(&doc))
}

/// Auto-detect bounding box for a specific page
///
/// # Parameters
/// - `pdf_bytes`: PDF file as Uint8Array
/// - `page_num`: Page number (0-indexed)
///
/// # Returns
/// WasmBoundingBox with detected bounds
#[wasm_bindgen(js_name = detectBbox)]
pub fn detect_bbox_wasm(pdf_bytes: &[u8], page_num: usize) -> Result<WasmBoundingBox, JsValue> {
    use crate::bbox::detect_bbox_by_rendering;

    let bbox = detect_bbox_by_rendering(pdf_bytes, page_num, Some(72.0))
        .map_err(|e| JsValue::from_str(&e.to_string()))?;

    Ok(bbox.into())
}

/// Get page dimensions (MediaBox)
///
/// # Parameters
/// - `pdf_bytes`: PDF file as Uint8Array
/// - `page_num`: Page number (0-indexed)
///
/// # Returns
/// Object with {width, height} in PDF points
#[wasm_bindgen(js_name = getPageDimensions)]
pub fn get_page_dimensions_wasm(pdf_bytes: &[u8], page_num: usize) -> Result<JsValue, JsValue> {
    use lopdf::Document;

    let doc = Document::load_mem(pdf_bytes)
        .map_err(|e| JsValue::from_str(&format!("Failed to load PDF: {}", e)))?;

    let (width, height) = crate::pdf_ops::get_page_dimensions(&doc, page_num)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;

    let obj = Object::new();
    js_sys::Reflect::set(&obj, &"width".into(), &JsValue::from_f64(width))?;
    js_sys::Reflect::set(&obj, &"height".into(), &JsValue::from_f64(height))?;
    Ok(obj.into())
}

/// Render a PDF page to PNG for preview in canvas
///
/// # Parameters
/// - `pdf_bytes`: PDF file as Uint8Array
/// - `page_num`: Page number (0-indexed)
/// - `dpi`: Optional DPI (default: 72)
///
/// # Returns
/// PNG image data as Uint8Array
#[wasm_bindgen(js_name = renderPageToPng)]
pub fn render_page_to_png_wasm(
    _pdf_bytes: &[u8],
    _page_num: usize,
    _dpi: Option<f32>,
) -> Result<Vec<u8>, JsValue> {
    // This will be implemented after we add the render_page_to_png function
    // to the main crate
    Err(JsValue::from_str(
        "Not yet implemented - will be added in next phase",
    ))
}
