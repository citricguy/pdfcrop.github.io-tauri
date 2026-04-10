//! Content stream filtering to remove elements outside crop box
//!
//! This module provides functionality to analyze PDF content streams and remove
//! drawing operations that fall completely outside the crop box, improving
//! privacy/security by ensuring clipped content is actually removed from the file.

use crate::bbox::BoundingBox;
use crate::error::{Error, Result};
mod bbox_utils;
mod font;
mod render_fallback;
use bbox_utils::expand_bbox;
use font::{FontCache, FontMetrics, Reliability, WritingMode};
use lopdf::{
    content::{Content, Operation},
    Dictionary, Document, Object, ObjectId, Stream,
};
pub use render_fallback::TextRenderFallback;

/// Task to recursively filter a Form XObject
#[derive(Clone)]
pub struct FormFilterTask {
    id: ObjectId,
    resources: Option<Dictionary>,
    ctm: [f64; 6],
}

/// Graphics state for tracking transformations and positions
#[derive(Debug, Clone)]
struct GraphicsState {
    /// Current transformation matrix [a b c d e f]
    ctm: [f64; 6],
    /// Current text matrix
    text_matrix: [f64; 6],
    /// Current text line matrix (used for T*, Td, TD)
    text_line_matrix: [f64; 6],
    /// Current text position
    text_pos: (f64, f64),
    /// Current font size (from Tf operator)
    font_size: f64,
    /// Current font name (from Tf operator)
    font_name: Option<Vec<u8>>,
    /// Character spacing (Tc)
    char_spacing: f64,
    /// Word spacing (Tw)
    word_spacing: f64,
    /// Horizontal scaling (Tz) expressed as factor (1.0 == 100%)
    horiz_scaling: f64,
    /// Text leading (TL)
    leading: f64,
    /// Text rise (Ts)
    text_rise: f64,
    /// Current stroke width
    line_width: f64,
}

impl Default for GraphicsState {
    fn default() -> Self {
        GraphicsState {
            ctm: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], // Identity matrix
            text_matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            text_line_matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            text_pos: (0.0, 0.0),
            font_size: 12.0,
            font_name: None,
            char_spacing: 0.0,
            word_spacing: 0.0,
            horiz_scaling: 1.0,
            leading: 0.0,
            text_rise: 0.0,
            line_width: 1.0,
        }
    }
}

#[derive(Debug, Clone)]
struct TextRenderState {
    font_name: Option<Vec<u8>>,
    font_size: f64,
    char_spacing: f64,
    word_spacing: f64,
    horiz_scaling: f64,
    leading: f64,
    text_rise: f64,
}

impl TextRenderState {
    fn from_graphics_state(state: &GraphicsState) -> Self {
        Self {
            font_name: state.font_name.clone(),
            font_size: state.font_size,
            char_spacing: state.char_spacing,
            word_spacing: state.word_spacing,
            horiz_scaling: state.horiz_scaling,
            leading: state.leading,
            text_rise: state.text_rise,
        }
    }
}

impl GraphicsState {
    /// Apply a transformation matrix to the CTM
    fn apply_transform(&mut self, matrix: &[f64; 6]) {
        // PDF spec: cm operator sets CTM = matrix × CTM (matrix is prepended)
        // So we compute: new_ctm = matrix * old_ctm
        let [a1, b1, c1, d1, e1, f1] = matrix; // The new matrix from cm
        let [a2, b2, c2, d2, e2, f2] = self.ctm; // Current CTM

        self.ctm = [
            a1 * a2 + b1 * c2,
            a1 * b2 + b1 * d2,
            c1 * a2 + d1 * c2,
            c1 * b2 + d1 * d2,
            e1 * a2 + f1 * c2 + e2,
            e1 * b2 + f1 * d2 + f2,
        ];
    }

    /// Transform a point from user space to device space
    fn transform_point(&self, x: f64, y: f64) -> (f64, f64) {
        let [a, b, c, d, e, f] = self.ctm;
        (a * x + c * y + e, b * x + d * y + f)
    }

    /// Reset text-related state when entering BT or when text resources change
    fn reset_text_state(&mut self) {
        self.text_matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        self.text_line_matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        self.text_pos = (0.0, 0.0);
        self.char_spacing = 0.0;
        self.word_spacing = 0.0;
        self.horiz_scaling = 1.0;
        self.leading = 0.0;
        self.text_rise = 0.0;
    }

    /// Set current text matrix and update cached position/line matrix
    fn set_text_matrix(&mut self, matrix: [f64; 6]) {
        self.text_matrix = matrix;
        self.text_line_matrix = matrix;
        self.update_text_position();
    }

    /// Translate current text matrix by tx, ty
    fn translate_text_matrix(&mut self, tx: f64, ty: f64) {
        let translation = [1.0, 0.0, 0.0, 1.0, tx, ty];
        self.text_matrix = multiply_matrices(&self.text_matrix, &translation);
        self.update_text_position();
    }

    /// Translate text line matrix (affects T*, Td, TD)
    fn translate_text_line_matrix(&mut self, tx: f64, ty: f64) {
        let translation = [1.0, 0.0, 0.0, 1.0, tx, ty];
        self.text_line_matrix = multiply_matrices(&self.text_line_matrix, &translation);
        self.text_matrix = self.text_line_matrix;
        self.update_text_position();
    }

    /// Move to start of next line (T*)
    fn move_to_next_line(&mut self) {
        let ty = -self.leading;
        self.translate_text_line_matrix(0.0, ty);
    }

    /// Get combined text matrix (text matrix composed with CTM)
    fn combined_text_matrix(&self) -> [f64; 6] {
        #[cfg(debug_assertions)]
        {
            let [a, b, c, d, _e, _f] = self.ctm;
            let [ta, tb, tc, td, te, tf] = self.text_matrix;
            if b.abs() > 0.1 || c.abs() > 0.1 {
                eprintln!("[DEBUG CTM] CTM: [{:.2}, {:.2}, {:.2}, {:.2}, {:.2}, {:.2}]", 
                         self.ctm[0], self.ctm[1], self.ctm[2], self.ctm[3], self.ctm[4], self.ctm[5]);
                eprintln!("[DEBUG CTM] TextMatrix: [{:.2}, {:.2}, {:.2}, {:.2}, {:.2}, {:.2}]",
                         ta, tb, tc, td, te, tf);
            }
        }
        multiply_matrices(&self.ctm, &self.text_matrix)
    }

    fn update_text_position(&mut self) {
        self.text_pos = (self.text_matrix[4], self.text_matrix[5]);
    }
}

/// Represents a filterable component in a PDF content stream
#[derive(Debug)]
enum ContentComponent {
    /// Path operations (path construction + painting operator)
    Path {
        operators: Vec<Operation>,
        bbox: Option<BoundingBox>,
        /// True if the path modifies the clipping path (contains W/W*)
        is_clipping: bool,
        /// CTM when the path was painted (for render fallback)
        ctm: [f64; 6],
        /// Stroke width at the time of painting
        line_width: f64,
    },
    /// Image XObject (Do operator with Image type)
    ImageXObject {
        operator: Operation,
        bbox: Option<BoundingBox>,
        /// CTM when the image was drawn
        ctm: [f64; 6],
    },
    /// Form XObject (Do operator with Form type) - now with proper bbox calculation
    FormXObject {
        operator: Operation,
        bbox: Option<BoundingBox>,
        /// CTM when the form was invoked (Matrix gets applied during Do)
        ctm: [f64; 6],
    },
    /// Text block (BT...ET)
    TextBlock {
        operators: Vec<Operation>,
        bbox: Option<BoundingBox>,
        estimated: bool,
        /// CTM active when entering the text block
        ctm: [f64; 6],
        /// Text state to seed rendering fallback
        render_state: Option<TextRenderState>,
        /// Text matrix before BT reset
        text_matrix: [f64; 6],
    },
    /// Orphan text operators (Tj/TJ/'/") that appear outside BT/ET blocks
    OrphanText {
        operator: Operation,
        bbox: Option<BoundingBox>,
        estimated: bool,
        /// CTM active when the operator was seen
        ctm: [f64; 6],
        /// Text state to seed rendering fallback
        render_state: Option<TextRenderState>,
        /// Text matrix when the operator was seen
        text_matrix: [f64; 6],
    },
    /// Graphics state operators (q, Q, cm, colors, line styles) - always kept
    GraphicsState { operators: Vec<Operation> },
}

fn flush_graphics_ops(components: &mut Vec<ContentComponent>, graphics_ops: &mut Vec<Operation>) {
    if !graphics_ops.is_empty() {
        let mut ops = Vec::new();
        std::mem::swap(&mut ops, graphics_ops);
        components.push(ContentComponent::GraphicsState { operators: ops });
    }
}

