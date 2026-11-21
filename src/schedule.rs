// Salah
//
// See LICENSE for more details.
// Copyright (c) 2019-2022 Farhan Ahmed. All rights reserved.
//

//! # Prayer Schedule
//!
//! This module provides the main objects that are used for calculating
//! the prayer times.

use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};

use crate::astronomy::ops;
use crate::astronomy::solar::SolarTime;
use crate::astronomy::unit::{Angle, Stride};
use crate::error::Error;
use crate::models::coordinates::Coordinates;
use crate::models::method::Method;
use crate::models::parameters::{Isha, Parameters};
use crate::models::prayer::Prayer;
use crate::models::rounding::Rounding;

/// A data struct to hold the timing for all
/// prayers.
#[derive(PartialEq, Debug, Copy, Clone)]
pub struct PrayerTimes {
    fajr: DateTime<Utc>,
    sunrise: DateTime<Utc>,
    dhuhr: DateTime<Utc>,
    asr: DateTime<Utc>,
    maghrib: DateTime<Utc>,
    isha: DateTime<Utc>,
    middle_of_the_night: DateTime<Utc>,
    qiyam: DateTime<Utc>,
    fajr_tomorrow: DateTime<Utc>,
    coordinates: Coordinates,
    date: DateTime<Utc>,
    parameters: Parameters,
}

/// `hours` after `date`'s 00:00 UTC, to the nearest minute. Counted from
/// midnight, so a time that rounds up to 24:00 is the next day's 00:00.
fn at_hours(date: NaiveDate, hours: f64) -> DateTime<Utc> {
    let seconds = (hours * 60.0 * 60.0).floor() as i64;
    let rounded_minutes = (seconds + 30).div_euclid(60);
    date.and_time(chrono::NaiveTime::MIN).and_utc() + Duration::minutes(rounded_minutes)
}

/// The sun's day as times.
#[derive(Debug, Copy, Clone)]
struct SolarDay {
    date: NaiveDate,
    solar: SolarTime,
    transit: DateTime<Utc>,
    sunrise: DateTime<Utc>,
    sunset: DateTime<Utc>,
}

impl SolarDay {
    fn new(date: NaiveDate, coordinates: Coordinates) -> Result<SolarDay, Error> {
        let solar = SolarTime::new(date, coordinates)?;
        Ok(SolarDay {
            date,
            solar,
            transit: at_hours(date, solar.transit),
            sunrise: at_hours(date, solar.sunrise),
            sunset: at_hours(date, solar.sunset),
        })
    }

    fn time_for_solar_angle(&self, angle: Angle, after_transit: bool) -> Option<DateTime<Utc>> {
        self.solar
            .time_for_solar_angle(angle, after_transit)
            .map(|hours| at_hours(self.date, hours))
    }
}

impl PrayerTimes {
    /// The prayer times of the solar day of the UTC date `date`, or why
    /// they can't be calculated.
    pub fn new(
        date: NaiveDate,
        coordinates: Coordinates,
        parameters: Parameters,
    ) -> Result<PrayerTimes, Error> {
        let prayer_date = date.and_time(chrono::NaiveTime::MIN).and_utc();
        let tomorrow = prayer_date.tomorrow();
        let solar_time = SolarDay::new(date, coordinates)?;
        let solar_time_tomorrow = SolarDay::new(tomorrow.date_naive(), coordinates)?;

        let asr = solar_time
            .solar
            .afternoon(parameters.madhab.shadow().into())
            .map(|hours| at_hours(date, hours))
            .unwrap_or(solar_time.transit);
        let night = solar_time_tomorrow
            .sunrise
            .signed_duration_since(solar_time.sunset);

        let final_fajr =
            PrayerTimes::calculate_fajr(parameters, solar_time, night, coordinates, prayer_date)
                .rounded_minute(parameters.rounding);
        let final_sunrise = solar_time
            .sunrise
            .adjust_time(i64::from(parameters.time_adjustments(Prayer::Sunrise)))
            .rounded_minute(parameters.rounding);
        let final_dhuhr = solar_time
            .transit
            .adjust_time(i64::from(parameters.time_adjustments(Prayer::Dhuhr)))
            .rounded_minute(parameters.rounding);
        let final_asr = asr
            .adjust_time(i64::from(parameters.time_adjustments(Prayer::Asr)))
            .rounded_minute(parameters.rounding);
        let final_maghrib = ops::adjust_time(
            &solar_time.sunset,
            i64::from(parameters.time_adjustments(Prayer::Maghrib)),
        )
        .rounded_minute(parameters.rounding);
        let final_isha =
            PrayerTimes::calculate_isha(parameters, solar_time, night, coordinates, prayer_date)
                .rounded_minute(parameters.rounding);

        // Calculate the middle of the night and qiyam times
        let (final_middle_of_night, final_qiyam, final_fajr_tomorrow) =
            PrayerTimes::calculate_qiyam(
                final_maghrib,
                parameters,
                solar_time_tomorrow,
                coordinates,
                tomorrow,
            )?;

        Ok(PrayerTimes {
            fajr: final_fajr,
            sunrise: final_sunrise,
            dhuhr: final_dhuhr,
            asr: final_asr,
            maghrib: final_maghrib,
            isha: final_isha,
            middle_of_the_night: final_middle_of_night,
            qiyam: final_qiyam,
            fajr_tomorrow: final_fajr_tomorrow,
            coordinates,
            date: prayer_date,
            parameters,
        })
    }

