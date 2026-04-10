use lopdf::{content::Content, Dictionary, Document, Object, Stream};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WritingMode {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reliability {
    Exact,
    Estimated,
}

/// Cached font metrics for calculating text bounding boxes
#[derive(Clone, Debug)]
pub struct FontMetrics {
    pub widths: HashMap<u32, f64>,
    pub default_width: f64,
    pub ascent: f64,
    pub descent: f64,
    pub is_cid: bool,
    pub bytes_per_char: usize,
    pub writing_mode: WritingMode,
    pub reliability: Reliability,
    pub cmap: Option<HashMap<Vec<u8>, u32>>,
}

impl FontMetrics {
    pub fn glyph_width(&self, code: u32) -> f64 {
        self.widths
            .get(&code)
            .copied()
            .unwrap_or(self.default_width)
    }

    pub fn fallback() -> Self {
        Self {
            widths: HashMap::new(),
            default_width: 500.0,
            ascent: 880.0,
            descent: -220.0,
            is_cid: false,
            bytes_per_char: 1,
            writing_mode: WritingMode::Horizontal,
            reliability: Reliability::Estimated,
            cmap: None,
        }
    }
}

/// Lazy font metrics cache keyed by font resource name
#[derive(Default)]
pub struct FontCache {
    cache: HashMap<Vec<u8>, FontMetrics>,
}

impl FontCache {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    pub fn get(
        &mut self,
        doc: &Document,
        resources: Option<&Dictionary>,
        font_name: &[u8],
    ) -> FontMetrics {
        if let Some(metrics) = self.cache.get(font_name) {
            return metrics.clone();
        }

        let metrics = load_font_metrics(doc, resources, font_name);
        self.cache.insert(font_name.to_vec(), metrics.clone());
        metrics
    }
}

pub fn load_font_metrics(
    doc: &Document,
    resources: Option<&Dictionary>,
    font_name: &[u8],
) -> FontMetrics {
    let font_dict = match get_font_dictionary(doc, resources, font_name) {
        Some(d) => d,
        None => return FontMetrics::fallback(),
    };
    let subtype = font_dict
        .get(b"Subtype")
        .ok()
        .and_then(|obj| obj.as_name().ok());

    match subtype {
        Some(b"Type0") => parse_type0_font(doc, &font_dict).unwrap_or_else(FontMetrics::fallback),
        Some(b"Type1") | Some(b"TrueType") => {
            parse_type1_font(doc, &font_dict).unwrap_or_else(FontMetrics::fallback)
        }
        Some(b"Type3") => parse_type3_font(doc, &font_dict).unwrap_or_else(FontMetrics::fallback),
        _ => FontMetrics::fallback(),
    }
}

fn get_font_dictionary(
    doc: &Document,
    resources: Option<&Dictionary>,
    font_name: &[u8],
) -> Option<Dictionary> {
    let resources = resources?;
    let font_entry = resources.get(b"Font").ok()?;
    let font_dict_obj = resolve_to_owned(doc, font_entry)?;
    let font_dict = font_dict_obj.as_dict().ok()?;
    let font_obj = font_dict.get(font_name).ok()?.clone();
    match resolve_to_owned(doc, &font_obj)? {
        Object::Dictionary(dict) => Some(dict),
        Object::Stream(stream) => Some(stream.dict),
        _ => None,
    }
}

fn resolve_to_owned(doc: &Document, obj: &Object) -> Option<Object> {
    match obj {
        Object::Reference(id) => doc.get_object(*id).ok().cloned(),
        other => Some(other.clone()),
    }
}

fn parse_type1_font(doc: &Document, font_dict: &Dictionary) -> Option<FontMetrics> {
    let first_char = font_dict
        .get(b"FirstChar")
        .ok()
        .and_then(|obj| obj.as_i64().ok())
        .unwrap_or(0) as u32;

    let widths_obj = font_dict.get(b"Widths").ok()?;
    let widths_array_obj = resolve_to_owned(doc, widths_obj)?;
    let widths_array = widths_array_obj.as_array().ok()?;

    let mut widths = HashMap::new();
    for (idx, value) in widths_array.iter().enumerate() {
        if let Some(width) = object_to_f64(value) {
            widths.insert(first_char + idx as u32, width);
        }
    }

    let descriptor_dict = font_dict
        .get(b"FontDescriptor")
        .ok()
        .and_then(|obj| resolve_to_owned(doc, obj))
        .and_then(|obj| match obj {
            Object::Dictionary(dict) => Some(dict),
            Object::Stream(stream) => Some(stream.dict),
            _ => None,
        });

    let (ascent, descent, missing_width) = descriptor_metrics(descriptor_dict.as_ref());

    Some(FontMetrics {
        widths,
        default_width: missing_width,
        ascent,
        descent,
        is_cid: false,
        bytes_per_char: 1,
        writing_mode: WritingMode::Horizontal,
        reliability: Reliability::Exact,
        cmap: None,
    })
}

fn parse_type0_font(doc: &Document, font_dict: &Dictionary) -> Option<FontMetrics> {
    // Detect vertical mode when possible; fall back to horizontal and keep metrics even if encoding is unknown
    let encoding_obj = font_dict.get(b"Encoding").ok()?;
    let writing_mode = match encoding_obj.as_name() {
        Ok(b"Identity-V") => WritingMode::Vertical,
        _ => WritingMode::Horizontal,
    };

    let descendant_fonts_obj = font_dict.get(b"DescendantFonts").ok()?;
    let descendant_fonts_resolved = resolve_to_owned(doc, descendant_fonts_obj)?;
    let descendant_array = descendant_fonts_resolved.as_array().ok()?;
    let first_descendant = match descendant_array.first() {
        Some(f) => f,
        None => return None,
    };
    let descendant_dict_obj = resolve_to_owned(doc, first_descendant)?;
    let descendant_dict = match descendant_dict_obj {
        Object::Dictionary(dict) => dict,
        Object::Stream(stream) => stream.dict,
        _ => return None,
    };

    let default_width = descendant_dict
        .get(b"DW")
        .ok()
        .and_then(object_to_f64)
        .unwrap_or(1000.0);

    let mut widths = HashMap::new();
    if let Ok(w_array_obj) = descendant_dict.get(b"W") {
        if let Some(resolved_w_array) = resolve_to_owned(doc, w_array_obj) {
            if let Ok(entries) = resolved_w_array.as_array() {
                parse_cid_widths(entries, &mut widths);
            }
        }
    }

    let descriptor_dict = descendant_dict
        .get(b"FontDescriptor")
        .ok()
        .and_then(|obj| resolve_to_owned(doc, obj))
        .and_then(|obj| match obj {
            Object::Dictionary(dict) => Some(dict),
            Object::Stream(stream) => Some(stream.dict),
            _ => None,
        });

    let (ascent, descent, missing_width) = descriptor_metrics(descriptor_dict.as_ref());

    let mut metrics = FontMetrics {
        widths,
        default_width: if default_width > 0.0 {
            default_width
        } else {
            missing_width
        },
        ascent,
        descent,
        is_cid: true,
        bytes_per_char: 2,
        writing_mode,
        reliability: Reliability::Estimated,
        cmap: None,
    };

    // Try to refine bytes_per_char using CMap (Encoding/ToUnicode as stream)
    if let Ok(enc_ref) = encoding_obj.as_reference() {
        if let Ok(obj) = doc.get_object(enc_ref) {
            if let Some(cmap_stream) = obj.as_stream().ok() {
                if let Some((cmap_bytes_per_char, cmap_map)) =
                    parse_cmap_bytes_per_char_and_map(cmap_stream)
                {
                    metrics.bytes_per_char = cmap_bytes_per_char.max(1);
                    metrics.cmap = Some(cmap_map);
                    metrics.reliability = Reliability::Estimated;
                }
            }
        }
    }
    if let Some(to_unicode_obj) = font_dict.get(b"ToUnicode").ok() {
        if let Ok(to_unicode_ref) = to_unicode_obj.as_reference() {
            if let Ok(obj) = doc.get_object(to_unicode_ref) {
                if let Some(cmap_stream) = obj.as_stream().ok() {
                    if let Some((cmap_bytes_per_char, cmap_map)) =
                        parse_cmap_bytes_per_char_and_map(cmap_stream)
                    {
                        metrics.bytes_per_char = cmap_bytes_per_char.max(1);
                        metrics.cmap = Some(cmap_map);
                        metrics.reliability = Reliability::Estimated;
                    }
                }
            }
        }
    }

    Some(metrics)
}

fn parse_type3_font(doc: &Document, font_dict: &Dictionary) -> Option<FontMetrics> {
    // Type3 fonts may not have Widths; fall back to FontBBox width
    let bbox_width = font_dict
        .get(b"FontBBox")
        .ok()
        .and_then(|obj| resolve_to_owned(doc, obj))
        .and_then(|obj| obj.as_array().ok().map(|arr| arr.to_vec()))
        .and_then(|vals| {
            if vals.len() == 4 {
                let left = object_to_f64(&vals[0])?;
                let right = object_to_f64(&vals[2])?;
                Some((right - left).abs())
            } else {
                None
            }
        })
        .unwrap_or(500.0);

    let mut widths = HashMap::new();
    for code in 0..=255u32 {
        widths.insert(code, bbox_width);
    }

    // Attempt to derive widths from CharProcs + Encoding
    if let Some(charprocs_obj) = font_dict.get(b"CharProcs").ok() {
        if let Some(charprocs_owned) = resolve_to_owned(doc, charprocs_obj) {
            if let Ok(charprocs_dict) = charprocs_owned.as_dict() {
                if let Some(encoding_widths) = extract_type3_widths(doc, font_dict, charprocs_dict)
                {
                    for (code, width) in encoding_widths {
                        widths.insert(code, width);
                    }
                }
            }
        }
    }

    let (ascent, descent, missing_width) = descriptor_metrics(
        font_dict
            .get(b"FontDescriptor")
            .ok()
            .and_then(|obj| {
                resolve_to_owned(doc, obj).and_then(|o| match o {
                    Object::Dictionary(dict) => Some(dict),
                    Object::Stream(stream) => Some(stream.dict),
                    _ => None,
                })
            })
            .as_ref(),
    );

    Some(FontMetrics {
        widths,
        default_width: missing_width,
        ascent,
        descent,
        is_cid: false,
        bytes_per_char: 1,
        writing_mode: WritingMode::Horizontal,
        reliability: Reliability::Estimated,
        cmap: None,
    })
}

fn parse_cid_widths(entries: &[Object], widths: &mut HashMap<u32, f64>) {
    let mut idx = 0;
    while idx < entries.len() {
        match &entries[idx] {
            Object::Array(arr) => {
                // Format: cFirst [w1 w2 ...]
                if idx + 1 >= entries.len() {
                    break;
                }
                let start_code = object_to_u32(&entries[idx]);
                let widths_array = arr;
                if let Some(start) = start_code {
                    for (i, w) in widths_array.iter().enumerate() {
                        if let Some(width) = object_to_f64(w) {
                            widths.insert(start + i as u32, width);
                        }
                    }
                }
                idx += 2;
            }
            Object::Integer(_) | Object::Real(_) => {
                if idx + 2 >= entries.len() {
                    break;
                }
                let end_code = match object_to_u32(&entries[idx + 1]) {
                    Some(val) => val,
                    None => {
                        idx += 1;
                        continue;
                    }
                };
                if let Some(width) = object_to_f64(&entries[idx + 2]) {
                    for code in object_to_u32(&entries[idx]).unwrap_or(0)..=end_code {
                        widths.insert(code, width);
                    }
                }
                idx += 3;
            }
            _ => {
                idx += 1;
            }
        }
    }
}

/// Parse CMap codespacerange and bfchar/bfrange to derive bytes_per_char and a code map
fn parse_cmap_bytes_per_char_and_map(stream: &Stream) -> Option<(usize, HashMap<Vec<u8>, u32>)> {
    let data = stream.decompressed_content().ok()?;
    let text = String::from_utf8_lossy(&data);
    let mut max_bytes = 0usize;
    let mut map: HashMap<Vec<u8>, u32> = HashMap::new();

    enum Mode {
        None,
        CodeSpace,
        BfChar,
        BfRange,
    }

    let mut mode = Mode::None;
    for line in text.lines() {
        let trimmed = line.trim();
        match trimmed {
            t if t.ends_with("begincodespacerange") => {
                mode = Mode::CodeSpace;
                continue;
            }
            t if t.ends_with("endcodespacerange") => {
                mode = Mode::None;
                continue;
            }
            t if t.ends_with("beginbfchar") => {
                mode = Mode::BfChar;
                continue;
            }
            t if t.ends_with("endbfchar") => {
                mode = Mode::None;
                continue;
            }
            t if t.ends_with("beginbfrange") => {
                mode = Mode::BfRange;
                continue;
            }
            t if t.ends_with("endbfrange") => {
                mode = Mode::None;
                continue;
            }
            _ => {}
        }

        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        match mode {
            Mode::CodeSpace => {
                if parts.len() >= 2 {
                    let left = parts[0].trim_matches(|c| c == '<' || c == '>');
                    let bytes = left.len() / 2;
                    max_bytes = max_bytes.max(bytes);
                }
            }
            Mode::BfChar => {
                if parts.len() >= 2 {
                    let src_bytes = hex_to_bytes(parts[0].trim_matches(|c| c == '<' || c == '>'));
                    let dst_hex = parts[1].trim_matches(|c| c == '<' || c == '>');
                    let dst_val = u32::from_str_radix(dst_hex, 16).unwrap_or(0);
                    max_bytes = max_bytes.max(src_bytes.len());
                    map.insert(src_bytes, dst_val);
                }
            }
            Mode::BfRange => {
                if parts.len() >= 3 {
                    let start_bytes = hex_to_bytes(parts[0].trim_matches(|c| c == '<' || c == '>'));
                    let end_bytes = hex_to_bytes(parts[1].trim_matches(|c| c == '<' || c == '>'));
                    max_bytes = max_bytes.max(start_bytes.len().max(end_bytes.len()));

                    // Third part may be a single destination start or an array
                    if parts[2].starts_with('<') {
                        let mut dst = u32::from_str_radix(
                            parts[2].trim_matches(|c| c == '<' || c == '>'),
                            16,
                        )
                        .unwrap_or(0);
                        let mut current = start_bytes.clone();
                        while current <= end_bytes {
                            map.insert(current.clone(), dst);
                            dst = dst.saturating_add(1);
                            current = increment_bytes(&current);
                        }
                    } else if parts[2].starts_with('[') {
                        let mut dsts = Vec::new();
                        for tok in &parts[2..] {
                            let cleaned =
                                tok.trim_matches(|c| c == '<' || c == '>' || c == '[' || c == ']');
                            if cleaned.is_empty() {
                                continue;
                            }
                            if let Ok(val) = u32::from_str_radix(cleaned, 16) {
                                dsts.push(val);
                            }
                            if tok.contains(']') {
                                break;
                            }
                        }
                        let mut current = start_bytes.clone();
                        for dst in dsts {
                            map.insert(current.clone(), dst);
                            if current == end_bytes {
                                break;
                            }
                            current = increment_bytes(&current);
                        }
                    }
                }
            }
            Mode::None => {}
        }
    }

    if max_bytes > 0 {
        Some((max_bytes, map))
    } else {
        None
    }
}

fn hex_to_bytes(hex: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    for pair in hex.as_bytes().chunks(2) {
        if pair.len() == 2 {
            if let Ok(val) = u8::from_str_radix(std::str::from_utf8(pair).unwrap_or(""), 16) {
                bytes.push(val);
            }
        }
    }
    bytes
}

fn increment_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    for i in (0..out.len()).rev() {
        if out[i] == 0xFF {
            out[i] = 0x00;
        } else {
            out[i] = out[i].saturating_add(1);
            break;
        }
    }
    out
}

