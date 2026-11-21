// Salah
//
// See LICENSE for more details.
// Copyright (c) 2019-2022 Farhan Ahmed. All rights reserved.
//

//! # Prayer Schedule
//!
//! This module provides the main objects that are used for calculating
//! the prayer times.

use chrono::{DateTime, Datelike, NaiveDate, Utc};

use crate::astronomy::ops;
use crate::astronomy::solar::SolarTime;
use crate::astronomy::unit::Angle;
use crate::error::Error;
use crate::models::coordinates::Coordinates;
use crate::models::method::Method;
use crate::models::parameters::{Isha, Parameters};
use crate::models::prayer::Prayer;
use crate::time::Moment;

/// The first date prayer times are calculated for.
pub const FIRST_DATE: NaiveDate = NaiveDate::from_ymd_opt(1000, 1, 1).unwrap();
/// The last date prayer times are calculated for. The astronomical series
/// are Meeus's, made for the centuries around 2000; the bounds keep every
/// date and time far from chrono's limits.
pub const LAST_DATE: NaiveDate = NaiveDate::from_ymd_opt(3000, 12, 31).unwrap();

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
    date: NaiveDate,
    parameters: Parameters,
}

/// The sun's day on a date, to the second.
#[derive(Debug, Copy, Clone)]
struct SolarDay {
    date: NaiveDate,
    solar: SolarTime,
    transit: Moment,
    sunrise: Moment,
    sunset: Moment,
}

impl SolarDay {
    fn new(date: NaiveDate, coordinates: Coordinates) -> Result<SolarDay, Error> {
        let solar = SolarTime::new(date, coordinates)?;
        let at = |hours| Moment::at_hours(date, hours);
        match (at(solar.transit), at(solar.sunrise), at(solar.sunset)) {
            (Some(transit), Some(sunrise), Some(sunset)) => Ok(SolarDay {
                date,
                solar,
                transit,
                sunrise,
                sunset,
            }),
            // SolarTime's hours are within a day of the date.
            _ => Err(Error::DateOutOfRange(date)),
        }
    }

    /// When the sun is `depression` degrees below the horizon, before or
    /// after transit.
    fn below_horizon(&self, depression: f64, after_transit: bool) -> Option<Moment> {
        let hours = self
            .solar
            .time_for_solar_angle(Angle::new(-depression), after_transit)?;
        Moment::at_hours(self.date, hours)
    }

    fn afternoon(&self, shadow_length: f64) -> Option<Moment> {
        Moment::at_hours(self.date, self.solar.afternoon(shadow_length)?)
    }
}

/// A day's six times, before adjustments and rounding.
#[derive(Debug, Copy, Clone)]
struct Day {
    fajr: Moment,
    sunrise: Moment,
    dhuhr: Moment,
    asr: Moment,
    maghrib: Moment,
    isha: Moment,
}