/// Parse PDF operations into filterable components
fn parse_into_components(
    doc: &Document,
    operations: &[Operation],
    resources: Option<&Dictionary>,
    base_ctm: &[f64; 6],
    form_tasks: &mut Vec<FormFilterTask>,
) -> Result<Vec<ContentComponent>> {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsValue;
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "[DEBUG] parse_into_components: Processing {} operations",
            operations.len()
        )));
        // Log all operators if there are few (to understand simple PDFs)
        let ops_to_log = if operations.len() <= 20 {
            operations.len()
        } else {
            10
        };
        for (i, op) in operations.iter().take(ops_to_log).enumerate() {
            let operands_str = op
                .operands
                .iter()
                .map(|o| match o {
                    Object::Name(n) => format!("Name({})", String::from_utf8_lossy(n)),
                    Object::Real(r) => format!("Real({})", r),
                    Object::Integer(i) => format!("Int({})", i),
                    Object::String(s, _) => format!("String({})", String::from_utf8_lossy(s)),
                    _ => format!("{:?}", o),
                })
                .collect::<Vec<_>>()
                .join(", ");
            web_sys::console::log_1(&JsValue::from_str(&format!(
                "[DEBUG] Op {}: {} [{}]",
                i, op.operator, operands_str
            )));
        }
    }

    let mut components = Vec::new();
    let mut state = GraphicsState::default();
    state.ctm = *base_ctm;
    let mut state_stack: Vec<GraphicsState> = Vec::new();

    // Buffers for building components
    let mut path_buffer: Vec<Operation> = Vec::new();
    let mut path_points: Vec<(f64, f64)> = Vec::new();
    let mut path_start = (0.0, 0.0);
    let mut current_point: Option<(f64, f64)> = None;
    let mut in_text_block = false;
    let mut text_block_ops: Vec<Operation> = Vec::new();
    let mut text_block_bbox: Option<BoundingBox> = None;
    let mut text_block_ctm = state.ctm;
    let mut text_block_render_state: Option<TextRenderState> = None;
    let mut text_block_text_matrix: [f64; 6] = state.text_matrix;
    let mut text_bbox_reliable = true;
    let mut graphics_state_ops: Vec<Operation> = Vec::new();
    let mut font_cache = FontCache::new();

    for (_op_idx, op) in operations.iter().enumerate() {
        let operator = op.operator.as_str();

        #[cfg(debug_assertions)]
        if operations.len() <= 5 {
            eprintln!(
                "[DEBUG] Operation {}: '{}' ({} bytes) with {} operands",
                _op_idx,
                operator,
                op.operator.len(),
                op.operands.len()
            );
            if operator == "Do" || operator.contains("o") {
                eprintln!("[DEBUG] Operator bytes: {:?}", op.operator.as_bytes());
                if let Some(first_operand) = op.operands.first() {
                    eprintln!("[DEBUG] First operand: {:?}", first_operand);
                }
            }
        }

        match operator {
            // Text block markers
            "BT" => {
                #[cfg(target_arch = "wasm32")]
                {
                    use wasm_bindgen::JsValue;
                    web_sys::console::log_1(&JsValue::from_str("[DEBUG] Found BT (Begin Text)"));
                }
                flush_graphics_ops(&mut components, &mut graphics_state_ops);
                in_text_block = true;
                text_block_ops.clear();
                text_block_ops.push(op.clone());
                text_block_bbox = None;
                text_block_ctm = state.ctm;
                text_block_render_state = Some(TextRenderState::from_graphics_state(&state));
                text_block_text_matrix = state.text_matrix;
                text_bbox_reliable = true;
                state.reset_text_state();
                // Seed text state with prior font settings so metrics and fallback have the correct font.
                // NOTE: We must NOT restore text_matrix/text_line_matrix here - PDF spec says
                // the text matrix is reset to identity at BT. Restoring the old matrix causes
                // text positions to incorrectly accumulate across BT/ET blocks.
                if let Some(render_state) = text_block_render_state.as_ref() {
                    apply_text_render_state(&mut state, render_state);
                    // text_matrix stays at identity from reset_text_state()
                }
            }
            "ET" => {
                text_block_ops.push(op.clone());
                #[cfg(target_arch = "wasm32")]
                {
                    use wasm_bindgen::JsValue;
                    web_sys::console::log_1(&JsValue::from_str(&format!(
                        "[DEBUG] Found ET (End Text) - creating TextBlock with {} operators",
                        text_block_ops.len()
                    )));
                }
                components.push(ContentComponent::TextBlock {
                    operators: text_block_ops.clone(),
                    bbox: if text_bbox_reliable {
                        text_block_bbox
                    } else {
                        None
                    },
                    estimated: !text_bbox_reliable,
                    ctm: text_block_ctm,
                    render_state: text_block_render_state.clone(),
                    text_matrix: text_block_text_matrix,
                });
                text_block_ops.clear();
                in_text_block = false;
                text_block_bbox = None;
                text_bbox_reliable = true;
                text_block_render_state = None;
            }

            // If inside text block, add to text block buffer
            _ if in_text_block => {
                text_block_ops.push(op.clone());
                // Update text state
                match operator {
                    "Tf" => {
                        if let Some(Object::Name(font_name)) = op.operands.first() {
                            state.font_name = Some(font_name.clone());
                        }
                        if let Some(size) = extract_number(&op.operands, 1) {
                            state.font_size = size;
                        }
                    }
                    "Tm" => {
                        if let Some(matrix) = extract_matrix(&op.operands) {
                            state.set_text_matrix(matrix);
                        }
                    }
                    "Td" | "TD" => {
                        if let (Some(tx), Some(ty)) = (
                            extract_number(&op.operands, 0),
                            extract_number(&op.operands, 1),
                        ) {
                            state.translate_text_line_matrix(tx, ty);
                            if operator == "TD" {
                                state.leading = -ty;
                            }
                        }
                    }
                    "T*" => {
                        state.move_to_next_line();
                    }
                    "Ts" => {
                        if let Some(rise) = extract_number(&op.operands, 0) {
                            state.text_rise = rise;
                        }
                    }
                    "Tw" => {
                        if let Some(space) = extract_number(&op.operands, 0) {
                            state.word_spacing = space;
                        }
                    }
                    "Tc" => {
                        if let Some(space) = extract_number(&op.operands, 0) {
                            state.char_spacing = space;
                        }
                    }
                    "Tz" => {
                        if let Some(scale) = extract_number(&op.operands, 0) {
                            state.horiz_scaling = scale / 100.0;
                        }
                    }
                    "TL" => {
                        if let Some(leading) = extract_number(&op.operands, 0) {
                            state.leading = leading;
                        }
                    }
                    "Tj" | "TJ" | "'" | "\"" => {
                        if let Some(font_name) = state.font_name.clone() {
                            let metrics = font_cache.get(doc, resources, &font_name);
                            let advance = match operator {
                                "Tj" => measure_text_from_string(
                                    op.operands.first().unwrap_or(&Object::Null),
                                    &metrics,
                                    &state,
                                ),
                                "TJ" => op
                                    .operands
                                    .first()
                                    .and_then(|arr| arr.as_array().ok())
                                    .and_then(|array| {
                                        measure_text_from_array(array, &metrics, &state)
                                    }),
                                "'" => {
                                    state.move_to_next_line();
                                    measure_text_from_string(
                                        op.operands.first().unwrap_or(&Object::Null),
                                        &metrics,
                                        &state,
                                    )
                                }
                                "\"" => {
                                    if let Some(space) = extract_number(&op.operands, 0) {
                                        state.word_spacing = space;
                                    }
                                    if let Some(space) = extract_number(&op.operands, 1) {
                                        state.char_spacing = space;
                                    }
                                    state.move_to_next_line();
                                    op.operands.get(2).and_then(|obj| {
                                        measure_text_from_string(obj, &metrics, &state)
                                    })
                                }
                                _ => None,
                            };

                            if let Some(adv) = advance {
                                if let Some(bbox) =
                                    calculate_text_bbox_from_state(&state, adv, &metrics)
                                {
                                    text_block_bbox = Some(match text_block_bbox {
                                        Some(existing) => existing.union(&bbox),
                                        None => bbox,
                                    });
                                }
                                if metrics.writing_mode == WritingMode::Vertical {
                                    state.translate_text_matrix(0.0, -adv);
                                } else {
                                    state.translate_text_matrix(adv, 0.0);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }

            // Path construction operators
            "m" | "l" | "c" | "v" | "y" | "re" | "h" => {
                // Flush any pending graphics state ops
                if !graphics_state_ops.is_empty() {
                    components.push(ContentComponent::GraphicsState {
                        operators: graphics_state_ops.clone(),
                    });
                    graphics_state_ops.clear();
                }

                path_buffer.push(op.clone());

                // Track path points for bbox calculation
                match operator {
                    "m" => {
                        if let (Some(x), Some(y)) = (
                            extract_number(&op.operands, 0),
                            extract_number(&op.operands, 1),
                        ) {
                            let pos = state.transform_point(x, y);
                            path_points.clear();
                            path_points.push(pos);
                            path_start = pos;
                            current_point = Some(pos);
                        }
                    }
                    "l" => {
                        if let (Some(x), Some(y)) = (
                            extract_number(&op.operands, 0),
                            extract_number(&op.operands, 1),
                        ) {
                            path_points.push(state.transform_point(x, y));
                            current_point = Some(state.transform_point(x, y));
                        }
                    }
                    "c" | "v" | "y" => {
                        // Cubic Bezier curve - include control points and extrema for bbox
                        let p0 = current_point;
                        match operator {
                            "c" => {
                                if op.operands.len() >= 6 {
                                    let p1 = state.transform_point(
                                        extract_number(&op.operands, 0).unwrap_or(0.0),
                                        extract_number(&op.operands, 1).unwrap_or(0.0),
                                    );
                                    let p2 = state.transform_point(
                                        extract_number(&op.operands, 2).unwrap_or(0.0),
                                        extract_number(&op.operands, 3).unwrap_or(0.0),
                                    );
                                    let p3 = state.transform_point(
                                        extract_number(&op.operands, 4).unwrap_or(0.0),
                                        extract_number(&op.operands, 5).unwrap_or(0.0),
                                    );
                                    if let Some(start) = p0 {
                                        extend_path_with_cubic_points(
                                            &mut path_points,
                                            start,
                                            p1,
                                            p2,
                                            p3,
                                        );
                                    } else {
                                        path_points.push(p1);
                                        path_points.push(p2);
                                        path_points.push(p3);
                                    }
                                    current_point = Some(p3);
                                }
                            }
                            "v" => {
                                if op.operands.len() >= 4 {
                                    let p1 = p0.unwrap_or((0.0, 0.0)); // first control is current point
                                    let p2 = state.transform_point(
                                        extract_number(&op.operands, 0).unwrap_or(0.0),
                                        extract_number(&op.operands, 1).unwrap_or(0.0),
                                    );
                                    let p3 = state.transform_point(
                                        extract_number(&op.operands, 2).unwrap_or(0.0),
                                        extract_number(&op.operands, 3).unwrap_or(0.0),
                                    );
                                    if let Some(start) = p0 {
                                        extend_path_with_cubic_points(
                                            &mut path_points,
                                            start,
                                            p1,
                                            p2,
                                            p3,
                                        );
                                    } else {
                                        path_points.push(p2);
                                        path_points.push(p3);
                                    }
                                    current_point = Some(p3);
                                }
                            }
                            "y" => {
                                if op.operands.len() >= 4 {
                                    let p1 = state.transform_point(
                                        extract_number(&op.operands, 0).unwrap_or(0.0),
                                        extract_number(&op.operands, 1).unwrap_or(0.0),
                                    );
                                    let p3 = state.transform_point(
                                        extract_number(&op.operands, 2).unwrap_or(0.0),
                                        extract_number(&op.operands, 3).unwrap_or(0.0),
                                    );
                                    let p2 = p3; // second control is the endpoint for 'y'
                                    if let Some(start) = p0 {
                                        extend_path_with_cubic_points(
                                            &mut path_points,
                                            start,
                                            p1,
                                            p2,
                                            p3,
                                        );
                                    } else {
                                        path_points.push(p1);
                                        path_points.push(p3);
                                    }
                                    current_point = Some(p3);
                                }
                            }
                            _ => {}
                        }
                    }
                    "re" => {
                        if let (Some(x), Some(y), Some(w), Some(h)) = (
                            extract_number(&op.operands, 0),
                            extract_number(&op.operands, 1),
                            extract_number(&op.operands, 2),
                            extract_number(&op.operands, 3),
                        ) {
                            path_points.clear();
                            path_points.push(state.transform_point(x, y));
                            path_points.push(state.transform_point(x + w, y));
                            path_points.push(state.transform_point(x + w, y + h));
                            path_points.push(state.transform_point(x, y + h));
                            current_point = Some(state.transform_point(x, y + h));
                        }
                    }
                    "h" => {
                        if !path_points.is_empty() {
                            path_points.push(path_start);
                        }
                        current_point = Some(path_start);
                    }
                    _ => {}
                }
            }

            // Path painting operators - commit the path component
            "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" => {
                path_buffer.push(op.clone());

                // Calculate bbox from path points
                let bbox = if path_points.is_empty() {
                    None
                } else {
                    calculate_path_bbox(&path_points)
                };

                components.push(ContentComponent::Path {
                    operators: path_buffer.clone(),
                    bbox,
                    is_clipping: path_buffer
                        .iter()
                        .any(|op| matches!(op.operator.as_str(), "W" | "W*")),
                    ctm: state.ctm,
                    line_width: state.line_width,
                });

                path_buffer.clear();
                path_points.clear();
                current_point = None; // Path ends after painting
            }

            // Clipping operators - add to path buffer
            "W" | "W*" => {
                path_buffer.push(op.clone());
            }

            // End path without painting - keep if it sets a clipping path
            "n" => {
                let has_clip = path_buffer
                    .iter()
                    .any(|op| matches!(op.operator.as_str(), "W" | "W*"));
                if has_clip {
                    path_buffer.push(op.clone()); // Preserve the path terminator
                    let bbox = if path_points.is_empty() {
                        None
                    } else {
                        calculate_path_bbox(&path_points)
                    };
                    components.push(ContentComponent::Path {
                        operators: path_buffer.clone(),
                        bbox,
                        is_clipping: true,
                        ctm: state.ctm,
                        line_width: state.line_width,
                    });
                }
                path_buffer.clear();
                path_points.clear();
                current_point = None;
            }

            // XObject operator (Do)
            "Do" => {
                // Flush any pending graphics state ops
                if !graphics_state_ops.is_empty() {
                    components.push(ContentComponent::GraphicsState {
                        operators: graphics_state_ops.clone(),
                    });
                    graphics_state_ops.clear();
                }

                if let Some(Object::Name(xobj_name)) = op.operands.first() {
                    if let Some(resources_dict) = resources {
                        // Try to determine if it's an Image or Form XObject
                        match get_xobject_type(doc, resources_dict, xobj_name) {
                            XObjectType::Image => {
                                // Calculate bbox for image placement
                                let bbox = calculate_image_bbox(&state.ctm);
                                components.push(ContentComponent::ImageXObject {
                                    operator: op.clone(),
                                    bbox,
                                    ctm: state.ctm,
                                });
                            }
                            XObjectType::Form => {
                                #[cfg(debug_assertions)]
                                eprintln!(
                                    "[DEBUG] Processing Form XObject: {}",
                                    String::from_utf8_lossy(xobj_name)
                                );

                                    // Calculate bbox for Form XObject with proper transformation
                                    let (bbox, maybe_task, combined_ctm) =
                                        if let Ok((xobj_ref, xobj_resources)) =
                                            get_form_xobject_ref(doc, resources_dict, xobj_name)
                                        {
                                            #[cfg(debug_assertions)]
                                            eprintln!("[DEBUG] Got XObject reference: {:?}", xobj_ref);

                                            let form_matrix = get_form_matrix(doc, xobj_ref)
                                                .unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
                                            let combined_ctm =
                                                multiply_matrices(&state.ctm, &form_matrix);
                                            let bbox = calculate_form_xobject_bbox(
                                                doc,
                                                xobj_ref,
                                                &state.ctm,
                                            );

                                            // Queue recursive filtering with combined CTM
                                            let task = FormFilterTask {
                                                id: xobj_ref,
                                                resources: xobj_resources.clone(),
                                                ctm: combined_ctm,
                                            };
                                            (bbox, Some(task), combined_ctm)
                                        } else {
                                            #[cfg(debug_assertions)]
                                            eprintln!("[DEBUG] Failed to get XObject reference");
                                            (None, None, state.ctm)
                                        };

                                if let Some(task) = maybe_task {
                                    form_tasks.push(task);
                                }

                                #[cfg(target_arch = "wasm32")]
                                if let Some(ref b) = bbox {
                                    use wasm_bindgen::JsValue;
                                    web_sys::console::log_1(&JsValue::from_str(&format!(
                                        "[DEBUG] Form XObject bbox: ({:.2}, {:.2}, {:.2}, {:.2})",
                                        b.left, b.bottom, b.right, b.top
                                    )));
                                }

                                components.push(ContentComponent::FormXObject {
                                    operator: op.clone(),
                                    bbox,
                                    ctm: combined_ctm,
                                });
                            }
                            XObjectType::Unknown => {
                                // Unknown type - keep as Form XObject to be safe
                                components.push(ContentComponent::FormXObject {
                                    operator: op.clone(),
                                    bbox: None,
                                    ctm: state.ctm,
                                });
                            }
                        }
                    } else {
                        // No resources - keep as Form XObject
                        components.push(ContentComponent::FormXObject {
                            operator: op.clone(),
                            bbox: None,
                            ctm: state.ctm,
                        });
                    }
                } else {
                    // Invalid Do operator - keep it
                    graphics_state_ops.push(op.clone());
                }
            }

            // Graphics state operators - buffer them
            "q" => {
                state_stack.push(state.clone());
                graphics_state_ops.push(op.clone());
            }
            "Q" => {
                if let Some(saved_state) = state_stack.pop() {
                    state = saved_state;
                }
                graphics_state_ops.push(op.clone());
            }
            "cm" => {
                if let Some(matrix) = extract_matrix(&op.operands) {
                    state.apply_transform(&matrix);
                }
                graphics_state_ops.push(op.clone());
            }

            // Color, line style, and other graphics state operators
            "CS" | "cs" | "SC" | "SCN" | "sc" | "scn" | "G" | "g" | "RG" | "rg" | "K" | "k"
            | "w" | "J" | "j" | "M" | "d" | "ri" | "i" | "gs" => {
                if operator == "w" {
                    if let Some(width) = extract_number(&op.operands, 0) {
                        state.line_width = width;
                    }
                }
                // If we are building a path, these state operators should be part of the path component
                // to ensure they stay in the correct order relative to the path painting operator.
                if !path_buffer.is_empty() {
                    path_buffer.push(op.clone());
                } else {
                    graphics_state_ops.push(op.clone());
                }
            }

            // Marked content operators
            "BMC" | "BDC" | "EMC" | "MP" | "DP" => {
                graphics_state_ops.push(op.clone());
            }

            // Text showing operators that might appear outside BT/ET (invalid but happens)
            "Tj" | "TJ" | "'" | "\"" => {
                // Skip text operators with no operands - this can happen when content streams
                // are meant to be concatenated (one ends with [(text)] and next starts with TJ)
                // but we filter them separately. A bare TJ without operands is invalid PDF.
                if op.operands.is_empty() {
                    #[cfg(not(target_arch = "wasm32"))]
                    eprintln!(
                        "[WARNING] Skipping '{}' operator with no operands (likely stream boundary issue)",
                        op.operator
                    );
                    continue;
                }
                flush_graphics_ops(&mut components, &mut graphics_state_ops);
                if let Some(component) =
                    handle_orphan_text_operation(doc, resources, op, &mut state, &mut font_cache)
                {
                    components.push(component);
                } else {
                    components.push(ContentComponent::GraphicsState {
                        operators: vec![op.clone()],
                    });
                }
            }

            // Text state and font operators - track and keep
            "Tf" => {
                // Font selection - update font size for text bbox estimation
                if let Some(Object::Name(font_name)) = op.operands.first() {
                    state.font_name = Some(font_name.clone());
                }
                if let Some(size) = extract_number(&op.operands, 1) {
                    state.font_size = size;

                    #[cfg(not(target_arch = "wasm32"))]
                    eprintln!("[DEBUG] Tf outside BT/ET: font size = {:.1}", size);
                }
                graphics_state_ops.push(op.clone());
            }

            "Ts" | "Tz" | "TL" | "Tw" | "Tc" | "Tr" => {
                match operator {
                    "Ts" => {
                        if let Some(rise) = extract_number(&op.operands, 0) {
                            state.text_rise = rise;
                        }
                    }
                    "Tz" => {
                        if let Some(scale) = extract_number(&op.operands, 0) {
                            state.horiz_scaling = scale / 100.0;
                        }
                    }
                    "TL" => {
                        if let Some(leading) = extract_number(&op.operands, 0) {
                            state.leading = leading;
                        }
                    }
                    "Tw" => {
                        if let Some(space) = extract_number(&op.operands, 0) {
                            state.word_spacing = space;
                        }
                    }
                    "Tc" => {
                        if let Some(space) = extract_number(&op.operands, 0) {
                            state.char_spacing = space;
                        }
                    }
                    _ => {}
                }
                graphics_state_ops.push(op.clone());
            }

            // Text positioning operators that might appear outside BT/ET
            "Tm" => {
                // Text matrix - sets absolute text position
                if let Some(matrix) = extract_matrix(&op.operands) {
                    state.set_text_matrix(matrix);

                    #[cfg(not(target_arch = "wasm32"))]
                    eprintln!(
                        "[DEBUG] Tm outside BT/ET: pos = ({:.1}, {:.1})",
                        state.text_pos.0, state.text_pos.1
                    );
                }
                graphics_state_ops.push(op.clone());
            }

            "Td" | "TD" => {
                // Text position - relative move
                if let (Some(tx), Some(ty)) = (
                    extract_number(&op.operands, 0),
                    extract_number(&op.operands, 1),
                ) {
                    state.translate_text_line_matrix(tx, ty);
                    if operator == "TD" {
                        state.leading = -ty;
                    }

                    #[cfg(not(target_arch = "wasm32"))]
                    eprintln!(
                        "[DEBUG] {} outside BT/ET: pos = ({:.1}, {:.1})",
                        operator, state.text_pos.0, state.text_pos.1
                    );
                }
                graphics_state_ops.push(op.clone());
            }

            "T*" => {
                // Move to start of next line
                state.move_to_next_line();
                graphics_state_ops.push(op.clone());
            }

            // Unknown operators - add to graphics state to be safe
            _ => {
                graphics_state_ops.push(op.clone());
            }
        }
    }

    // Flush any remaining graphics state ops
    if !graphics_state_ops.is_empty() {
        components.push(ContentComponent::GraphicsState {
            operators: graphics_state_ops,
        });
    }

    // IMPORTANT: If we're still in a text block (unmatched BT), flush it
    if in_text_block && !text_block_ops.is_empty() {
        // #[cfg(debug_assertions)]
        // eprintln!("[WARNING] Unmatched BT - text block never ended with ET!");

        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::JsValue;
            web_sys::console::log_1(&JsValue::from_str(&format!(
                "[WARNING] Unmatched BT - creating TextBlock with {} operators",
                text_block_ops.len()
            )));
        }

        components.push(ContentComponent::TextBlock {
            operators: text_block_ops,
            bbox: if text_bbox_reliable {
                text_block_bbox
            } else {
                None
            },
            estimated: !text_bbox_reliable,
            ctm: text_block_ctm,
            render_state: text_block_render_state,
            text_matrix: text_block_text_matrix,
        });
    }

    Ok(components)
}

/// Calculate bounding box from path points
fn calculate_path_bbox(points: &[(f64, f64)]) -> Option<BoundingBox> {
    if points.is_empty() {
        return None;
    }

    let min_x = points
        .iter()
        .map(|(x, _)| x)
        .fold(f64::INFINITY, |a, &b| a.min(b));
    let max_x = points
        .iter()
        .map(|(x, _)| x)
        .fold(f64::NEG_INFINITY, |a, &b| a.max(b));
    let min_y = points
        .iter()
        .map(|(_, y)| y)
        .fold(f64::INFINITY, |a, &b| a.min(b));
    let max_y = points
        .iter()
        .map(|(_, y)| y)
        .fold(f64::NEG_INFINITY, |a, &b| a.max(b));

    BoundingBox::new(min_x, min_y, max_x, max_y).ok()
}

/// Extend path point set with cubic Bezier control/extrema to build an accurate bbox
fn extend_path_with_cubic_points(
    points: &mut Vec<(f64, f64)>,
    p0: (f64, f64),
    p1: (f64, f64),
    p2: (f64, f64),
    p3: (f64, f64),
) {
    // Include endpoint (start point is already present in the path)
    points.push(p3);

    // Include extrema along X and Y (solve derivative=0)
    for t in cubic_extrema_1d(p0.0, p1.0, p2.0, p3.0) {
        points.push(eval_cubic(p0, p1, p2, p3, t));
    }
    for t in cubic_extrema_1d(p0.1, p1.1, p2.1, p3.1) {
        points.push(eval_cubic(p0, p1, p2, p3, t));
    }
}

fn cubic_extrema_1d(p0: f64, p1: f64, p2: f64, p3: f64) -> Vec<f64> {
    // Coefficients for derivative of cubic Bezier
    let a = -p0 + 3.0 * p1 - 3.0 * p2 + p3;
    let b = 2.0 * (p0 - 2.0 * p1 + p2);
    let c = p1 - p0;

    let mut ts = Vec::new();
    const EPS: f64 = 1e-9;

    if a.abs() < EPS {
        if b.abs() > EPS {
            let t = -c / b;
            if (0.0..=1.0).contains(&t) {
                ts.push(t);
            }
        }
    } else {
        let disc = b * b - 4.0 * a * c;
        if disc >= 0.0 {
            let sqrt_disc = disc.sqrt();
            let t1 = (-b + sqrt_disc) / (2.0 * a);
            let t2 = (-b - sqrt_disc) / (2.0 * a);
            for t in [t1, t2] {
                if (0.0..=1.0).contains(&t) {
                    ts.push(t);
                }
            }
        }
    }

    ts
}

fn eval_cubic(
    p0: (f64, f64),
    p1: (f64, f64),
    p2: (f64, f64),
    p3: (f64, f64),
    t: f64,
) -> (f64, f64) {
    let mt = 1.0 - t;
    let mt2 = mt * mt;
    let t2 = t * t;

    let x = mt2 * mt * p0.0 + 3.0 * mt2 * t * p1.0 + 3.0 * mt * t2 * p2.0 + t2 * t * p3.0;
    let y = mt2 * mt * p0.1 + 3.0 * mt2 * t * p1.1 + 3.0 * mt * t2 * p2.1 + t2 * t * p3.1;

    (x, y)
}

fn handle_orphan_text_operation(
    doc: &Document,
    resources: Option<&Dictionary>,
    op: &Operation,
    state: &mut GraphicsState,
    font_cache: &mut FontCache,
) -> Option<ContentComponent> {
    let operator = op.operator.as_str();
    let font_name = match state.font_name.clone() {
        Some(name) => name,
        None => {
            #[cfg(not(target_arch = "wasm32"))]
            eprintln!(
                "[WARNING] Orphaned '{}' encountered without active font - keeping in stream",
                operator
            );
            return None;
        }
    };
    let metrics = match font_cache.get(doc, resources, &font_name) {
        metrics => metrics,
    };

    let advance = match operator {
        "Tj" => measure_text_from_string(op.operands.first()?, &metrics, state)?,
        "TJ" => {
            let array = op.operands.first()?.as_array().ok()?;
            measure_text_from_array(array, &metrics, state)?
        }
        "'" => {
            state.move_to_next_line();
            measure_text_from_string(op.operands.first()?, &metrics, state)?
        }
        "\"" => {
            if let Some(space) = extract_number(&op.operands, 0) {
                state.word_spacing = space;
            }
            if let Some(space) = extract_number(&op.operands, 1) {
                state.char_spacing = space;
            }
            state.move_to_next_line();
            measure_text_from_string(op.operands.get(2)?, &metrics, state)?
        }
        _ => return None,
    };

    let bbox = calculate_text_bbox_from_state(state, advance, &metrics);
    if metrics.writing_mode == WritingMode::Vertical {
        state.translate_text_matrix(0.0, -advance);
    } else {
        state.translate_text_matrix(advance, 0.0);
    }

    Some(ContentComponent::OrphanText {
        operator: op.clone(),
        bbox,
        estimated: metrics.reliability == Reliability::Estimated,
        ctm: state.ctm,
        render_state: Some(TextRenderState::from_graphics_state(state)),
        text_matrix: state.text_matrix,
    })
}

fn measure_text_from_string(
    operand: &Object,
    metrics: &FontMetrics,
    state: &GraphicsState,
) -> Option<f64> {
    if let Some(bytes) = extract_string_bytes(operand) {
        Some(measure_text_displacement(&bytes, metrics, state))
    } else {
        // Fallback: estimate width using default_width
        Some((metrics.default_width / 1000.0) * state.font_size * metrics.bytes_per_char as f64)
    }
}

fn measure_text_from_array(
    array: &[Object],
    metrics: &FontMetrics,
    state: &GraphicsState,
) -> Option<f64> {
    let mut width = 0.0;
    let mut any = false;
    for item in array {
        match item {
            Object::String(_, _) => {
                let bytes = extract_string_bytes(item)?;
                width += measure_text_displacement(&bytes, metrics, state);
                any = true;
            }
            Object::Integer(val) => {
                width -= (*val as f64 / 1000.0) * state.font_size * state.horiz_scaling;
            }
            Object::Real(val) => {
                width -= (*val as f64 / 1000.0) * state.font_size * state.horiz_scaling;
            }
            _ => {}
        }
    }
    if any {
        Some(width)
    } else {
        // Fallback: estimate width using default_width times element count
        Some(
            (metrics.default_width / 1000.0)
                * state.font_size
                * metrics.bytes_per_char as f64
                * array.len() as f64,
        )
    }
}

fn measure_text_displacement(bytes: &[u8], metrics: &FontMetrics, state: &GraphicsState) -> f64 {
    let mut advance_total = 0.0;
    let scale = state.horiz_scaling;
    for code in decode_text_codes(bytes, metrics) {
        let mut advance = (metrics.glyph_width(code) / 1000.0) * state.font_size;
        advance += state.char_spacing;
        if !metrics.is_cid && code == 32 {
            advance += state.word_spacing;
        }
        advance_total += advance * scale;
    }
    advance_total
}

fn decode_text_codes(bytes: &[u8], metrics: &FontMetrics) -> Vec<u32> {
    if metrics.is_cid {
        let mut codes = Vec::new();
        let bpc = metrics.bytes_per_char.max(1);
        for chunk in bytes.chunks(bpc) {
            if chunk.len() == bpc {
                if let Some(ref cmap) = metrics.cmap {
                    if let Some(val) = cmap.get(chunk) {
                        codes.push(*val);
                        continue;
                    }
                }
                let mut value = 0u32;
                for &b in chunk {
                    value = (value << 8) | b as u32;
                }
                codes.push(value);
            }
        }
        if !codes.is_empty() {
            return codes;
        }
        // Fallback: treat as single-byte codes if multi-byte decode failed
        return bytes.iter().map(|b| *b as u32).collect();
    } else {
        bytes.iter().map(|b| *b as u32).collect()
    }
}

fn extract_string_bytes(obj: &Object) -> Option<Vec<u8>> {
    match obj {
        Object::String(bytes, _) => Some(bytes.clone()),
        _ => None,
    }
}

fn calculate_text_bbox_from_state(
    state: &GraphicsState,
    advance: f64,
    metrics: &FontMetrics,
) -> Option<BoundingBox> {
    let adv = if advance.abs() < f64::EPSILON {
        (metrics.default_width / 1000.0) * state.font_size
    } else {
        advance
    };

    let ascent = (metrics.ascent / 1000.0) * state.font_size + state.text_rise;
    let descent = (metrics.descent / 1000.0) * state.font_size + state.text_rise;
    let combined = state.combined_text_matrix();

    #[cfg(debug_assertions)]
    {
        // Check if this is a rotated matrix (off-diagonal elements non-zero)
        let [a, b, c, d, e, f] = combined;
        if b.abs() > 0.1 || c.abs() > 0.1 {
            eprintln!("[DEBUG] Rotated text matrix: [{:.2}, {:.2}, {:.2}, {:.2}, {:.2}, {:.2}]", a, b, c, d, e, f);
            eprintln!("[DEBUG] Text params: advance={:.2}, ascent={:.2}, descent={:.2}, font_size={:.2}", 
                     advance, ascent, descent, state.font_size);
        }
    }

    let points = if metrics.writing_mode == WritingMode::Vertical {
        let glyph_width = (ascent - descent).abs().max(state.font_size * 0.5);
        let half_w = glyph_width / 2.0;
        [
            transform_point_with_matrix(&combined, -half_w, 0.0),
            transform_point_with_matrix(&combined, half_w, 0.0),
            transform_point_with_matrix(&combined, half_w, -advance),
            transform_point_with_matrix(&combined, -half_w, -advance),
        ]
    } else {
        let p1 = transform_point_with_matrix(&combined, 0.0, descent);
        let p2 = transform_point_with_matrix(&combined, adv, descent);
        let p3 = transform_point_with_matrix(&combined, adv, ascent.max(descent + 0.1));
        let p4 = transform_point_with_matrix(&combined, 0.0, ascent.max(descent + 0.1));
        
        #[cfg(debug_assertions)]
        {
            let [a, b, c, _d, _e, _f] = combined;
            if b.abs() > 0.1 || c.abs() > 0.1 {
                eprintln!("[DEBUG] Transformed corners: p1=({:.2},{:.2}), p2=({:.2},{:.2}), p3=({:.2},{:.2}), p4=({:.2},{:.2})",
                         p1.0, p1.1, p2.0, p2.1, p3.0, p3.1, p4.0, p4.1);
            }
        }
        
        [p1, p2, p3, p4]
    };

    calculate_path_bbox(&points)
}

fn transform_point_with_matrix(matrix: &[f64; 6], x: f64, y: f64) -> (f64, f64) {
    let [a, b, c, d, e, f] = matrix;
    (a * x + c * y + e, b * x + d * y + f)
}

fn apply_text_render_state(state: &mut GraphicsState, render_state: &TextRenderState) {
    state.font_name = render_state.font_name.clone();
    state.font_size = render_state.font_size;
    state.char_spacing = render_state.char_spacing;
    state.word_spacing = render_state.word_spacing;
    state.horiz_scaling = render_state.horiz_scaling;
    state.leading = render_state.leading;
    state.text_rise = render_state.text_rise;
}

/// Calculate bounding box for image XObject placement
/// Images are placed at (0,0)-(1,1) in user space, transformed by CTM
fn calculate_image_bbox(ctm: &[f64; 6]) -> Option<BoundingBox> {
    // Image corners in user space: (0,0), (1,0), (1,1), (0,1)
    let corners = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];

    // Transform corners by CTM
    let [a, b, c, d, e, f] = ctm;
    let transformed: Vec<(f64, f64)> = corners
        .iter()
        .map(|(x, y)| (a * x + c * y + e, b * x + d * y + f))
        .collect();

    calculate_path_bbox(&transformed)
}

/// Get the Matrix entry of a Form XObject (or identity if missing/invalid)
fn get_form_matrix(doc: &Document, xobj_ref: ObjectId) -> Option<[f64; 6]> {
    let xobj = doc.get_object(xobj_ref).ok()?;
    let stream = xobj.as_stream().ok()?;
    let dict = &stream.dict;

    if let Ok(matrix_obj) = dict.get(b"Matrix") {
        if let Ok(matrix_array) = matrix_obj.as_array() {
            if matrix_array.len() == 6 {
                let mut m = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
                for (i, val) in matrix_array.iter().enumerate() {
                    m[i] = val
                        .as_f32()
                        .unwrap_or(if i == 0 || i == 3 { 1.0 } else { 0.0 })
                        as f64;
                }
                return Some(m);
            }
        }
    }

    Some([1.0, 0.0, 0.0, 1.0, 0.0, 0.0])
}

/// Calculate bounding box for a Form XObject by transforming its BBox to page space
fn calculate_form_xobject_bbox(
    doc: &Document,
    xobj_ref: ObjectId,
    page_ctm: &[f64; 6],
) -> Option<BoundingBox> {
    // Get the Form XObject stream
    let xobj = doc.get_object(xobj_ref).ok()?;
    let stream = xobj.as_stream().ok()?;
    let dict = &stream.dict;

    // Get the BBox from the Form XObject (required for Form XObjects)
    let bbox_array = dict.get(b"BBox").ok()?.as_array().ok()?;
    if bbox_array.len() != 4 {
        return None;
    }

    // Parse BBox coordinates
    let x1 = bbox_array[0].as_f32().unwrap_or(0.0) as f64;
    let y1 = bbox_array[1].as_f32().unwrap_or(0.0) as f64;
    let x2 = bbox_array[2].as_f32().unwrap_or(0.0) as f64;
    let y2 = bbox_array[3].as_f32().unwrap_or(0.0) as f64;

    // Get the transformation Matrix if present (default is identity)
    let matrix = get_form_matrix(doc, xobj_ref).unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    // Combine the Form XObject's matrix with the page CTM
    let combined_ctm = multiply_matrices(page_ctm, &matrix);

    // Transform the four corners of the BBox
    let corners = [(x1, y1), (x2, y1), (x2, y2), (x1, y2)];

    let [a, b, c, d, e, f] = combined_ctm;
    let transformed: Vec<(f64, f64)> = corners
        .iter()
        .map(|(x, y)| (a * x + c * y + e, b * x + d * y + f))
        .collect();

    calculate_path_bbox(&transformed)
}

/// Multiply two transformation matrices
fn multiply_matrices(m1: &[f64; 6], m2: &[f64; 6]) -> [f64; 6] {
    let [a1, b1, c1, d1, e1, f1] = m1;
    let [a2, b2, c2, d2, e2, f2] = m2;

    [
        a1 * a2 + b1 * c2,
        a1 * b2 + b1 * d2,
        c1 * a2 + d1 * c2,
        c1 * b2 + d1 * d2,
        e1 * a2 + f1 * c2 + e2,
        e1 * b2 + f1 * d2 + f2,
    ]
}

#[derive(Debug, Clone, Copy)]
enum XObjectType {
    Image,
    Form,
    Unknown,
}

/// Determine the type of XObject (Image or Form)
fn get_xobject_type(doc: &Document, resources: &Dictionary, xobj_name: &[u8]) -> XObjectType {
    let xobject_ref = resources.get(b"XObject");
    if xobject_ref.is_err() {
        return XObjectType::Unknown;
    }

    let xobject_dict = xobject_ref.unwrap().as_dict();
    if xobject_dict.is_err() {
        return XObjectType::Unknown;
    }

    let xobj_ref = xobject_dict
        .unwrap()
        .get(xobj_name)
        .ok()
        .and_then(|obj| obj.as_reference().ok());
    if xobj_ref.is_none() {
        return XObjectType::Unknown;
    }

    let xobj_stream = doc.get_object(xobj_ref.unwrap());
    if xobj_stream.is_err() {
        return XObjectType::Unknown;
    }

    let xobj_stream = xobj_stream.unwrap().as_stream();
    if xobj_stream.is_err() {
        return XObjectType::Unknown;
    }

    // Check Subtype
    let subtype = xobj_stream
        .unwrap()
        .dict
        .get(b"Subtype")
        .ok()
        .and_then(|obj| obj.as_name().ok());

    match subtype {
        Some(b"Image") => XObjectType::Image,
        Some(b"Form") => XObjectType::Form,
        _ => XObjectType::Unknown,
    }
}

fn build_text_preamble(
    render_state: Option<&TextRenderState>,
    text_matrix: Option<[f64; 6]>,
) -> Vec<Operation> {
    let mut ops = Vec::new();
    if let Some(tm) = text_matrix {
        ops.push(Operation::new(
            "Tm",
            tm.iter().copied().map(|v| Object::Real(v as f32)).collect(),
        ));
    }

    if let Some(state) = render_state {
        if let Some(font) = &state.font_name {
            ops.push(Operation::new(
                "Tf",
                vec![
                    Object::Name(font.clone()),
                    Object::Real(state.font_size as f32),
                ],
            ));
        }
        if state.char_spacing.abs() > f64::EPSILON {
            ops.push(Operation::new(
                "Tc",
                vec![Object::Real(state.char_spacing as f32)],
            ));
        }
        if state.word_spacing.abs() > f64::EPSILON {
            ops.push(Operation::new(
                "Tw",
                vec![Object::Real(state.word_spacing as f32)],
            ));
        }
        if (state.horiz_scaling - 1.0).abs() > f64::EPSILON {
            ops.push(Operation::new(
                "Tz",
                vec![Object::Real((state.horiz_scaling * 100.0) as f32)],
            ));
        }
        if state.leading.abs() > f64::EPSILON {
            ops.push(Operation::new(
                "TL",
                vec![Object::Real(state.leading as f32)],
            ));
        }
        if state.text_rise.abs() > f64::EPSILON {
            ops.push(Operation::new(
                "Ts",
                vec![Object::Real(state.text_rise as f32)],
            ));
        }
    }

    ops
}

#[cfg(debug_assertions)]
fn ops_contains_keyword(ops: &[Operation], needle: &str) -> bool {
    let needle = needle.as_bytes();
    let contains = |buf: &[u8]| {
        buf.windows(needle.len())
            .any(|w| w.eq_ignore_ascii_case(needle))
    };
    for op in ops {
        for obj in &op.operands {
            match obj {
                Object::String(data, _) => {
                    if contains(data) {
                        return true;
                    }
                }
                Object::Name(data) => {
                    if contains(data) {
                        return true;
                    }
                }
                _ => {}
            };
        }
    }
    false
}

/// Filter components based on bbox overlap with crop box
fn filter_components(
    components: Vec<ContentComponent>,
    crop_box: &BoundingBox,
    render_fallback: &mut Option<TextRenderFallback>,
    force_keep: bool,
) -> Vec<Operation> {
    const PATH_MARGIN: f64 = 10.0;
    const IMAGE_MARGIN: f64 = 10.0;
    const FORM_MARGIN: f64 = 12.0;
    const CLIP_MARGIN: f64 = 2.0; // Small cushion for clip paths to account for numeric/curve bounds
    // Extra guard band to avoid false negatives from imperfect bboxes or CTM confusion.
    // Keep it small to avoid leaking off-crop content.
    const KEEP_GUARD: f64 = 8.0;
    fn is_paint_op(op: &Operation) -> bool {
        matches!(
            op.operator.as_str(),
            "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n"
        )
    }
    fn has_paint(ops: &[Operation]) -> bool {
        ops.iter().any(is_paint_op)
    }

    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsValue;
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "[DEBUG] Filtering {} components with crop box: ({:.2}, {:.2}, {:.2}, {:.2})",
            components.len(),
            crop_box.left,
            crop_box.bottom,
            crop_box.right,
            crop_box.top
        )));
    }

    #[cfg(debug_assertions)]
    eprintln!(
        "[DEBUG] filter_components: {} components, crop: ({:.1}, {:.1}, {:.1}, {:.1})",
        components.len(),
        crop_box.left,
        crop_box.bottom,
        crop_box.right,
        crop_box.top
    );

    #[cfg(debug_assertions)]
    const TARGET_POINTS: [((f64, f64), &str); 4] = [
        ((168.33, 687.33), "p2 triangle"),
        ((124.67, 677.33), "p10 emoji"),
        ((167.0, 659.33), "p4 math"),
        ((81.67, 624.33), "p20 cell"),
    ];
    #[cfg(debug_assertions)]
    const TARGET_MARGIN: f64 = 12.0;

    #[cfg(debug_assertions)]
    fn bbox_contains_with_margin(b: &BoundingBox, p: (f64, f64), margin: f64) -> bool {
        p.0 >= b.left - margin
            && p.0 <= b.right + margin
            && p.1 >= b.bottom - margin
            && p.1 <= b.top + margin
    }

    let mut output = Vec::new();
    let mut stats = ComponentStats::default();
    // Track the last kept path bbox so a paint-only path can inherit it.
    let mut last_kept_path_bbox: Option<BoundingBox> = None;

    let mut iter = components.into_iter().peekable();
    while let Some(component) = iter.next() {
        match component {
            // Always keep graphics state operators
            ContentComponent::GraphicsState { operators } => {
                stats.graphics_state += 1;
                output.extend(operators);
            }

            // Filter text blocks cautiously using computed bbox
            ContentComponent::TextBlock {
                operators,
                bbox,
                estimated,
                ctm,
                render_state,
                text_matrix,
            } => {
                stats.text_blocks += 1;
                let parsed_bbox = bbox;
                let mut render_bbox = None;
                let mut used_render = false;

                // Check if the CTM contains rotation (off-diagonal elements)
                // If rotated, the parsed bbox is in rotated coordinate space,
                // but the crop box is in page space. We need to transform the crop box
                // into rotated space for comparison, OR use render fallback.
                let [a, b, c, d, _e, _f] = ctm;
                let is_rotated = b.abs() > 0.01 || c.abs() > 0.01;
                
                // Always attempt render; if it fails, keep the text.
                if let Some(fallback) = render_fallback.as_mut() {
                    let mut assembled = operators.clone();
                    let insert_at = assembled
                        .iter()
                        .position(|op| op.operator == "BT")
                        .map(|idx| idx + 1)
                        .unwrap_or(0);

                    // Build preamble with IDENTITY text matrix and pass the CTM to hayro.
                    // The text_block_text_matrix is captured before BT (when PDF spec says
                    // text matrix resets to identity), so it may contain leftover values
                    // from previous text blocks. Using identity avoids accumulation issues
                    // when the block uses Td/TD (move) operators rather than Tm (set).
                    // The block's own Tm/Td/TD operators will set the correct position.
                    let identity_tm = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
                    let mut preamble =
                        build_text_preamble(render_state.as_ref(), Some(identity_tm));
                    if !preamble.is_empty() {
                        assembled.splice(insert_at..insert_at, preamble.drain(..));
                    }
                    // Pass the actual CTM so hayro transforms coordinates correctly
                    if let Some(rendered) = fallback.measure_text_bbox(&assembled, &ctm) {
                        used_render = true;
                        render_bbox = Some(rendered);
                    }
                }

                let overlaps_render = render_bbox.as_ref().map(|b| {
                    let padded = expand_bbox(b, KEEP_GUARD, 0.5);
                    has_overlap(&padded, crop_box, 0.0)
                });
                // For rotated text, the parsed bbox is in rotated space.
                // We trust the render fallback if available, otherwise keep the text.
                let overlaps_parsed = if is_rotated {
                    if used_render {
                        // Render fallback handles coordinate transformation correctly
                        None  // Don't use parsed bbox for rotated text
                    } else {
                        #[cfg(debug_assertions)]
                        eprintln!("[DEBUG] Rotated text without render fallback - keeping by default. CTM: [{:.2}, {:.2}, {:.2}, {:.2}, ...]", a, b, c, d);
                        Some(true)  // Keep rotated text if we can't render
                    }
                } else {
                    // Non-rotated text: parsed bbox is in page space, compare normally
                    parsed_bbox.as_ref().map(|b| {
                        let padded = if estimated {
                            expand_bbox(b, KEEP_GUARD, 0.5)
                        } else {
                            expand_bbox(b, KEEP_GUARD, 0.5)
                        };
                        has_overlap(&padded, crop_box, 0.0)
                    })
                };

                #[cfg(debug_assertions)]
                {
                    for (pt, label) in TARGET_POINTS {
                        let hit_parsed = parsed_bbox
                            .as_ref()
                            .map(|b| bbox_contains_with_margin(b, pt, TARGET_MARGIN))
                            .unwrap_or(false);
                        let hit_render = render_bbox
                            .as_ref()
                            .map(|b| bbox_contains_with_margin(b, pt, TARGET_MARGIN))
                            .unwrap_or(false);
                        if hit_parsed || hit_render {
                            eprintln!(
                                "[DEBUG HIT] TextBlock intersects target {} via {} bbox: parsed={:?}, render={:?}, is_rotated={}, estimated={}, used_render={}",
                                label,
                                if hit_render { "render" } else { "parsed" },
                                parsed_bbox,
                                render_bbox,
                                is_rotated,
                                estimated,
                                used_render
                            );
                        }
                    }
                }
                
                #[cfg(debug_assertions)]
                if ops_contains_keyword(&operators, "Frequency")
                    || ops_contains_keyword(&operators, "Time")
                    || ops_contains_keyword(&operators, "\\int")
                {
                    eprintln!(
                        "[DEBUG] TextBlock keyword hit: is_rotated={}, used_render={}, estimated={}, render_bbox={:?} parsed_bbox={:?} overlaps_render={:?} overlaps_parsed={:?}",
                        is_rotated, used_render, estimated, render_bbox, parsed_bbox, overlaps_render, overlaps_parsed
                    );
                }

                let keep = force_keep
                    || overlaps_render.unwrap_or(true)
                    || overlaps_parsed.unwrap_or(true)
                    || !used_render;
                #[cfg(debug_assertions)]
                if !keep {
                    eprintln!(
                        "[DEBUG DROP] TextBlock dropped: estimated={} is_rotated={} parsed_bbox={:?} render_bbox={:?} overlaps_parsed={:?} overlaps_render={:?}",
                        estimated,
                        is_rotated,
                        parsed_bbox,
                        render_bbox,
                        overlaps_parsed,
                        overlaps_render
                    );
                }
                #[cfg(debug_assertions)]
                {
                    // Debug very small text blocks that might be math symbols
                    if let Some(b) = parsed_bbox.as_ref() {
                        if b.left >= 160.0 && b.left <= 170.0 && b.bottom >= 650.0 && b.bottom <= 660.0 {
                            eprintln!("[DEBUG] Small TextBlock near math equation area: bbox={:?}, is_rotated={}, keep={}", b, is_rotated, keep);
                            eprintln!("[DEBUG] TextBlock operations ({}):", operators.len());
                            for (i, op) in operators.iter().enumerate() {
                                if i < 10 { // Limit to first 10 ops
                                    eprintln!("[DEBUG]   {}: {}", i, op.operator);
                                }
                            }
                        }
                    }
                }
                
                if keep {
                    output.extend(operators);
                } else {
                    // When dropping a TextBlock, preserve graphics state operators that were
                    // embedded in it (color changes, etc.), because subsequent drawing operations
                    // may depend on these state changes. Same logic as for dropped Paths.
                    // IMPORTANT: Also preserve "Tf" (set font) because subsequent text blocks
                    // may rely on the font that was set in a dropped block.
                    for op in operators {
                        match op.operator.as_str() {
                            "CS" | "cs" | "SC" | "SCN" | "sc" | "scn" | "G" | "g" | "RG" | "rg" | "K" | "k"
                            | "w" | "J" | "j" | "M" | "d" | "ri" | "i" | "gs" | "Tf" => {
                                output.push(op);
                            }
                            _ => {}
                        }
                    }
                }
            }

            // Filter Form XObjects based on bbox (now with proper transformation)
            // Filter Form XObjects based on bbox
            ContentComponent::FormXObject {
                operator,
                bbox,
                ctm,
            } => {
                stats.form_xobjects += 1;
                
                // ALWAYS keep Form XObjects.
                // 1. Their internal content is recursively filtered, so privacy is preserved.
                // 2. Their BBox in the dictionary might be wrong or missing, leading to false negatives.
                // 3. They might contain content outside their BBox (invalid but possible).
                // 4. The user reported issues with missing labels that live inside Form XObjects.
                output.push(operator);
            }

            // Filter paths based on bbox overlap
            ContentComponent::Path {
                operators,
                bbox,
                is_clipping,
                ctm,
                line_width,
            } => {
                // Merge with a following paint-only Path if present so stroke/fill isn't separated from geometry.
                let mut operators = operators;
                let mut bbox = bbox;
                if !has_paint(&operators) {
                    if let Some(ContentComponent::Path {
                        operators: next_ops,
                        bbox: next_bbox,
                        is_clipping: next_clip,
                        ..
                    }) = iter.peek()
                    {
                        if has_paint(next_ops) && *next_clip == is_clipping {
                            let mut next_ops = next_ops.clone();
                            operators.append(&mut next_ops);
                            if bbox.is_none() {
                                bbox = *next_bbox;
                            }
                            iter.next();
                        }
                    }
                }
                stats.paths_total += 1;
                let parsed_bbox = bbox;
                let has_geometry = operators.iter().any(|op| {
                    matches!(
                        op.operator.as_str(),
                        "m" | "l" | "c" | "v" | "y" | "re" | "h"
                    )
                });
                let mut effective_bbox = parsed_bbox;
                let mut render_bbox = None;
                let mut used_render = false;
                let has_paint_only = has_paint(&operators) && !has_geometry;

                // Check for rotation in CTM (same issue as text)
                let [a, b, c, d, _e, _f] = ctm;
                let is_rotated = b.abs() > 0.01 || c.abs() > 0.01;

                let stroke_scale = {
                    let scale_x = (a * a + b * b).sqrt();
                    let scale_y = (c * c + d * d).sqrt();
                    (scale_x + scale_y).max(f64::EPSILON) * 0.5
                };
                let stroke_pad = line_width * stroke_scale * 0.5;

                if let Some(fallback) = render_fallback.as_mut() {
                    let mut assembled = Vec::new();
                    if line_width > 0.0 {
                        assembled.push(Operation::new(
                            "w",
                            vec![Object::Real(line_width as f32)],
                        ));
                    }
                    assembled.extend(operators.clone());
                    if let Some(rendered) = fallback.measure_ops_bbox(&assembled, &ctm) {
                        render_bbox = Some(rendered);
                        used_render = true;
                    }
                }

                // Paint-only paths sometimes rely on the current path; if we have no bbox and no
                // geometry, inherit the last kept path's bbox so the paint isn't discarded.
                if effective_bbox.is_none() && has_paint_only {
                    effective_bbox = last_kept_path_bbox;
                }

                let overlaps_render = render_bbox.as_ref().map(|b| {
                    let padded = expand_bbox(b, KEEP_GUARD, 0.5);
                    has_overlap(&padded, crop_box, 0.0)
                });
                let overlaps_special_parsed = false;
                let overlaps_special_render = false;
                
                // For rotated paths, ignore parsed bbox (wrong coordinate space)
                let overlaps_parsed = if is_rotated {
                    if used_render {
                        None // Trust render fallback for rotated paths
                    } else {
                        Some(true) // Keep rotated paths without render
                    }
                } else {
                    effective_bbox.as_ref().map(|b| {
                        let padded = expand_bbox(b, KEEP_GUARD + stroke_pad, 0.5);
                        has_overlap(&padded, crop_box, 0.0)
                    })
                };

                // Force keep small paths (likely arrowheads or dots)
                let is_small = if let Some(b) = effective_bbox.as_ref() {
                    (b.right - b.left).abs() < 10.0 && (b.top - b.bottom).abs() < 10.0
                } else {
                    false
                };

                // For clipping paths, check if they overlap the crop box
                // Only keep clipping paths that affect visible content
                let clip_overlaps = if is_clipping {
                    // Use render bbox if available, otherwise parsed bbox
                    let clip_bbox = render_bbox.or(effective_bbox);
                    clip_bbox
                        .as_ref()
                        .map(|b| {
                            let padded = expand_bbox(b, CLIP_MARGIN, 0.5);
                            has_overlap(&padded, crop_box, 0.0)
                        })
                        .unwrap_or(true) // If no bbox, conservatively keep
                } else {
                    false
                };

                let keep = clip_overlaps // Keep clipping paths that overlap crop box
                    || is_small // Force keep small paths
                    || overlaps_render.unwrap_or(true)
                    || overlaps_parsed.unwrap_or(true)
                    || has_paint_only // Preserve paint-only ops to keep prior geometry visible
                    || !used_render
                    || force_keep; // If we couldn't render, keep it (conservative)

                #[cfg(debug_assertions)]
                {
                    for (pt, label) in TARGET_POINTS {
                        let hit_parsed = effective_bbox
                            .as_ref()
                            .map(|b| bbox_contains_with_margin(b, pt, TARGET_MARGIN))
                            .unwrap_or(false);
                        let hit_render = render_bbox
                            .as_ref()
                            .map(|b| bbox_contains_with_margin(b, pt, TARGET_MARGIN))
                            .unwrap_or(false);
                        if hit_parsed || hit_render {
                            eprintln!(
                                "[DEBUG HIT] Path intersects target {} via {} bbox: parsed={:?} render={:?} line_width={:.2} stroke_pad={:.2} is_rotated={}",
                                label,
                                if hit_render { "render" } else { "parsed" },
                                effective_bbox,
                                render_bbox,
                                line_width,
                                stroke_pad,
                                is_rotated
                            );
                        }
                    }
                    // Debug paths near math equation area
                    if let Some(b) = parsed_bbox.as_ref() {
                        if b.left >= 160.0 && b.left <= 175.0 && b.bottom >= 650.0 && b.bottom <= 665.0 {
                            eprintln!("[DEBUG] Path near math equation area: bbox={:?}, is_small={}, is_rotated={}, keep={}", b, is_small, is_rotated, keep);
                            eprintln!("[DEBUG]   overlaps_render={:?}, overlaps_parsed={:?}, used_render={}", overlaps_render, overlaps_parsed, used_render);
                        }
                    }
                }

                if keep {
                    stats.paths_kept += 1;
                    output.extend(operators);
                } else {
                    #[cfg(debug_assertions)]
                    {
                        eprintln!(
                            "[DEBUG DROP] Path dropped: force_keep={} is_clipping={} clip_overlaps={} is_small={} is_rotated={} used_render={} parsed_bbox={:?} render_bbox={:?} overlaps_parsed={:?} overlaps_render={:?}",
                            force_keep,
                            is_clipping,
                            clip_overlaps,
                            is_small,
                            is_rotated,
                            used_render,
                            parsed_bbox,
                            render_bbox,
                            overlaps_parsed,
                            overlaps_render
                        );
                    }
                    // When dropping a path, we MUST preserve graphics state operators that were
                    // embedded in it (e.g. color changes), because subsequent drawing operations
                    // may depend on these state changes. Not preserving them causes subsequent
                    // content to get wrong colors (e.g. cells turning black instead of gray).
                    for op in operators {
                        match op.operator.as_str() {
                            "CS" | "cs" | "SC" | "SCN" | "sc" | "scn" | "G" | "g" | "RG" | "rg" | "K" | "k"
                            | "w" | "J" | "j" | "M" | "d" | "ri" | "i" | "gs" => {
                                output.push(op);
                            }
                            _ => {}
                        }
                    }
                }
                // Remember the bbox of the last kept path so paint-only paths can reuse it.
                if keep {
                    if let Some(b) = render_bbox.or(parsed_bbox) {
                        last_kept_path_bbox = Some(b);
                    }
                }
            }

            // Filter images based on bbox overlap
            ContentComponent::ImageXObject {
                operator,
                bbox,
                ctm,
            } => {
                stats.images_total += 1;
                let parsed_bbox = bbox;
                let mut render_bbox = None;
                let mut used_render = false;

                // Check for rotation in CTM
                let [a, b, c, d, _e, _f] = ctm;
                let is_rotated = b.abs() > 0.01 || c.abs() > 0.01;

                if let Some(fallback) = render_fallback.as_mut() {
                    if let Some(rendered) = fallback.measure_ops_bbox(&[operator.clone()], &ctm) {
                        used_render = true;
                        render_bbox = Some(rendered);
                    }
                }

                let overlaps_render = render_bbox.as_ref().map(|b| {
                    let padded = expand_bbox(b, KEEP_GUARD, 0.5);
                    has_overlap(&padded, crop_box, 0.0)
                });
                
                
                // For rotated images, ignore parsed bbox
                let overlaps_parsed = if is_rotated {
                    if used_render {
                        None // Trust render for rotated images
                    } else {
                        Some(true) // Keep rotated images without render
                    }
                } else {
                    parsed_bbox.as_ref().map(|b| {
                        let padded = expand_bbox(b, KEEP_GUARD, 0.5);
                        has_overlap(&padded, crop_box, 0.0)
                    })
                };

                // For images, if either bbox shows overlap, or we can't measure, keep it
                // Images are often critical content so we're conservative
                let keep = force_keep
                    || overlaps_render.unwrap_or(true)
                    || overlaps_parsed.unwrap_or(true)
                    || !used_render;
                #[cfg(debug_assertions)]
                {
                    for (pt, label) in TARGET_POINTS {
                        let hit_parsed = parsed_bbox
                            .as_ref()
                            .map(|b| bbox_contains_with_margin(b, pt, TARGET_MARGIN))
                            .unwrap_or(false);
                        let hit_render = render_bbox
                            .as_ref()
                            .map(|b| bbox_contains_with_margin(b, pt, TARGET_MARGIN))
                            .unwrap_or(false);
                        if hit_parsed || hit_render {
                            eprintln!(
                                "[DEBUG HIT] Image intersects target {} via {} bbox: parsed={:?} render={:?} is_rotated={}",
                                label,
                                if hit_render { "render" } else { "parsed" },
                                parsed_bbox,
                                render_bbox,
                                is_rotated
                            );
                        }
                    }
                }
                if keep {
                    stats.images_kept += 1;
                    output.push(operator);
                } else {
                    #[cfg(debug_assertions)]
                    {
                        eprintln!(
                            "[DEBUG DROP] Image dropped: is_rotated={} parsed_bbox={:?} render_bbox={:?} overlaps_parsed={:?} overlaps_render={:?}",
                            is_rotated,
                            parsed_bbox,
                            render_bbox,
                            overlaps_parsed,
                            overlaps_render
                        );
                    }
                }
            }
            ContentComponent::OrphanText {
                operator,
                bbox,
                estimated,
                ctm,
                render_state,
                text_matrix,
            } => {
                stats.orphan_text_total += 1;
                let parsed_bbox = bbox;
                let mut render_bbox = None;
                let mut used_render = false;

                if let Some(fallback) = render_fallback.as_mut() {
                    let mut assembled = Vec::new();
                    assembled.push(Operation::new("BT", vec![]));
                    let mut preamble =
                        build_text_preamble(render_state.as_ref(), Some(text_matrix));
                    assembled.append(&mut preamble);
                    assembled.push(operator.clone());
                    assembled.push(Operation::new("ET", vec![]));
                    if let Some(rendered) = fallback.measure_text_bbox(&assembled, &ctm) {
                        used_render = true;
                        render_bbox = Some(rendered);
                    }
                }

                let overlaps_render = render_bbox.as_ref().map(|b| {
                    let padded = expand_bbox(b, KEEP_GUARD, 0.5);
                    has_overlap(&padded, crop_box, 0.0)
                });
                let overlaps_parsed = parsed_bbox.as_ref().map(|b| {
                    let padded = if estimated {
                        expand_bbox(b, KEEP_GUARD, 0.5)
                    } else {
                        expand_bbox(b, KEEP_GUARD, 0.5)
                    };
                    has_overlap(&padded, crop_box, 0.0)
                });
                let overlaps_special_parsed = false;
                let overlaps_special_render = false;

                #[cfg(debug_assertions)]
                {
                    for (pt, label) in TARGET_POINTS {
                        let hit_parsed = parsed_bbox
                            .as_ref()
                            .map(|b| bbox_contains_with_margin(b, pt, TARGET_MARGIN))
                            .unwrap_or(false);
                        let hit_render = render_bbox
                            .as_ref()
                            .map(|b| bbox_contains_with_margin(b, pt, TARGET_MARGIN))
                            .unwrap_or(false);
                        if hit_parsed || hit_render {
                            eprintln!(
                                "[DEBUG HIT] OrphanText intersects target {} via {} bbox: parsed={:?} render={:?} estimated={} used_render={}",
                                label,
                                if hit_render { "render" } else { "parsed" },
                                parsed_bbox,
                                render_bbox,
                                estimated,
                                used_render
                            );
                        }
                    }
                }
                #[cfg(debug_assertions)]
                if ops_contains_keyword(std::slice::from_ref(&operator), "Frequency")
                    || ops_contains_keyword(std::slice::from_ref(&operator), "Time")
                    || ops_contains_keyword(std::slice::from_ref(&operator), "\\int")
                {
                    eprintln!(
                        "[DEBUG] OrphanText keyword hit: used_render={}, estimated={}, render_bbox={:?} parsed_bbox={:?} overlaps_render={:?} overlaps_parsed={:?}",
                        used_render, estimated, render_bbox, parsed_bbox, overlaps_render, overlaps_parsed
                    );
                }

                let keep = overlaps_render.unwrap_or(true)
                    || overlaps_parsed.unwrap_or(true)
                    || !used_render;
                if keep {
                    stats.orphan_text_kept += 1;
                    output.push(operator);
                } else {
                    #[cfg(debug_assertions)]
                    {
                        eprintln!(
                            "[DEBUG DROP] OrphanText dropped: estimated={} parsed_bbox={:?} render_bbox={:?} overlaps_parsed={:?} overlaps_render={:?}",
                            estimated,
                            parsed_bbox,
                            render_bbox,
                            overlaps_parsed,
                            overlaps_render
                        );
                    }
                }
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsValue;
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "[DEBUG] Component stats: {} text blocks, {} graphics state, {} form XObjects, {}/{} paths kept, {}/{} images kept, {}/{} orphan text kept",
            stats.text_blocks, stats.graphics_state, stats.form_xobjects,
            stats.paths_kept, stats.paths_total,
            stats.images_kept, stats.images_total,
            stats.orphan_text_kept, stats.orphan_text_total
        )));
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "[DEBUG] Final output: {} operators",
            output.len()
        )));
    }

    output
}

