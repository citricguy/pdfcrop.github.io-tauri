use crate::bbox::BoundingBox;

pub fn expand_bbox(bbox: &BoundingBox, margin_pt: f64, margin_pct: f64) -> BoundingBox {
    let width = bbox.width();
    let height = bbox.height();
    let extra_w = width * margin_pct;
    let extra_h = height * margin_pct;
    let margin_left = margin_pt + extra_w;
    let margin_right = margin_pt + extra_w;
    let margin_top = margin_pt + extra_h;
    let margin_bottom = margin_pt + extra_h;

    BoundingBox::new(
        bbox.left - margin_left,
        bbox.bottom - margin_bottom,
        bbox.right + margin_right,
        bbox.top + margin_top,
    )
    .unwrap_or(*bbox)
}
