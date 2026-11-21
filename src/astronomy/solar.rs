// Salah
//
// See LICENSE for more details.
// Copyright (c) 2019-2022 Farhan Ahmed. All rights reserved.
//

use chrono::{Datelike, NaiveDate};

use crate::astronomy::ops;
use crate::astronomy::unit::Angle;
use crate::error::Error;
use crate::models::coordinates::Coordinates;

#[derive(PartialEq, Debug, Copy, Clone)]
pub struct SolarCoordinates {
    // The declination of the sun, the angle between
    // the rays of the Sun and the plane of the Earth's equator.
    declination: Angle,

    // Right ascension of the Sun, the angular distance on the
    // celestial equator from the vernal equinox to the hour circle.
    right_ascension: Angle,

    // Apparent sidereal time, the hour angle of the vernal equinox.
    apparent_sidereal_time: Angle,
}

impl SolarCoordinates {
    fn new(julian_day: f64) -> Self {
        let julian_century = ops::julian_century(julian_day);
        let mean_solar_longitude = ops::mean_solar_longitude(julian_century);
        let mean_lunar_longitude = ops::mean_lunar_longitude(julian_century);
        let ascending_lunar_node = ops::ascending_lunar_node_longitude(julian_century);
        let apparent_solar_longitude =
            ops::apparent_solar_longitude(julian_century, mean_solar_longitude).radians();

        let mean_sidereal_time = ops::mean_sidereal_time(julian_century);
        let nutation_longitude = ops::nutation_in_longitude(
            mean_solar_longitude,
            mean_lunar_longitude,
            ascending_lunar_node,
        );
        let nutation_obliq = ops::nutation_in_obliquity(
            mean_solar_longitude,
            mean_lunar_longitude,
            ascending_lunar_node,
        );

        let mean_obliq_ecliptic = ops::mean_obliquity_of_the_ecliptic(julian_century);
        let apparent_obliq_ecliptic =
            ops::apparent_obliquity_of_the_ecliptic(julian_century, mean_obliq_ecliptic).radians();

        // Equation from Astronomical Algorithms page 165
        let declination = Angle::from_radians(
            (apparent_obliq_ecliptic.sin() * apparent_solar_longitude.sin()).asin(),
        );

        // Equation from Astronomical Algorithms page 165
        let right_ascension = Angle::from_radians(
            (apparent_obliq_ecliptic.cos() * apparent_solar_longitude.sin())
                .atan2(apparent_solar_longitude.cos()),
        )
        .unwound();

        // Equation from Astronomical Algorithms page 88
        let apparent_sidereal_time = Angle::new(
            mean_sidereal_time.degrees
                + ((nutation_longitude * 3600.0)
                    * Angle::new(mean_obliq_ecliptic.degrees + nutation_obliq)
                        .radians()
                        .cos())
                    / 3600.0,
        );

        SolarCoordinates {
            declination: declination,
            right_ascension: right_ascension,
            apparent_sidereal_time: apparent_sidereal_time,
        }
    }
}

/// The sun's transit, rising and setting on a UTC date at a place, in hours
/// after that date's 00:00 UTC (a time of day for the place's solar day,
/// which can be below 0 or above 24 away from the prime meridian).
#[derive(Debug, Copy, Clone)]
pub struct SolarTime {
    observer: Coordinates,
    solar: SolarCoordinates,
    prev_solar: SolarCoordinates,
    next_solar: SolarCoordinates,
    approx_transit: f64,
    pub transit: f64,
    pub sunrise: f64,
    pub sunset: f64,
}

impl SolarTime {
    /// The sun's day on `date` at `coordinates`, or why it has no sunrise
    /// or sunset.
    pub fn new(date: NaiveDate, coordinates: Coordinates) -> Result<SolarTime, Error> {
        // All calculations are for 0h UTC of the date.
        let julian_day = ops::julian_day(date.year(), date.month() as i32, date.day() as i32, 0.0);
        let prev_solar = SolarCoordinates::new(julian_day - 1.0);
        let solar = SolarCoordinates::new(julian_day);
        let next_solar = SolarCoordinates::new(julian_day + 1.0);
        let approx_transit = ops::approximate_transit(
            coordinates.longitude_angle(),
            solar.apparent_sidereal_time,
            solar.right_ascension,
        );
        let transit = ops::corrected_transit(
            approx_transit,
            coordinates.longitude_angle(),
            solar.apparent_sidereal_time,
            solar.right_ascension,
            prev_solar.right_ascension,
            next_solar.right_ascension,
        );
        let partial = SolarTime {
            observer: coordinates,
            solar,
            prev_solar,
            next_solar,
            approx_transit,
            transit,
            sunrise: f64::NAN,
            sunset: f64::NAN,
        };
        // The sun's upper limb on the horizon, with refraction: Astronomical
        // Algorithms page 101.
        let horizon = Angle::new(-50.0 / 60.0);
        match (
            partial.time_for_solar_angle(horizon, false),
            partial.time_for_solar_angle(horizon, true),
        ) {
            (Some(sunrise), Some(sunset)) => Ok(SolarTime {
                sunrise,
                sunset,
                ..partial
            }),
            // Never crossing the horizon: below it at noon, or above it all day.
            _ if partial.noon_altitude() < horizon.degrees => Err(Error::SunNeverRises(date)),
            _ => Err(Error::SunNeverSets(date)),
        }
    }