impl Day {
    /// The times of the solar day of `date`, which take the next day's
    /// sunrise for the length of the night.
    fn new(
        date: NaiveDate,
        coordinates: Coordinates,
        parameters: &Parameters,
    ) -> Result<Day, Error> {
        let today = SolarDay::new(date, coordinates)?;
        let tomorrow = SolarDay::new(next_day(date)?, coordinates)?;
        let night = tomorrow.sunrise.since(today.sunset) as f64;
        let moonsighting = parameters.method == Method::MoonsightingCommittee;
        let moonsighting_high = moonsighting && coordinates.latitude >= 55.0;

        let safe_fajr = if moonsighting {
            today.sunrise.plus(ops::season_adjusted_morning_twilight(
                coordinates.latitude,
                date.ordinal(),
                date.year() as u32,
            ))
        } else {
            today
                .sunrise
                .plus_fraction(-parameters.night_portion(parameters.fajr_angle) * night)
        };
        let fajr = if moonsighting_high {
            Some(today.sunrise.plus_fraction(-night / 7.0))
        } else {
            today.below_horizon(parameters.fajr_angle.degrees(), false)
        };
        let fajr = fajr.map_or(safe_fajr, |fajr| fajr.max(safe_fajr));

        let isha = match parameters.isha {
            Isha::Interval(minutes) => today.sunset.plus_minutes(minutes.into()),
            Isha::Angle(angle) => {
                let safe_isha = if moonsighting {
                    today.sunset.plus(ops::season_adjusted_evening_twilight(
                        coordinates.latitude,
                        date.ordinal(),
                        date.year() as u32,
                        parameters.shafaq,
                    ))
                } else {
                    today
                        .sunset
                        .plus_fraction(parameters.night_portion(angle) * night)
                };
                let isha = if moonsighting_high {
                    Some(today.sunset.plus_fraction(night / 7.0))
                } else {
                    today.below_horizon(angle.degrees(), true)
                };
                isha.map_or(safe_isha, |isha| isha.min(safe_isha))
            }
        };

        // Tehran's method: Maghrib when the sun is that far below the
        // horizon, if that is between sunset and Isha.
        let maghrib = parameters
            .maghrib_angle
            .and_then(|angle| today.below_horizon(angle.degrees(), true))
            .filter(|&maghrib| today.sunset < maghrib && maghrib < isha)
            .unwrap_or(today.sunset);

        Ok(Day {
            fajr,
            sunrise: today.sunrise,
            dhuhr: today.transit,
            asr: today
                .afternoon(parameters.madhab.shadow().into())
                .unwrap_or(today.transit),
            maghrib,
            isha,
        })
    }

    /// With the adjustments added and rounded.
    fn adjusted(self, parameters: &Parameters) -> Day {
        let adjust = |time: Moment, prayer| {
            time.plus_minutes(parameters.time_adjustments(prayer))
                .rounded(parameters.rounding)
        };
        Day {
            fajr: adjust(self.fajr, Prayer::Fajr),
            sunrise: adjust(self.sunrise, Prayer::Sunrise),
            dhuhr: adjust(self.dhuhr, Prayer::Dhuhr),
            asr: adjust(self.asr, Prayer::Asr),
            maghrib: adjust(self.maghrib, Prayer::Maghrib),
            isha: adjust(self.isha, Prayer::Isha),
        }
    }
}

fn next_day(date: NaiveDate) -> Result<NaiveDate, Error> {
    date.succ_opt().ok_or(Error::DateOutOfRange(date))
}

