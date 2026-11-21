// Salah
//
// See LICENSE for more details.
// Copyright (c) 2019-2022 Farhan Ahmed. All rights reserved.
//

#[cfg(not(feature = "std"))]
#[allow(unused_imports)]
use crate::math::Float;
use core::f64::consts::PI;
use core::ops::{Add, Div, Mul, Sub};

pub trait Normalize {
    fn normalized_to_scale(&self, max: f64) -> f64;
}

impl Normalize for f64 {
    /// In [0, max) for a positive max, (max, 0] for a negative one, NaN
    /// for NaN.
    fn normalized_to_scale(&self, max: f64) -> f64 {
        let normalized = self - (max * (self / max).floor());
        // A tiny negative value rounds up to max itself.
        if normalized.abs() >= max.abs() {
            0.0
        } else {
            normalized
        }
    }
}

#[derive(PartialEq, Debug, Copy, Clone)]
pub struct Angle {
    pub degrees: f64,
}

impl Angle {
    pub fn new(value: f64) -> Self {
        Angle { degrees: value }
    }

    pub fn from_radians(value: f64) -> Self {
        Angle {
            degrees: (value * 180.0) / PI,
        }
    }

    pub fn radians(&self) -> f64 {
        (self.degrees * PI) / 180.0
    }

    pub fn unwound(&self) -> Angle {
        Angle {
            degrees: self.degrees.normalized_to_scale(360.0),
        }
    }

    /// The same direction in [-180, 180].
    pub fn quadrant_shifted(&self) -> Angle {
        if (-180.0..=180.0).contains(&self.degrees) {
            *self
        } else {
            Angle {
                degrees: self.degrees - (360.0 * (self.degrees / 360.0).round()),
            }
        }
    }
}

impl Add for Angle {
    type Output = Angle;

    fn add(self, rhs: Angle) -> Angle {
        Angle {
            degrees: self.degrees + rhs.degrees,
        }
    }
}

impl Sub for Angle {
    type Output = Angle;

    fn sub(self, rhs: Angle) -> Angle {
        Angle {
            degrees: self.degrees - rhs.degrees,
        }
    }
}

impl Mul for Angle {
    type Output = Angle;

    fn mul(self, rhs: Angle) -> Angle {
        Angle {
            degrees: self.degrees * rhs.degrees,
        }
    }
}

impl Div for Angle {
    type Output = Angle;

    fn div(self, rhs: Angle) -> Angle {
        if rhs.degrees == 0.0 {
            panic!("Cannot divide by zero.");
        }

        Angle {
            degrees: self.degrees / rhs.degrees,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn angle_conversion_from_radians() {
        assert_eq!(Angle::from_radians(PI).degrees, 180.0);
        assert_eq!(Angle::from_radians(PI / 2.0).degrees, 90.0);
    }

    #[test]
    fn angle_conversion_degrees_to_radians() {
        assert_eq!(Angle::new(180.0).radians(), PI);
        assert_eq!(Angle::new(90.0).radians(), PI / 2.0);
    }

    #[test]
    fn normalize_value() {
        assert_eq!(2.0_f64.normalized_to_scale(-5.0), -3.0);
        assert_eq!((-4.0_f64).normalized_to_scale(-5.0), -4.0);
        assert_eq!((-6.0_f64).normalized_to_scale(-5.0), -1.0);

        assert_eq!((-1.0_f64).normalized_to_scale(24.0), 23.0);
        assert_eq!(1.0_f64.normalized_to_scale(24.0), 1.0);
        assert_eq!(49.0_f64.normalized_to_scale(24.0), 1.0);

        assert_eq!(361.0_f64.normalized_to_scale(360.0), 1.0);
        assert_eq!(360.0_f64.normalized_to_scale(360.0), 0.0);
        assert_eq!(259.0_f64.normalized_to_scale(360.0), 259.0);
        assert_eq!(2592.0_f64.normalized_to_scale(360.0), 72.0);
    }

    #[test]
    fn angle_unwound() {
        assert_eq!(Angle::new(-45.0).unwound().degrees, 315.0);
        assert_eq!(Angle::new(361.0).unwound().degrees, 1.0);
        assert_eq!(Angle::new(360.0).unwound().degrees, 0.0);
        assert_eq!(Angle::new(259.0).unwound().degrees, 259.0);
        assert_eq!(Angle::new(2592.0).unwound().degrees, 72.0);
    }

    #[test]
    fn unwound_stays_below_360() {
        // salah 0.7 unwound -1e-17° to 360°, a Qiblah bearing out of range.
        assert_eq!(Angle::new(-1e-17).unwound().degrees, 0.0);
        assert_eq!((-1e-17_f64).normalized_to_scale(1.0), 0.0);
    }

    #[test]
    fn closest_angle() {
        assert_eq!(Angle::new(360.0).quadrant_shifted().degrees, 0.0);
        assert_eq!(Angle::new(361.0).quadrant_shifted().degrees, 1.0);
        assert_eq!(Angle::new(1.0).quadrant_shifted().degrees, 1.0);
        assert_eq!(Angle::new(-1.0).quadrant_shifted().degrees, -1.0);
        assert_eq!(Angle::new(-181.0).quadrant_shifted().degrees, 179.0);
        assert_eq!(Angle::new(180.0).quadrant_shifted().degrees, 180.0);
        assert_eq!(Angle::new(359.0).quadrant_shifted().degrees, -1.0);
        assert_eq!(Angle::new(-359.0).quadrant_shifted().degrees, 1.0);
        assert_eq!(Angle::new(1261.0).quadrant_shifted().degrees, -179.0);
    }

    #[test]
    fn adding_angles() {
        let angle_a = Angle::new(45.0);
        let angle_b = Angle::new(45.0);

        assert_eq!((angle_a + angle_b).degrees, 90.0);
    }
}