#[derive(Default)]
struct ComponentStats {
    text_blocks: usize,
    graphics_state: usize,
    form_xobjects: usize,
    paths_total: usize,
    paths_kept: usize,
    images_total: usize,
    images_kept: usize,
    orphan_text_total: usize,
    orphan_text_kept: usize,
}

/// Check if two bounding boxes have any overlap (with safety margin)
fn has_overlap(component_bbox: &BoundingBox, crop_box: &BoundingBox, margin: f64) -> bool {
    let actual_margin = margin.max(0.0);
    let left = crop_box.left - actual_margin;
    let bottom = crop_box.bottom - actual_margin;
    let right = crop_box.right + actual_margin;
    let top = crop_box.top + actual_margin;

    // Check if bboxes overlap (not just touch)
    !(component_bbox.right < left
        || component_bbox.left > right
        || component_bbox.top < bottom
        || component_bbox.bottom > top)
}

/// Filter content stream to remove operations outside the crop box
///
/// This analyzes the content stream and removes drawing operations (text, paths, images)
/// that fall completely outside the specified crop box. Operations that are at least
/// partially inside the crop box are preserved.
///
/// # Arguments
/// * `doc` - The PDF document (for looking up Form XObjects)
/// * `stream` - The page content stream to filter
/// * `resources` - The page's Resources dictionary (for XObject lookup)
/// * `crop_box` - The bounding box to use for filtering
/// * `base_ctm` - Current transformation matrix to map stream coordinates into page space
///
/// # Returns
/// Tuple of (filtered_content_bytes, form_xobjects_to_filter)
/// where form_xobjects_to_filter carries XObject id, resources, and page-space CTM
pub fn filter_content_stream(
    doc: &Document,
    stream: &Stream,
    resources: Option<&Dictionary>,
    crop_box: &BoundingBox,
    base_ctm: &[f64; 6],
    render_fallback: &mut Option<TextRenderFallback>,
    force_keep: bool,
) -> Result<(Vec<u8>, Vec<FormFilterTask>)> {
    // Queue of form XObjects that need recursive filtering (with their page-space CTM)
    let form_tasks: Vec<FormFilterTask> = Vec::new();

    // Decode the content stream into operations
    // Handle both compressed and uncompressed streams
    let decoded_bytes = if stream.dict.has(b"Filter") {
        stream
            .decompressed_content()
            .map_err(|e| Error::PdfParse(format!("Failed to decompress content stream: {}", e)))?
    } else {
        // Stream is not compressed, use content directly
        stream.content.clone()
    };

    #[cfg(debug_assertions)]
    {
        // Show first few bytes of decoded content
        let preview = if decoded_bytes.len() > 50 {
            &decoded_bytes[..50]
        } else {
            &decoded_bytes
        };
        eprintln!("[DEBUG] Raw content bytes (first 50): {:?}", preview);
        // Try to interpret as ASCII
        let ascii_preview = String::from_utf8_lossy(preview);
        eprintln!("[DEBUG] As ASCII: {}", ascii_preview);
    }

    let content = match Content::decode(&decoded_bytes) {
        Ok(c) => {
            // Check if the parsed content looks suspicious (single invalid operator)
            if c.operations.len() == 1 {
                let op = &c.operations[0];
                // "x" and "H" are not valid PDF operators
                if op.operator == "x" || op.operator == "H" {
                    #[cfg(target_arch = "wasm32")]
                    {
                        use wasm_bindgen::JsValue;
                        web_sys::console::log_1(&JsValue::from_str(&format!(
                            "[WARNING] Invalid operator '{}' detected - keeping original content",
                            op.operator
                        )));
                    }

                    // Return original content unchanged
                    return Ok((decoded_bytes, form_tasks));
                }
            }
            c
        }
        Err(e) => {
            #[cfg(debug_assertions)]
            eprintln!("[DEBUG] Content::decode failed: {:?}", e);

            #[cfg(target_arch = "wasm32")]
            {
                use wasm_bindgen::JsValue;
                web_sys::console::log_1(&JsValue::from_str(&format!(
                    "[WARNING] Content::decode failed - keeping original content: {:?}",
                    e
                )));
            }

            // If parsing fails, return original content unchanged
            return Ok((decoded_bytes, form_tasks));
        }
    };

    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsValue;
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "[DEBUG] Content stream has {} operations",
            content.operations.len()
        )));

        // Log raw bytes info for debugging
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "[DEBUG] Decoded {} bytes from content stream",
            decoded_bytes.len()
        )));

        if content.operations.len() == 1 {
            let op = &content.operations[0];
            web_sys::console::log_1(&JsValue::from_str(&format!(
                "[DEBUG] Single operation: '{}' with {} operands",
                op.operator,
                op.operands.len()
            )));

            // Show raw bytes for debugging single-op streams
            if decoded_bytes.len() < 100 {
                let preview = String::from_utf8_lossy(&decoded_bytes);
                web_sys::console::log_1(&JsValue::from_str(&format!(
                    "[DEBUG] Raw content: {}",
                    preview
                )));
            }
        }
    }

    // Non-WASM debug logging
    #[cfg(not(target_arch = "wasm32"))]
    {
        eprintln!(
            "[DEBUG] Content stream has {} operations",
            content.operations.len()
        );
        if content.operations.len() == 1 {
            let op = &content.operations[0];
            eprintln!(
                "[DEBUG] Single op: '{}', operands: {:?}",
                op.operator, op.operands
            );
            if let Some(Object::Name(name)) = op.operands.first() {
                eprintln!(
                    "[DEBUG] First operand is Name: {}",
                    String::from_utf8_lossy(name)
                );
            }
        } else if !content.operations.is_empty() {
            // Check if stream starts with text operators (potential issue)
            let first_op = &content.operations[0];
            if matches!(first_op.operator.as_str(), "Tj" | "TJ" | "'" | "\"") {
                eprintln!(
                    "[WARNING] Stream starts with text operator '{}' without BT!",
                    first_op.operator
                );
                eprintln!(
                    "[WARNING] This is invalid PDF - text operators should be inside BT/ET blocks"
                );
            }

            // Count BT/ET pairs
            let bt_count = content
                .operations
                .iter()
                .filter(|op| op.operator == "BT")
                .count();
            let et_count = content
                .operations
                .iter()
                .filter(|op| op.operator == "ET")
                .count();
            let text_ops_count = content
                .operations
                .iter()
                .filter(|op| matches!(op.operator.as_str(), "Tj" | "TJ" | "'" | "\""))
                .count();

            eprintln!(
                "[DEBUG] BT: {}, ET: {}, Text ops: {}",
                bt_count, et_count, text_ops_count
            );

            if content.operations.len() <= 10 {
                // Show first few operators for small streams
                for (i, op) in content.operations.iter().take(5).enumerate() {
                    eprintln!("[DEBUG] Op[{}]: '{}'", i, op.operator);
                }
            }
        }
    }

    // NEW: Component-based filtering
    // Parse operations into filterable components
    let mut form_tasks = Vec::new();
    let components = parse_into_components(
        doc,
        &content.operations,
        resources,
        base_ctm,
        &mut form_tasks,
    )?;

    #[cfg(not(target_arch = "wasm32"))]
    {
        eprintln!("[DEBUG] Parsed into {} components", components.len());
        for (i, comp) in components.iter().enumerate() {
            match comp {
                ContentComponent::Path { bbox, .. } => {
                    if let Some(b) = bbox {
                        eprintln!(
                            "[DEBUG] Component {} (Path): bbox=({:.1},{:.1},{:.1},{:.1})",
                            i, b.left, b.bottom, b.right, b.top
                        );
                    }
                }
                ContentComponent::FormXObject { bbox, .. } => {
                    if let Some(b) = bbox {
                        eprintln!(
                            "[DEBUG] Component {} (FormXObject): bbox=({:.1},{:.1},{:.1},{:.1})",
                            i, b.left, b.bottom, b.right, b.top
                        );
                    } else {
                        eprintln!("[DEBUG] Component {} (FormXObject): no bbox", i);
                    }
                }
                ContentComponent::TextBlock {
                    operators, bbox, ..
                } => {
                    if let Some(b) = bbox {
                        eprintln!(
                            "[DEBUG] Component {} (TextBlock): {} ops, bbox=({:.1},{:.1},{:.1},{:.1})",
                            i,
                            operators.len(),
                            b.left,
                            b.bottom,
                            b.right,
                            b.top
                        );
                    } else {
                        eprintln!(
                            "[DEBUG] Component {} (TextBlock): {} ops, no bbox",
                            i,
                            operators.len()
                        );
                    }
                }
                ContentComponent::GraphicsState { operators } => {
                    eprintln!(
                        "[DEBUG] Component {} (GraphicsState): {} ops",
                        i,
                        operators.len()
                    );
                }
                _ => {}
            }
        }
    }

    // Filter components based on bbox overlap
    let filtered_ops = filter_components(components, crop_box, render_fallback, force_keep);

    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsValue;
        let original_count = content.operations.len();
        let filtered_count = filtered_ops.len();
        let removed_count = original_count.saturating_sub(filtered_count);
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "[DEBUG] Filtered to {} operations ({} removed)",
            filtered_count, removed_count
        )));

        // IMPORTANT: If nothing was filtered, return original bytes to avoid re-encoding issues
        if removed_count == 0 {
            web_sys::console::log_1(&JsValue::from_str(
                "[DEBUG] No operations removed - keeping original content stream",
            ));
            return Ok((decoded_bytes, form_tasks));
        }
    }

    // Non-WASM debug logging
    #[cfg(not(target_arch = "wasm32"))]
    {
        let original_count = content.operations.len();
        let filtered_count = filtered_ops.len();
        let removed_count = original_count.saturating_sub(filtered_count);
        eprintln!(
            "[DEBUG] Filtered to {} operations ({} removed)",
            filtered_count, removed_count
        );

        // IMPORTANT: If nothing was filtered, return original bytes to avoid re-encoding issues
        if removed_count == 0 {
            eprintln!("[DEBUG] No operations removed - keeping original content stream");
            return Ok((decoded_bytes, form_tasks));
        }
    }

    // Only re-encode if we actually filtered something
    // Encode back to bytes
    let filtered_content = Content {
        operations: filtered_ops,
    };

    let encoded = filtered_content
        .encode()
        .map_err(|e| Error::PdfParse(format!("Failed to encode content stream: {}", e)))?;

    Ok((encoded, form_tasks))
}

