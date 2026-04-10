use crate::bbox::BoundingBox;
use anyhow::{anyhow, Result};
use hayro::{InterpreterSettings, Pdf as HayroPdf};
use hayro_interpret::font::Glyph;
use hayro_interpret::hayro_syntax::content::TypedIter;
use hayro_interpret::{interpret, ClipPath, Context, Device};
use lopdf::content::{Content, Operation};
use std::sync::Arc;
use vello_cpu::kurbo::{Affine, BezPath, Rect, Shape};

/// Lightweight renderer that replays text operations into a bbox-collecting device.
pub struct TextRenderFallback {
    pdf_data: Arc<Vec<u8>>,
    pdf: HayroPdf,
    settings: InterpreterSettings,
    page_index: usize,
    viewport: Rect,
}

impl TextRenderFallback {
    pub fn new(pdf_bytes: Vec<u8>, page_index: usize) -> Result<Self> {
        let data = Arc::new(pdf_bytes);
        let pdf = HayroPdf::new(data.clone()).map_err(|e| anyhow!("{:?}", e))?;
        Ok(Self {
            pdf_data: data,
            pdf,
            settings: InterpreterSettings::default(),
            page_index,
            viewport: Rect::new(-10_000.0, -10_000.0, 10_000.0, 10_000.0),
        })
    }

    /// Render the provided operators using hayro and return their page-space bbox.
    pub fn measure_text_bbox(&self, ops: &[Operation], ctm: &[f64; 6]) -> Option<BoundingBox> {
        self.measure_ops_bbox(ops, ctm)
    }

    /// Render arbitrary operators and return their page-space bbox.
    pub fn measure_ops_bbox(&self, ops: &[Operation], ctm: &[f64; 6]) -> Option<BoundingBox> {
        let page = self.pdf.pages().get(self.page_index)?;

        // Encode lopdf operations back into a content stream hayro can interpret.
        let content = Content {
            operations: ops.to_vec(),
        };
        let encoded = content.encode().ok()?;

        let mut context = Context::new(
            Affine::new(*ctm),
            self.viewport,
            page.xref(),
            self.settings.clone(),
        );
        let mut device = BBoxDevice::default();

        interpret(
            TypedIter::new(&encoded),
            page.resources(),
            &mut context,
            &mut device,
        );

        device.into_bbox()
    }

    /// Keep the underlying PDF data alive for as long as the fallback exists.
    pub fn pdf_data(&self) -> Arc<Vec<u8>> {
        self.pdf_data.clone()
    }
}

#[derive(Default)]
struct BBoxDevice {
    bbox: Option<Rect>,
}

impl BBoxDevice {
    fn record_rect(&mut self, rect: Rect) {
        if !rect.is_finite() || rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        self.bbox = Some(match self.bbox {
            Some(existing) => existing.union(rect),
            None => rect,
        });
    }

    fn into_bbox(self) -> Option<BoundingBox> {
        let rect = self.bbox?;
        BoundingBox::new(rect.x0, rect.y0, rect.x1, rect.y1).ok()
    }
}

impl<'a> Device<'a> for BBoxDevice {
    fn set_soft_mask(&mut self, _mask: Option<hayro_interpret::SoftMask<'a>>) {}

    fn draw_path(
        &mut self,
        path: &BezPath,
        transform: Affine,
        _paint: &hayro_interpret::Paint<'a>,
        draw_mode: &hayro_interpret::PathDrawMode,
    ) {
        let base_rect = path.bounding_box();
        let inflated = match draw_mode {
            hayro_interpret::PathDrawMode::Fill(_) => base_rect,
            hayro_interpret::PathDrawMode::Stroke(props) => {
                base_rect.inflate(props.line_width as f64 * 0.5, props.line_width as f64 * 0.5)
            }
        };
        let transformed = (transform * inflated.to_path(0.1)).bounding_box();
        self.record_rect(transformed);
    }

    fn push_clip_path(&mut self, _clip_path: &ClipPath) {}

    fn push_transparency_group(
        &mut self,
        _opacity: f32,
        _mask: Option<hayro_interpret::SoftMask<'a>>,
    ) {
    }

    fn draw_glyph(
        &mut self,
        glyph: &Glyph<'a>,
        transform: Affine,
        glyph_transform: Affine,
        paint: &hayro_interpret::Paint<'a>,
        draw_mode: &hayro_interpret::GlyphDrawMode,
    ) {
        match glyph {
            Glyph::Outline(outline) => {
                let path = outline.outline();
                let base_rect = match draw_mode {
                    hayro_interpret::GlyphDrawMode::Fill => path.bounding_box(),
                    hayro_interpret::GlyphDrawMode::Stroke(props) => path
                        .bounding_box()
                        .inflate(props.line_width as f64 * 0.5, props.line_width as f64 * 0.5),
                };
                let glyph_affine = transform * glyph_transform;
                let transformed = (glyph_affine * base_rect.to_path(0.1)).bounding_box();
                self.record_rect(transformed);
            }
            Glyph::Type3(shape) => {
                // Interpret the Type3 glyph, letting nested drawing update the bbox.
                shape.interpret(self, transform, glyph_transform, paint);
            }
        }
    }

    fn draw_image(&mut self, image: hayro_interpret::Image<'a, '_>, transform: Affine) {
        match image {
            hayro_interpret::Image::Stencil(stencil) => {
                stencil.with_stencil(|luma, _| {
                    let rect = Rect::new(0.0, 0.0, luma.width as f64, luma.height as f64);
                    let transformed = (transform * rect.to_path(0.1)).bounding_box();
                    self.record_rect(transformed);
                });
            }
            hayro_interpret::Image::Raster(raster) => {
                raster.with_rgba(|rgb, _| {
                    let rect = Rect::new(0.0, 0.0, rgb.width as f64, rgb.height as f64);
                    let transformed = (transform * rect.to_path(0.1)).bounding_box();
                    self.record_rect(transformed);
                });
            }
        }
    }

    fn pop_clip_path(&mut self) {}

    fn pop_transparency_group(&mut self) {}
}