    pub fn time(&self, prayer: Prayer) -> DateTime<Utc> {
        match prayer {
            Prayer::Fajr => self.fajr,
            Prayer::Sunrise => self.sunrise,
            Prayer::Dhuhr => self.dhuhr,
            Prayer::Asr => self.asr,
            Prayer::Maghrib => self.maghrib,
            Prayer::Isha => self.isha,
            Prayer::Qiyam => self.qiyam,
            Prayer::FajrTomorrow => self.fajr_tomorrow,
        }
    }

    pub fn current(&self) -> Prayer {
        self.current_time(Utc::now()).expect("Out of bounds")
    }

    pub fn next(&self) -> Prayer {
        match self.current() {
            Prayer::Fajr => Prayer::Sunrise,
            Prayer::Sunrise => Prayer::Dhuhr,
            Prayer::Dhuhr => Prayer::Asr,
            Prayer::Asr => Prayer::Maghrib,
            Prayer::Maghrib => Prayer::Isha,
            Prayer::Isha => Prayer::Qiyam,
            Prayer::Qiyam => Prayer::FajrTomorrow,
            _ => Prayer::FajrTomorrow,
        }
    }

    pub fn time_remaining(&self) -> (u32, u32) {
        let next_time = self.time(self.next());
        let now = Utc::now();
        let now_to_next = next_time.signed_duration_since(now).num_seconds() as f64;
        let whole: f64 = now_to_next / 60.0 / 60.0;
        let fract = whole.fract();
        let hours = whole.trunc() as u32;
        let minutes = (fract * 60.0).round() as u32;

        (hours, minutes)
    }

    fn current_time(&self, time: DateTime<Utc>) -> Option<Prayer> {
        let current_prayer: Option<Prayer>;

        if self.fajr_tomorrow.signed_duration_since(time).num_seconds() <= 0 {
            current_prayer = Some(Prayer::FajrTomorrow)
        } else if self.qiyam.signed_duration_since(time).num_seconds() <= 0 {
            current_prayer = Some(Prayer::Qiyam)
        } else if self.isha.signed_duration_since(time).num_seconds() <= 0 {
            current_prayer = Some(Prayer::Isha);
        } else if self.maghrib.signed_duration_since(time).num_seconds() <= 0 {
            current_prayer = Some(Prayer::Maghrib);
        } else if self.asr.signed_duration_since(time).num_seconds() <= 0 {
            current_prayer = Some(Prayer::Asr);
        } else if self.dhuhr.signed_duration_since(time).num_seconds() <= 0 {
            current_prayer = Some(Prayer::Dhuhr);
        } else if self.sunrise.signed_duration_since(time).num_seconds() <= 0 {
            current_prayer = Some(Prayer::Sunrise);
        } else if self.fajr.signed_duration_since(time).num_seconds() <= 0 {
            current_prayer = Some(Prayer::Fajr);
        } else {
            current_prayer = None;
        }

        current_prayer
    }

    fn calculate_fajr(
        parameters: Parameters,
        solar_time: SolarDay,
        night: Duration,
        coordinates: Coordinates,
        prayer_date: DateTime<Utc>,
    ) -> DateTime<Utc> {
        let safe_fajr = if parameters.method == Method::MoonsightingCommittee {
            let day_of_year = prayer_date.ordinal();
            ops::season_adjusted_morning_twilight(
                coordinates.latitude,
                day_of_year,
                prayer_date.year() as u32,
                solar_time.sunrise,
            )
        } else {
            let portion = parameters.night_portion(parameters.fajr_angle);
            let night_fraction = portion * (night.num_seconds() as f64);

            solar_time
                .sunrise
                .checked_add_signed(Duration::seconds(-night_fraction as i64))
                .unwrap()
        };

        let mut fajr = solar_time
            .time_for_solar_angle(Angle::new(-parameters.fajr_angle.degrees()), false)
            .unwrap_or(safe_fajr);

        // special case for moonsighting committee above latitude 55
        if parameters.method == Method::MoonsightingCommittee && coordinates.latitude >= 55.0 {
            let night_fraction = night.num_seconds() / 7;
            fajr = solar_time
                .sunrise
                .checked_add_signed(Duration::seconds(-night_fraction))
                .unwrap();
        }

        if fajr < safe_fajr {
            fajr = safe_fajr;
        }

        fajr.adjust_time(i64::from(parameters.time_adjustments(Prayer::Fajr)))
    }