/// Get Form XObject ObjectId
#[allow(dead_code)]
fn get_xobject_object_id(
    _doc: &Document,
    resources: &Dictionary,
    xobj_name: &[u8],
) -> Result<ObjectId> {
    // Look up XObject in Resources
    let xobject_ref = resources.get(b"XObject")?;
    let xobject_dict = xobject_ref
        .as_dict()
        .map_err(|_| Error::PdfParse("XObject is not a dictionary".to_string()))?;

    let xobj_ref = xobject_dict
        .get(xobj_name)
        .ok()
        .and_then(|obj| obj.as_reference().ok())
        .ok_or_else(|| {
            Error::PdfParse(format!(
                "XObject {} not found in Resources",
                String::from_utf8_lossy(xobj_name)
            ))
        })?;

    Ok(xobj_ref)
}

/// Get Form XObject reference and resources for later filtering
/// Returns (ObjectId, Option<Dictionary>) if it's a Form XObject
#[allow(dead_code)]
fn get_form_xobject_ref(
    doc: &Document,
    resources: &Dictionary,
    xobj_name: &[u8],
) -> Result<(ObjectId, Option<Dictionary>)> {
    // Look up XObject in Resources
    let xobject_ref = resources.get(b"XObject")?;
    let xobject_dict = xobject_ref
        .as_dict()
        .map_err(|_| Error::PdfParse("XObject is not a dictionary".to_string()))?;

    let xobj_ref = xobject_dict
        .get(xobj_name)
        .ok()
        .and_then(|obj| obj.as_reference().ok())
        .ok_or_else(|| {
            Error::PdfParse(format!(
                "XObject {} not found in Resources",
                String::from_utf8_lossy(xobj_name)
            ))
        })?;

    // Get the XObject stream to check if it's a Form
    let xobj_stream = doc
        .get_object(xobj_ref)
        .map_err(|e| Error::PdfParse(format!("Failed to get XObject: {}", e)))?
        .as_stream()
        .map_err(|e| Error::PdfParse(format!("XObject is not a stream: {}", e)))?;

    // Check if it's a Form XObject (Subtype = Form)
    let is_form = xobj_stream
        .dict
        .get(b"Subtype")
        .ok()
        .and_then(|obj| obj.as_name().ok())
        .map(|name| name == b"Form")
        .unwrap_or(false);

    if !is_form {
        // Not a Form XObject (probably an Image), skip
        return Err(Error::PdfParse("Not a Form XObject".to_string()));
    }

    // Get Form XObject's Resources (it may have its own)
    let form_resources = xobj_stream
        .dict
        .get(b"Resources")
        .ok()
        .and_then(|obj| obj.as_dict().ok())
        .cloned();

    Ok((xobj_ref, form_resources))
}