/// Extract Type3 widths from CharProcs using the Encoding array/dict
fn extract_type3_widths(
    doc: &Document,
    font_dict: &Dictionary,
    charprocs: &Dictionary,
) -> Option<Vec<(u32, f64)>> {
    // Only handle Encoding as array (code -> name)
    let encoding = font_dict.get(b"Encoding").ok()?;
    let encoding_array = match resolve_to_owned(doc, encoding) {
        Some(Object::Array(arr)) => arr,
        _ => return None,
    };

    let mut result = Vec::new();
    for (code, name_obj) in encoding_array.iter().enumerate() {
        if let Object::Name(name) = name_obj {
            if let Ok(proc_obj) = charprocs.get(name) {
                if let Some(resolved) = resolve_to_owned(doc, proc_obj) {
                    if let Ok(proc_stream) = resolved.as_stream() {
                        if let Some(width) = parse_charproc_width(proc_stream) {
                            result.push((code as u32, width));
                        }
                    }
                }
            }
        }
    }
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

/// Parse Type3 CharProc stream to extract width (d0/d1 operators)
fn parse_charproc_width(stream: &Stream) -> Option<f64> {
    let data = stream.decompressed_content().ok()?;
    let content = Content::decode(&data).ok()?;
    for op in content.operations {
        match op.operator.as_str() {
            "d0" => {
                if let (Some(w0), Some(_)) = (
                    op.operands.get(0).and_then(object_to_f64),
                    op.operands.get(1).and_then(object_to_f64),
                ) {
                    return Some(w0);
                }
            }
            "d1" => {
                if let Some(w0) = op.operands.get(0).and_then(object_to_f64) {
                    return Some(w0);
                }
            }
            _ => {}
        }
    }
    None
}

fn object_to_f64(obj: &Object) -> Option<f64> {
    match obj {
        Object::Real(val) => Some(*val as f64),
        Object::Integer(val) => Some(*val as f64),
        _ => None,
    }
}

fn object_to_u32(obj: &Object) -> Option<u32> {
    match obj {
        Object::Integer(val) => {
            if *val >= 0 {
                Some(*val as u32)
            } else {
                None
            }
        }
        Object::Real(val) => {
            if *val >= 0.0 {
                Some(*val as u32)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn descriptor_metrics(descriptor_dict: Option<&Dictionary>) -> (f64, f64, f64) {
    let ascent = descriptor_dict
        .and_then(|d| d.get(b"Ascent").ok())
        .and_then(|o| o.as_f32().ok())
        .unwrap_or(880.0) as f64;
    let descent = descriptor_dict
        .and_then(|d| d.get(b"Descent").ok())
        .and_then(|o| o.as_f32().ok())
        .unwrap_or(-220.0) as f64;
    let missing_width = descriptor_dict
        .and_then(|d| d.get(b"MissingWidth").ok())
        .and_then(|o| o.as_f32().ok())
        .unwrap_or(500.0) as f64;
    (ascent, descent, missing_width)
}