    fn calculate_isha(
        parameters: Parameters,
        solar_time: SolarDay,
        night: Duration,
        coordinates: Coordinates,
        prayer_date: DateTime<Utc>,
    ) -> DateTime<Utc> {
        let isha = match parameters.isha {
            Isha::Interval(minutes) => solar_time.sunset + Duration::minutes(minutes.into()),
            Isha::Angle(angle) => {
                let safe_isha = if parameters.method == Method::MoonsightingCommittee {
                    let day_of_year = prayer_date.ordinal();

                    ops::season_adjusted_evening_twilight(
                        coordinates.latitude,
                        day_of_year,
                        prayer_date.year() as u32,
                        solar_time.sunset,
                        parameters.shafaq,
                    )
                } else {
                    let portion = parameters.night_portion(angle);
                    let night_fraction = portion * (night.num_seconds() as f64);

                    solar_time
                        .sunset
                        .checked_add_signed(Duration::seconds(night_fraction as i64))
                        .unwrap()
                };

                let mut isha = solar_time
                    .time_for_solar_angle(Angle::new(-angle.degrees()), true)
                    .unwrap_or(safe_isha);

                // special case for moonsighting committee above latitude 55
                if parameters.method == Method::MoonsightingCommittee
                    && coordinates.latitude >= 55.0
                {
                    let night_fraction = night.num_seconds() / 7;
                    isha = solar_time
                        .sunset
                        .checked_add_signed(Duration::seconds(night_fraction))
                        .unwrap();
                }

                isha.min(safe_isha)
            }
        };

        isha.adjust_time(i64::from(parameters.time_adjustments(Prayer::Isha)))
    }