/// Filter a Form XObject's content stream
/// This is called in the second pass after collecting all Form XObjects
pub fn filter_form_xobject(
    doc: &mut Document,
    task: FormFilterTask,
    crop_box: &BoundingBox,
    render_fallback: &mut Option<TextRenderFallback>,
) -> Result<Vec<FormFilterTask>> {
    let xobj_id = task.id;
    // Get the XObject stream (immutably first to avoid borrow conflicts)
    let xobj_stream = doc
        .get_object(xobj_id)
        .map_err(|e| Error::PdfParse(format!("Failed to get XObject: {}", e)))?
        .as_stream()
        .map_err(|e| Error::PdfParse(format!("XObject is not a stream: {}", e)))?;

    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsValue;
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "[DEBUG] Filtering Form XObject: {:?}",
            xobj_id
        )));
    }

    // Filter the Form XObject's content stream (returns nested Form XObjects)
    let (filtered_content, nested_form_xobjects) = filter_content_stream(
        doc,
        xobj_stream,
        task.resources.as_ref(),
        crop_box,
        &task.ctm,
        render_fallback,
        true, // Force keep content inside Form XObjects to prevent dropping labels/arrows due to bad bbox/CTM
    )?;

    // Update the Form XObject's content
    let xobj_stream_mut = doc
        .get_object_mut(xobj_id)
        .map_err(|e| Error::PdfParse(format!("Failed to get XObject mut: {}", e)))?
        .as_stream_mut()
        .map_err(|e| Error::PdfParse(format!("XObject is not a stream (mut): {}", e)))?;

    xobj_stream_mut.set_plain_content(filtered_content);

    // Return nested Form XObjects for recursive filtering
    Ok(nested_form_xobjects)
}