impl PrayerTimes {
    /// The prayer times of the solar day of the UTC date `date`, or why
    /// they can't be calculated.
    pub fn new(
        date: NaiveDate,
        coordinates: Coordinates,
        parameters: Parameters,
    ) -> Result<PrayerTimes, Error> {
        if !(FIRST_DATE..=LAST_DATE).contains(&date) {
            return Err(Error::DateOutOfRange(date));
        }
        let day = Day::new(date, coordinates, &parameters)?.adjusted(&parameters);
        // The next day's Fajr, as that day's own times have it.
        let fajr_tomorrow = Day::new(next_day(date)?, coordinates, &parameters)?
            .adjusted(&parameters)
            .fajr;

        // The night from Maghrib to the next Fajr: its middle and the start
        // of its last third (Adhan's SunnahTimes).
        let night = fajr_tomorrow.since(day.maghrib) as f64;
        let middle_of_the_night = day
            .maghrib
            .plus((night / 2.0) as i64)
            .rounded(parameters.rounding);
        let qiyam = day
            .maghrib
            .plus((night * (2.0 / 3.0)) as i64)
            .rounded(parameters.rounding);

        let utc = |time: Moment| time.utc().ok_or(Error::DateOutOfRange(date));
        Ok(PrayerTimes {
            fajr: utc(day.fajr)?,
            sunrise: utc(day.sunrise)?,
            dhuhr: utc(day.dhuhr)?,
            asr: utc(day.asr)?,
            maghrib: utc(day.maghrib)?,
            isha: utc(day.isha)?,
            middle_of_the_night: utc(middle_of_the_night)?,
            qiyam: utc(qiyam)?,
            fajr_tomorrow: utc(fajr_tomorrow)?,
            coordinates,
            date,
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
        [
            Prayer::FajrTomorrow,
            Prayer::Qiyam,
            Prayer::Isha,
            Prayer::Maghrib,
            Prayer::Asr,
            Prayer::Dhuhr,
            Prayer::Sunrise,
            Prayer::Fajr,
        ]
        .into_iter()
        .find(|&prayer| self.time(prayer) <= time)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Configuration;
    use crate::models::high_altitude_rule::HighLatitudeRule;
    use crate::models::madhab::Madhab;
    use crate::models::rounding::Rounding;
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
    fn tomorrows_fajr_is_the_next_days_fajr() {
        // salah 0.7 left it unrounded, seconds and all, unlike the next
        // day's own Fajr, and counted the last third of the night from it.
        let params = Configuration::with(Method::NorthAmerica, Madhab::Hanafi);
        let place = Coordinates::new(35.7750, -78.6336).unwrap();
        for day in 1..=27 {
            let today = PrayerTimes::new(date(2026, 2, day), place, params).unwrap();
            let tomorrow = PrayerTimes::new(date(2026, 2, day + 1), place, params).unwrap();
            assert_eq!(
                today.time(Prayer::FajrTomorrow),
                tomorrow.time(Prayer::Fajr)
            );
        }
    }

    #[test]
    fn no_rounding_keeps_the_seconds() {
        // salah 0.7 rounded every time to the minute before Rounding was
        // applied, so Rounding::None changed nothing.
        let mut params = Configuration::with(Method::MuslimWorldLeague, Madhab::Shafi);
        params.rounding = Rounding::None;
        let place = Coordinates::new(35.7750, -78.6336).unwrap();
        let times = PrayerTimes::new(date(2026, 2, 1), place, params).unwrap();
        let seconds = [
            Prayer::Fajr,
            Prayer::Sunrise,
            Prayer::Dhuhr,
            Prayer::Asr,
            Prayer::Maghrib,
        ]
        .map(|p| chrono::Timelike::second(&times.time(p)));
        assert_ne!(seconds, [0; 5]);
    }

    #[test]
    fn tehrans_maghrib_is_at_its_angle() {
        // salah 0.7 kept Tehran's 4.5° Maghrib angle but never used it:
        // Maghrib was at sunset (14:47 UTC). Adhan's times.
        let params = Configuration::with(Method::Tehran, Madhab::Shafi);
        let tehran = Coordinates::new(35.6892, 51.3890).unwrap();
        let times = PrayerTimes::new(date(2026, 3, 21), tehran, params).unwrap();

        assert_eq!(
            times.time(Prayer::Maghrib).format("%-k:%M").to_string(),
            "15:05"
        );
        assert_eq!(
            times.time(Prayer::Isha).format("%-k:%M").to_string(),
            "15:52"
        );
    }

    #[test]
    fn dates_outside_the_supported_years_are_errors() {
        // salah 0.7 panicked in its date arithmetic at chrono's last date.
        let params = Configuration::with(Method::MuslimWorldLeague, Madhab::Shafi);
        let mecca = Coordinates::new(21.4225, 39.8262).unwrap();
        for date in [
            NaiveDate::MAX,
            NaiveDate::MIN,
            date(999, 12, 31),
            date(3001, 1, 1),
        ] {
            assert_eq!(
                PrayerTimes::new(date, mecca, params),
                Err(Error::DateOutOfRange(date))
            );
        }
        assert!(PrayerTimes::new(FIRST_DATE, mecca, params).is_ok());
        assert!(PrayerTimes::new(LAST_DATE, mecca, params).is_ok());
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