    fn calculate_qiyam(
        current_maghrib: DateTime<Utc>,
        parameters: Parameters,
        solar_time: SolarDay,
        coordinates: Coordinates,
        prayer_date: DateTime<Utc>,
    ) -> Result<(DateTime<Utc>, DateTime<Utc>, DateTime<Utc>), Error> {
        let tomorrow = prayer_date.tomorrow();
        let solar_time_tomorrow = SolarDay::new(tomorrow.date_naive(), coordinates)?;
        let night = solar_time_tomorrow
            .sunrise
            .signed_duration_since(solar_time.sunset);

        let tomorrow_fajr =
            PrayerTimes::calculate_fajr(parameters, solar_time, night, coordinates, prayer_date);
        let night_duration = tomorrow_fajr
            .signed_duration_since(current_maghrib)
            .num_seconds() as f64;
        let middle_night_portion = (night_duration / 2.0) as i64;
        let last_third_portion = (night_duration * (2.0 / 3.0)) as i64;
        let middle_of_night = current_maghrib
            .checked_add_signed(Duration::seconds(middle_night_portion))
            .unwrap()
            .rounded_minute(Rounding::Nearest);
        let last_third_of_night = current_maghrib
            .checked_add_signed(Duration::seconds(last_third_portion))
            .unwrap()
            .rounded_minute(Rounding::Nearest);

        Ok((middle_of_night, last_third_of_night, tomorrow_fajr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Configuration;
    use crate::models::high_altitude_rule::HighLatitudeRule;
    use crate::models::madhab::Madhab;
    use chrono::{NaiveDate, TimeZone, Utc};

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    /// The six times as "H:MM" in UTC, Fajr to Isha.
    fn clock(times: &PrayerTimes) -> [String; 6] {
        [
            Prayer::Fajr,
            Prayer::Sunrise,
            Prayer::Dhuhr,
            Prayer::Asr,
            Prayer::Maghrib,
            Prayer::Isha,
        ]
        .map(|p| times.time(p).format("%-k:%M").to_string())
    }

    fn raleigh_2015_07_12() -> PrayerTimes {
        let params = Configuration::with(Method::NorthAmerica, Madhab::Hanafi);
        PrayerTimes::new(
            date(2015, 7, 12),
            Coordinates::new(35.7750, -78.6336).unwrap(),
            params,
        )
        .unwrap()
    }

    #[test]
    fn current_prayer_through_the_day() {
        // Fajr 08:42, sunrise 10:08, Dhuhr 17:21, Asr 22:22, Maghrib 00:32,
        // Isha 01:57 (UTC).
        let times = raleigh_2015_07_12();
        let at = |d, h, m| Utc.with_ymd_and_hms(2015, 7, d, h, m, 0).unwrap();

        assert_eq!(times.current_time(at(12, 8, 0)), None);
        assert_eq!(times.current_time(at(12, 9, 0)), Some(Prayer::Fajr));
        assert_eq!(times.current_time(at(12, 11, 0)), Some(Prayer::Sunrise));
        assert_eq!(times.current_time(at(12, 19, 0)), Some(Prayer::Dhuhr));
        assert_eq!(times.current_time(at(12, 22, 26)), Some(Prayer::Asr));
        assert_eq!(times.current_time(at(13, 1, 0)), Some(Prayer::Maghrib));
        assert_eq!(times.current_time(at(13, 2, 0)), Some(Prayer::Isha));
    }

    #[test]
    fn calculate_times_for_moonsighting_method() {
        let params = Configuration::with(Method::MoonsightingCommittee, Madhab::Shafi);
        let times = PrayerTimes::new(
            date(2016, 1, 31),
            Coordinates::new(35.7750, -78.6336).unwrap(),
            params,
        )
        .unwrap();

        assert_eq!(
            clock(&times),
            ["10:48", "12:16", "17:33", "20:20", "22:43", "0:05"]
        );
    }

    #[test]
    fn calculate_times_for_moonsighting_method_with_high_latitude() {
        let params = Configuration::with(Method::MoonsightingCommittee, Madhab::Hanafi);
        let times = PrayerTimes::new(
            date(2016, 1, 1),
            Coordinates::new(59.9094, 10.7349).unwrap(),
            params,
        )
        .unwrap();

        assert_eq!(
            clock(&times),
            ["6:34", "8:19", "11:25", "12:36", "14:25", "16:02"]
        );
    }

    fn seventh_of_the_night() -> Parameters {
        let mut params = Configuration::with(Method::MuslimWorldLeague, Madhab::Shafi);
        params.high_latitude_rule = HighLatitudeRule::SeventhOfTheNight;
        params.rounding = Rounding::Nearest;
        params
    }

    #[test]
    fn calculate_times_for_muslim_world_league_with_high_latitude_for_paris_france() {
        let paris = Coordinates::new(48.8566, 2.3522).unwrap();
        let times = PrayerTimes::new(date(2026, 6, 21), paris, seventh_of_the_night()).unwrap();

        assert_eq!(
            clock(&times),
            ["2:40", "3:47", "11:53", "16:10", "19:58", "21:05"]
        );
    }

    #[test]
    fn calculate_times_for_muslim_world_league_with_high_latitude_for_brussels_belgium() {
        let brussels = Coordinates::new(50.85, 4.35).unwrap();
        let times = PrayerTimes::new(date(2026, 6, 21), brussels, seventh_of_the_night()).unwrap();

        assert_eq!(
            clock(&times),
            ["2:25", "3:29", "11:45", "16:06", "20:00", "21:04"]
        );
    }

    #[test]
    fn calculate_times_with_isha_rounding_up_to_midnight() {
        // Isha in Lima on this day is at 23:59:3x UTC, which rounds up to
        // 24:00. It used to fall back to the safe Isha time (05:08 UTC).
        let params = Configuration::with(Method::NorthAmerica, Madhab::Shafi);
        let lima = Coordinates::new(-12.05, -77.04).unwrap();
        let times = PrayerTimes::new(date(2026, 4, 19), lima, params).unwrap();

        assert_eq!(
            times.time(Prayer::Isha),
            Utc.with_ymd_and_hms(2026, 4, 20, 0, 0, 0).unwrap()
        );
    }

    #[test]
    fn sunset_rounding_up_to_midnight_rolls_over_to_the_next_day() {
        // Mexico City's sunset on this day is at 23:59:3x UTC, which rounds
        // up to 24:00 and used to panic.
        let params = Configuration::with(Method::MuslimWorldLeague, Madhab::Shafi);
        let mexico_city = Coordinates::new(19.43, -99.13).unwrap();
        let times = PrayerTimes::new(date(2026, 11, 8), mexico_city, params).unwrap();

        assert_eq!(
            times.time(Prayer::Maghrib),
            Utc.with_ymd_and_hms(2026, 11, 9, 0, 0, 0).unwrap()
        );
    }

    #[test]
    fn polar_day_and_night_are_errors_not_panics() {
        // Inside the Arctic Circle, issue #20. The next day's sunrise is
        // needed too, so a day before the polar night is an error as well.
        let arctic = Coordinates::new(69.65, 18.96).unwrap();
        let params = Configuration::with(Method::MuslimWorldLeague, Madhab::Shafi);

        assert_eq!(
            PrayerTimes::new(date(2026, 6, 21), arctic, params),
            Err(Error::SunNeverSets(date(2026, 6, 21)))
        );
        assert_eq!(
            PrayerTimes::new(date(2026, 12, 21), arctic, params),
            Err(Error::SunNeverRises(date(2026, 12, 21)))
        );
    }
}