/// Filter operations based on crop box intersection
/// Collects Form XObjects for later filtering (two-pass approach)
#[allow(dead_code)]
fn filter_operations(
    doc: &Document,
    operations: &[Operation],
    resources: Option<&Dictionary>,
    crop_box: &BoundingBox,
) -> Result<(Vec<Operation>, Vec<(ObjectId, Option<Dictionary>)>)> {
    let mut filtered = vec![];
    let mut form_xobjects: Vec<(ObjectId, Option<Dictionary>)> = vec![];
    let mut state = GraphicsState::default();
    let mut state_stack: Vec<GraphicsState> = vec![];
    let mut current_path: Vec<(f64, f64)> = vec![];
    let mut path_start = (0.0, 0.0);
    let mut path_ops_buffer: Vec<Operation> = vec![]; // Buffer for path construction operators

    // Note: We don't add a clipping path here - we just filter operations
    // The CropBox will handle visual cropping

    for op in operations {
        let operator = op.operator.as_str();
        let should_keep = match operator {
            // Graphics state operators - always keep
            "q" => {
                state_stack.push(state.clone());
                true
            }
            "Q" => {
                if let Some(saved_state) = state_stack.pop() {
                    state = saved_state;
                }
                true
            }
            "cm" => {
                // Transformation matrix
                if let Some(matrix) = extract_matrix(&op.operands) {
                    state.apply_transform(&matrix);
                }
                true
            }

            // Text state operators - always keep (needed for subsequent text)
            "Tf" => {
                // Font and size
                if let Some(size) = extract_number(&op.operands, 1) {
                    state.font_size = size;
                }
                true
            }
            "Tc" | "Tw" | "Tz" | "TL" | "Tr" | "Ts" => true, // Text rendering params

            // Text positioning operators
            "Td" | "TD" => {
                if let (Some(tx), Some(ty)) = (
                    extract_number(&op.operands, 0),
                    extract_number(&op.operands, 1),
                ) {
                    state.text_pos = (state.text_pos.0 + tx, state.text_pos.1 + ty);
                }
                true
            }
            "Tm" => {
                if let Some(matrix) = extract_matrix(&op.operands) {
                    state.text_matrix = matrix;
                    state.text_pos = (matrix[4], matrix[5]);
                }
                true
            }
            "T*" => true,

            // Text block operators
            "BT" => {
                state.text_matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
                state.text_pos = (0.0, 0.0);
                true
            }
            "ET" => true,

            // Text showing operators - always keep (too risky to filter)
            // Text extent is hard to calculate (depends on font metrics, rotation, etc.)
            // Better to keep all text than accidentally clip user's content
            "Tj" | "TJ" | "'" | "\"" => true,

            // Path construction operators - buffer them for now
            "m" => {
                // Move to
                if let (Some(x), Some(y)) = (
                    extract_number(&op.operands, 0),
                    extract_number(&op.operands, 1),
                ) {
                    let pos = state.transform_point(x, y);
                    current_path.clear();
                    current_path.push(pos);
                    path_start = pos;
                }
                path_ops_buffer.push(op.clone());
                false // Don't add to filtered yet
            }
            "l" => {
                // Line to
                if let (Some(x), Some(y)) = (
                    extract_number(&op.operands, 0),
                    extract_number(&op.operands, 1),
                ) {
                    current_path.push(state.transform_point(x, y));
                }
                path_ops_buffer.push(op.clone());
                false // Don't add to filtered yet
            }
            "c" | "v" | "y" => {
                // Bezier curves - just track end point
                if op.operands.len() >= 2 {
                    if let (Some(x), Some(y)) = (
                        extract_number(&op.operands, op.operands.len() - 2),
                        extract_number(&op.operands, op.operands.len() - 1),
                    ) {
                        current_path.push(state.transform_point(x, y));
                    }
                }
                path_ops_buffer.push(op.clone());
                false // Don't add to filtered yet
            }
            "re" => {
                // Rectangle
                if let (Some(x), Some(y), Some(w), Some(h)) = (
                    extract_number(&op.operands, 0),
                    extract_number(&op.operands, 1),
                    extract_number(&op.operands, 2),
                    extract_number(&op.operands, 3),
                ) {
                    current_path.clear();
                    current_path.push(state.transform_point(x, y));
                    current_path.push(state.transform_point(x + w, y));
                    current_path.push(state.transform_point(x + w, y + h));
                    current_path.push(state.transform_point(x, y + h));
                }
                path_ops_buffer.push(op.clone());
                false // Don't add to filtered yet
            }
            "h" => {
                // Close path
                if !current_path.is_empty() {
                    current_path.push(path_start);
                }
                path_ops_buffer.push(op.clone());
                false // Don't add to filtered yet
            }

            // Path painting operators - check if path intersects crop box
            "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" => {
                let keep = path_intersects_box(&current_path, crop_box);
                if keep {
                    // Commit buffered path construction operators
                    filtered.append(&mut path_ops_buffer);
                    filtered.push(op.clone());
                } else {
                    // Discard buffered path construction operators
                    path_ops_buffer.clear();
                }
                current_path.clear();
                false // Already added above if needed
            }

            // Clipping operators - buffer them (they're part of the path)
            "W" | "W*" => {
                path_ops_buffer.push(op.clone());
                false // Don't add to filtered yet
            }
            "n" => {
                // End path without painting - discard buffered path ops
                current_path.clear();
                path_ops_buffer.clear();
                false // No need to keep 'n' if path was discarded
            }

            // Color operators - always keep
            "CS" | "cs" | "SC" | "SCN" | "sc" | "scn" | "G" | "g" | "RG" | "rg" | "K" | "k" => true,

            // XObject operator - collect Form XObjects for later filtering
            "Do" => {
                // Extract XObject name and collect it for second pass
                if let Some(Object::Name(xobj_name)) = op.operands.first() {
                    if let Some(resources_dict) = resources {
                        // Try to get the XObject reference
                        if let Ok((xobj_id, xobj_resources)) =
                            get_form_xobject_ref(doc, resources_dict, xobj_name)
                        {
                            form_xobjects.push((xobj_id, xobj_resources));
                        }
                    }
                }
                // Always keep Do operators
                true
            }

            // Line width and other graphics state - always keep
            "w" | "J" | "j" | "M" | "d" | "ri" | "i" | "gs" => true,

            // Marked content operators - always keep
            "BMC" | "BDC" | "EMC" | "MP" | "DP" => true,

            // Unknown operators - keep to be safe
            _ => true,
        };

        if should_keep {
            filtered.push(op.clone());
        } else {
            #[cfg(target_arch = "wasm32")]
            {
                use wasm_bindgen::JsValue;
                web_sys::console::log_1(&JsValue::from_str(&format!(
                    "[DEBUG] Filtered out: {}",
                    operator
                )));
            }

            // Non-WASM debug logging
            #[cfg(not(target_arch = "wasm32"))]
            {
                eprintln!(
                    "[DEBUG] Filtered out: {} (operands: {})",
                    operator,
                    op.operands.len()
                );
            }
        }
    }

    Ok((filtered, form_xobjects))
}