    /// The sun's altitude at transit, in degrees.
    fn noon_altitude(&self) -> f64 {
        90.0 - (self.observer.latitude - self.solar.declination.degrees).abs()
    }

    /// When the sun's center is at `angle` (degrees above the horizon)
    /// before or after transit, in hours after 00:00 UTC; None if it never
    /// is that day.
    pub fn time_for_solar_angle(&self, angle: Angle, after_transit: bool) -> Option<f64> {
        let hours = ops::corrected_hour_angle(
            self.approx_transit,
            angle,
            self.observer,
            after_transit,
            self.solar.apparent_sidereal_time,
            self.solar.right_ascension,
            self.prev_solar.right_ascension,
            self.next_solar.right_ascension,
            self.solar.declination,
            self.prev_solar.declination,
            self.next_solar.declination,
        );

        hours.is_finite().then_some(hours)
    }

    /// When a stick's shadow is `shadow_length` times its height plus its
    /// noon shadow (Asr), in hours after 00:00 UTC.
    pub fn afternoon(&self, shadow_length: f64) -> Option<f64> {
        let absolute_degrees = (self.observer.latitude - self.solar.declination.degrees).abs();
        let tangent = Angle::new(absolute_degrees);
        let inverse = shadow_length + tangent.radians().tan();
        let angle = Angle::from_radians((1.0 / inverse).atan());

        self.time_for_solar_angle(angle, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astronomy::ops;

    /// `hours` after 00:00 UTC as "H:MM", to the nearest minute.
    fn clock(hours: f64) -> String {
        let minutes = (hours * 60.0).round() as i64;
        format!(
            "{}:{:02}",
            minutes.div_euclid(60).rem_euclid(24),
            minutes.rem_euclid(60)
        )
    }

    fn raleigh() -> Coordinates {
        Coordinates::new(35.0 + 47.0 / 60.0, -78.0 - 39.0 / 60.0).unwrap()
    }

    #[test]
    fn solar_coordinates() {
        let julian_day = ops::julian_day(1992, 10, 13, 0.0);
        let solar = SolarCoordinates::new(julian_day);

        assert_eq!(solar.declination.degrees, -7.7850685152648795);
        assert_eq!(solar.right_ascension.degrees, 198.38082214251881);
        assert_eq!(solar.right_ascension.unwound().degrees, 198.38082214251881);
    }

    #[test]
    fn calculate_solar_time() {
        let date = NaiveDate::from_ymd_opt(2015, 7, 12).unwrap();
        let solar = SolarTime::new(date, raleigh()).unwrap();

        assert_eq!(clock(solar.transit), "17:20");
        assert_eq!(clock(solar.sunrise), "10:08");
        assert_eq!(clock(solar.sunset), "0:32");
        assert!(solar.sunset > 24.0, "sunset is on the next UTC day");
    }

    #[test]
    fn calculate_time_for_solar_angle() {
        let date = NaiveDate::from_ymd_opt(2015, 7, 12).unwrap();
        let solar = SolarTime::new(date, raleigh()).unwrap();
        let angle = Angle::new(-6.0);
        let twilight_start = solar.time_for_solar_angle(angle, false).unwrap();
        let twilight_end = solar.time_for_solar_angle(angle, true).unwrap();

        assert_eq!(clock(twilight_start), "9:38");
        assert_eq!(clock(twilight_end), "1:02");
    }

    #[test]
    fn polar_day_and_night_are_errors_not_panics() {
        // Inside the Arctic Circle: midnight sun in June, polar night in
        // December. salah 0.7 unwrapped the missing sunrise (issue #20).
        let arctic = Coordinates::new(69.65, 18.96).unwrap();
        let june = NaiveDate::from_ymd_opt(2026, 6, 21).unwrap();
        let december = NaiveDate::from_ymd_opt(2026, 12, 21).unwrap();

        assert_eq!(
            SolarTime::new(june, arctic).unwrap_err(),
            Error::SunNeverSets(june)
        );
        assert_eq!(
            SolarTime::new(december, arctic).unwrap_err(),
            Error::SunNeverRises(december)
        );
    }

    #[test]
    fn calculate_corrected_hour_angle() {
        let coordinates = raleigh();
        let julian_day = ops::julian_day(2015, 7, 12, 0.0);
        let prev_solar = SolarCoordinates::new(julian_day - 1.0);
        let solar = SolarCoordinates::new(julian_day);
        let next_solar = SolarCoordinates::new(julian_day + 1.0);
        let solar_altitude = Angle::new(-50.0 / 60.0);
        let approx_transit = ops::approximate_transit(
            coordinates.longitude_angle(),
            solar.apparent_sidereal_time,
            solar.right_ascension,
        );
        let sunrise_time = ops::corrected_hour_angle(
            approx_transit,
            solar_altitude,
            coordinates,
            false,
            solar.apparent_sidereal_time,
            solar.right_ascension,
            prev_solar.right_ascension,
            next_solar.right_ascension,
            solar.declination,
            prev_solar.declination,
            next_solar.declination,
        );

        assert_eq!(sunrise_time, 10.131800480632849);
    }
}
