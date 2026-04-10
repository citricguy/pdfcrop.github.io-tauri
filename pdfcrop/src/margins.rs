//! Margin calculations and utilities

/// Margins to apply around a bounding box
///
/// PDF coordinates use points (1/72 inch) as the unit.
/// Origin is at the bottom-left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Margins {
    /// Left margin (added to the left side)
    pub left: f64,
    /// Top margin (added to the top side)
    pub top: f64,
    /// Right margin (added to the right side)
    pub right: f64,
    /// Bottom margin (added to the bottom side)
    pub bottom: f64,
}

impl Margins {
    /// Create margins with all sides set to zero
    pub fn none() -> Self {
        Self {
            left: 0.0,
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
        }
    }

    /// Create uniform margins (same value for all sides)
    pub fn uniform(value: f64) -> Self {
        Self {
            left: value,
            top: value,
            right: value,
            bottom: value,
        }
    }

    /// Create margins from individual values
    pub fn new(left: f64, top: f64, right: f64, bottom: f64) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    /// Create margins from a string specification
    ///
    /// Supports:
    /// - Single value: "10" -> all margins = 10
    /// - Two values: "10 20" -> left/right = 10, top/bottom = 20
    /// - Four values: "10 20 30 40" -> left, top, right, bottom
    pub fn from_str(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.split_whitespace().collect();

        match parts.len() {
            1 => {
                let value = parts[0]
                    .parse::<f64>()
                    .map_err(|e| format!("Invalid margin value: {}", e))?;
                Ok(Self::uniform(value))
            }
            2 => {
                let h = parts[0]
                    .parse::<f64>()
                    .map_err(|e| format!("Invalid horizontal margin: {}", e))?;
                let v = parts[1]
                    .parse::<f64>()
                    .map_err(|e| format!("Invalid vertical margin: {}", e))?;
                Ok(Self {
                    left: h,
                    top: v,
                    right: h,
                    bottom: v,
                })
            }
            4 => {
                let left = parts[0]
                    .parse::<f64>()
                    .map_err(|e| format!("Invalid left margin: {}", e))?;
                let top = parts[1]
                    .parse::<f64>()
                    .map_err(|e| format!("Invalid top margin: {}", e))?;
                let right = parts[2]
                    .parse::<f64>()
                    .map_err(|e| format!("Invalid right margin: {}", e))?;
                let bottom = parts[3]
                    .parse::<f64>()
                    .map_err(|e| format!("Invalid bottom margin: {}", e))?;
                Ok(Self::new(left, top, right, bottom))
            }
            _ => Err(format!(
                "Invalid margin specification: expected 1, 2, or 4 values, got {}",
                parts.len()
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_margins_none() {
        let m = Margins::none();
        assert_eq!(m.left, 0.0);
        assert_eq!(m.top, 0.0);
        assert_eq!(m.right, 0.0);
        assert_eq!(m.bottom, 0.0);
    }

    #[test]
    fn test_margins_uniform() {
        let m = Margins::uniform(10.0);
        assert_eq!(m.left, 10.0);
        assert_eq!(m.top, 10.0);
        assert_eq!(m.right, 10.0);
        assert_eq!(m.bottom, 10.0);
    }

    #[test]
    fn test_margins_from_str_single() {
        let m = Margins::from_str("15").unwrap();
        assert_eq!(m, Margins::uniform(15.0));
    }

    #[test]
    fn test_margins_from_str_two_values() {
        let m = Margins::from_str("10 20").unwrap();
        assert_eq!(m.left, 10.0);
        assert_eq!(m.right, 10.0);
        assert_eq!(m.top, 20.0);
        assert_eq!(m.bottom, 20.0);
    }

    #[test]
    fn test_margins_from_str_four_values() {
        let m = Margins::from_str("10 20 30 40").unwrap();
        assert_eq!(m.left, 10.0);
        assert_eq!(m.top, 20.0);
        assert_eq!(m.right, 30.0);
        assert_eq!(m.bottom, 40.0);
    }

    #[test]
    fn test_margins_from_str_invalid() {
        assert!(Margins::from_str("10 20 30").is_err());
        assert!(Margins::from_str("invalid").is_err());
    }
}