/// Create PDF operations for a rectangular clipping path
#[allow(dead_code)]
fn create_clipping_path_operations(bbox: &BoundingBox) -> Vec<lopdf::content::Operation> {
    use lopdf::content::Operation;

    vec![
        // q - Save graphics state
        Operation::new("q", vec![]),
        // x y width height re - Rectangle
        Operation::new(
            "re",
            vec![
                Object::Real(bbox.left as f32),
                Object::Real(bbox.bottom as f32),
                Object::Real(bbox.width() as f32),
                Object::Real(bbox.height() as f32),
            ],
        ),
        // W - Clip
        Operation::new("W", vec![]),
        // n - End path without painting
        Operation::new("n", vec![]),
    ]
}

/// Extract a transformation matrix from PDF operands
fn extract_matrix(operands: &[Object]) -> Option<[f64; 6]> {
    if operands.len() >= 6 {
        Some([
            extract_number(operands, 0)?,
            extract_number(operands, 1)?,
            extract_number(operands, 2)?,
            extract_number(operands, 3)?,
            extract_number(operands, 4)?,
            extract_number(operands, 5)?,
        ])
    } else {
        None
    }
}

/// Extract a numeric value from PDF operands at the given index
fn extract_number(operands: &[Object], index: usize) -> Option<f64> {
    operands.get(index).and_then(|obj| match obj {
        Object::Integer(i) => Some(*i as f64),
        Object::Real(f) => Some(*f as f64),
        _ => None,
    })
}

