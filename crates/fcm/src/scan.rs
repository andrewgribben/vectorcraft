//! Where a scan the machine saved to USB sits on the cutting mat.
//!
//! The machine's scan JPEG is not an exact 300 dpi picture of the mat: its pixels are a little
//! different in size along each axis, and its top-left pixel is not exactly the mat's origin. A
//! [`MatScanProfile`] holds those numbers for one machine. `px = px_per_mm * mm + origin_px` on each
//! axis, so a scan pixel converts to a mat position by `mm = (px - origin_px) / px_per_mm`.

/// Millimetres per document point.
pub const MM_PER_PT: f64 = 25.4 / 72.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MatScanProfile {
    pub px_per_mm_x: f64,
    pub px_per_mm_y: f64,
    /// Where the mat's 0, 0 falls in the scan, in pixels from its top-left pixel (negative: the first
    /// pixel is already a little inside the mat).
    pub origin_px_x: f64,
    pub origin_px_y: f64,
}

impl Default for MatScanProfile {
    /// Fitted to 32 cut squares in four runs on one CM900 with a 12 x 12 in mat, sheet at the four
    /// corners (0.09 mm rms in x, 0.14 mm in y, none worse than 0.28 mm). The mat's own 1 in dot grid
    /// agrees with the scale: 300 px/in across, 297.3 px/in down. Re-measure for another machine, and
    /// after running the machine's own alignment tool.
    fn default() -> Self {
        MatScanProfile { px_per_mm_x: 11.7975, px_per_mm_y: 11.7275, origin_px_x: -3.65, origin_px_y: -10.40 }
    }
}

/// Where the picture goes, in millimetres from the mat's top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScanPlacement {
    pub x_mm: f64,
    pub y_mm: f64,
    pub width_mm: f64,
    pub height_mm: f64,
}

impl MatScanProfile {
    /// Checks the numbers are usable (finite, and within a believable range of 300 dpi).
    pub fn validate(&self) -> Result<(), String> {
        for (name, v) in [("px_per_mm_x", self.px_per_mm_x), ("px_per_mm_y", self.px_per_mm_y)] {
            if !(v.is_finite() && (5.0..=24.0).contains(&v)) {
                return Err(format!("{name} must be between 5 and 24 pixels per mm, not {v}"));
            }
        }
        for (name, v) in [("origin_px_x", self.origin_px_x), ("origin_px_y", self.origin_px_y)] {
            if !(v.is_finite() && v.abs() <= 500.0) {
                return Err(format!("{name} must be within 500 pixels of zero, not {v}"));
            }
        }
        Ok(())
    }

    /// A scan pixel as a position on the mat, in mm.
    pub fn px_to_mm(&self, px: f64, py: f64) -> (f64, f64) {
        ((px - self.origin_px_x) / self.px_per_mm_x, (py - self.origin_px_y) / self.px_per_mm_y)
    }

    /// The box a `width_px` x `height_px` scan covers on the mat.
    pub fn placement(&self, width_px: u32, height_px: u32) -> ScanPlacement {
        let (x_mm, y_mm) = self.px_to_mm(0.0, 0.0);
        ScanPlacement { x_mm, y_mm, width_mm: f64::from(width_px) / self.px_per_mm_x, height_mm: f64::from(height_px) / self.px_per_mm_y }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_measured_profile_puts_the_first_pixel_just_inside_the_mat() {
        let p = MatScanProfile::default().placement(3504, 3529);
        assert!((p.x_mm - 0.31).abs() < 0.01 && (p.y_mm - 0.89).abs() < 0.01, "{p:?}");
        assert!((p.width_mm - 297.0).abs() < 0.2 && (p.height_mm - 300.9).abs() < 0.2, "{p:?}");
    }

    #[test]
    fn a_pixel_converts_to_the_mat_position_it_was_measured_at() {
        // from the calibration run: a square cut at (16.48, 17.60) mm was seen at pixel (189.8, 199.0)
        let (x, y) = MatScanProfile::default().px_to_mm(189.8, 199.0);
        assert!((x - 16.48).abs() < 0.15 && (y - 17.60).abs() < 0.3, "{x} {y}");
    }

    #[test]
    fn a_true_300_dpi_profile_is_a_straight_scale() {
        let p = MatScanProfile { px_per_mm_x: 300.0 / 25.4, px_per_mm_y: 300.0 / 25.4, origin_px_x: 0.0, origin_px_y: 0.0 };
        let b = p.placement(3000, 1500);
        assert_eq!((b.x_mm, b.y_mm), (0.0, 0.0));
        assert!((b.width_mm - 254.0).abs() < 1e-9 && (b.height_mm - 127.0).abs() < 1e-9);
    }

    #[test]
    fn rejects_unusable_profiles() {
        assert!(MatScanProfile::default().validate().is_ok());
        for bad in [
            MatScanProfile { px_per_mm_x: 0.0, ..Default::default() },
            MatScanProfile { px_per_mm_y: f64::NAN, ..Default::default() },
            MatScanProfile { px_per_mm_x: 100.0, ..Default::default() },
            MatScanProfile { origin_px_x: 1e6, ..Default::default() },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
    }
}