/// Check if a point is within or near the bounding box
#[allow(dead_code)]
fn is_point_near_box(point: (f64, f64), bbox: &BoundingBox, margin: f64) -> bool {
    let (x, y) = point;
    x >= bbox.left - margin
        && x <= bbox.right + margin
        && y >= bbox.bottom - margin
        && y <= bbox.top + margin
}

/// Check if a path intersects with the bounding box
#[allow(dead_code)]
fn path_intersects_box(path: &[(f64, f64)], bbox: &BoundingBox) -> bool {
    if path.is_empty() {
        return true; // Keep if we can't determine
    }

    // Check if any point of the path is inside or near the box
    for &(x, y) in path {
        if is_point_near_box((x, y), bbox, 10.0) {
            return true;
        }
    }

    // Compute bounding box of the path
    let min_x = path
        .iter()
        .map(|(x, _)| x)
        .fold(f64::INFINITY, |a, &b| a.min(b));
    let max_x = path
        .iter()
        .map(|(x, _)| x)
        .fold(f64::NEG_INFINITY, |a, &b| a.max(b));
    let min_y = path
        .iter()
        .map(|(_, y)| y)
        .fold(f64::INFINITY, |a, &b| a.min(b));
    let max_y = path
        .iter()
        .map(|(_, y)| y)
        .fold(f64::NEG_INFINITY, |a, &b| a.max(b));

    // Check if path bounding box intersects with crop box
    !(max_x < bbox.left || min_x > bbox.right || max_y < bbox.bottom || min_y > bbox.top)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_clipping_path() {
        let bbox = BoundingBox::new(100.0, 100.0, 500.0, 700.0).unwrap();
        let ops = create_clipping_path_operations(&bbox);
        assert_eq!(ops.len(), 4);
        assert_eq!(ops[0].operator, "q");
        assert_eq!(ops[1].operator, "re");
        assert_eq!(ops[2].operator, "W");
        assert_eq!(ops[3].operator, "n");
    }

    #[test]
    fn test_is_point_near_box() {
        let bbox = BoundingBox::new(100.0, 100.0, 500.0, 700.0).unwrap();

        // Inside
        assert!(is_point_near_box((300.0, 400.0), &bbox, 0.0));

        // Outside
        assert!(!is_point_near_box((50.0, 50.0), &bbox, 0.0));

        // Near with margin
        assert!(is_point_near_box((95.0, 100.0), &bbox, 10.0));
    }

    #[test]
    fn test_extract_number() {
        let operands = vec![Object::Integer(42), Object::Real(3.14)];
        assert_eq!(extract_number(&operands, 0), Some(42.0));
        let real_value = extract_number(&operands, 1).unwrap();
        assert!((real_value - 3.14).abs() < 1e-6);
        assert_eq!(extract_number(&operands, 2), None);
    }

    #[test]
    fn test_cubic_extrema_in_bbox() {
        // Symmetric curve bulges to y≈0.75; ensure extrema are captured
        let p0 = (0.0, 0.0);
        let p1 = (0.0, 1.0);
        let p2 = (1.0, 1.0);
        let p3 = (1.0, 0.0);

        let mut points = vec![p0];
        extend_path_with_cubic_points(&mut points, p0, p1, p2, p3);
        let bbox = calculate_path_bbox(&points).unwrap();

        assert_eq!(bbox.left, 0.0);
        assert_eq!(bbox.right, 1.0);
        assert_eq!(bbox.bottom, 0.0);
        assert!(bbox.top > 0.7 && bbox.top < 0.8);
    }
}
