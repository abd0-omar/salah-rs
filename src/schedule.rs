// Salah
//
// See LICENSE for more details.
// Copyright (c) 2019-2022 Farhan Ahmed. All rights reserved.
//

//! # Prayer Schedule
//!
//! This module provides the main objects that are used for calculating
//! the prayer times.

use chrono::{DateTime, Datelike, NaiveDate, TimeDelta, TimeZone, Utc};

use crate::astronomy::ops;
use crate::astronomy::solar::SolarTime;
use crate::astronomy::unit::Angle;
use crate::error::Error;
use crate::models::coordinates::Coordinates;
use crate::models::method::Method;
use crate::models::parameters::{Isha, Parameters};
use crate::models::polar_circle_resolution::PolarCircleResolution;
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
    /// The times of the solar day of `date`, or, where there are none
    /// near the poles, those the polar circle resolution takes.
    fn new(
        date: NaiveDate,
        coordinates: Coordinates,
        parameters: &Parameters,
    ) -> Result<Day, Error> {
        Day::unresolved(date, coordinates, parameters).or_else(|error| match error {
            Error::DateOutOfRange(_) => Err(error),
            Error::SunNeverRises(_)
            | Error::SunNeverSets(_)
            | Error::NoAsr(_)
            | Error::TimesOutOfOrder(_) => match parameters.polar_circle_resolution {
                PolarCircleResolution::Unresolved => None,
                PolarCircleResolution::AqrabBalad => {
                    Day::nearest_latitude(date, coordinates, parameters)
                }
                PolarCircleResolution::AqrabYaum => Day::nearest_day(date, coordinates, parameters),
            }
            .ok_or(error),
        })
    }

    /// The times of the nearest latitude toward the equator, in steps of
    /// half a degree, that has them on `date`; at the equator the season is
    /// the other hemisphere's, so the search stops there.
    fn nearest_latitude(
        date: NaiveDate,
        coordinates: Coordinates,
        parameters: &Parameters,
    ) -> Option<Day> {
        let steps = (coordinates.latitude.abs() / LATITUDE_STEP).ceil() as u32;
        (1..=steps).find_map(|step| {
            let latitude = (coordinates.latitude.abs() - f64::from(step) * LATITUDE_STEP).max(0.0)
                * coordinates.latitude.signum();
            let place = Coordinates {
                latitude,
                ..coordinates
            };
            Day::unresolved(date, place, parameters).ok()
        })
    }

    /// The times of the nearest day that has them at the place, a day
    /// after then a day before, up to half a year away, moved to `date`.
    fn nearest_day(
        date: NaiveDate,
        coordinates: Coordinates,
        parameters: &Parameters,
    ) -> Option<Day> {
        (1..=DAYS_SEARCHED)
            .flat_map(|days| [days, -days])
            .find_map(|days| {
                let other = date.checked_add_signed(TimeDelta::days(days))?;
                let day = Day::unresolved(other, coordinates, parameters).ok()?;
                Some(day.shifted(-days * SECONDS_PER_DAY))
            })
    }

    /// Every time `seconds` later.
    fn shifted(self, seconds: i64) -> Day {
        let [fajr, sunrise, dhuhr, asr, maghrib, isha] = self.times().map(|t| t.plus(seconds));
        Day {
            fajr,
            sunrise,
            dhuhr,
            asr,
            maghrib,
            isha,
        }
    }

    /// The times of the solar day of `date`, which take the next day's
    /// sunrise for the length of the night.
    fn unresolved(
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
        let fajr = fajr
            .filter(|&fajr| fajr >= safe_fajr && in_order(fajr, today.sunrise))
            .unwrap_or(safe_fajr);

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
                isha.filter(|&isha| isha <= safe_isha && in_order(today.sunset, isha))
                    .unwrap_or(safe_isha)
            }
        };

        // Tehran's method: Maghrib when the sun is that far below the
        // horizon, if that is between sunset and Isha.
        let maghrib = parameters
            .maghrib_angle
            .and_then(|angle| today.below_horizon(angle.degrees(), true))
            .filter(|&maghrib| today.sunset < maghrib && maghrib < isha)
            .unwrap_or(today.sunset);

        let day = Day {
            fajr,
            sunrise: today.sunrise,
            dhuhr: today.transit,
            asr: today
                .afternoon(parameters.madhab.shadow().into())
                .ok_or(Error::NoAsr(date))?,
            maghrib,
            isha,
        };
        // Near the polar circles the times can crowd or cross; then there
        // are none (Adhan's guard).
        match day
            .times()
            .windows(2)
            .all(|pair| in_order(pair[0], pair[1]))
        {
            true => Ok(day),
            false => Err(Error::TimesOutOfOrder(date)),
        }
    }

    fn times(&self) -> [Moment; 6] {
        [
            self.fajr,
            self.sunrise,
            self.dhuhr,
            self.asr,
            self.maghrib,
            self.isha,
        ]
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

/// AqrabBalad's step toward the equator, in degrees (Adhan's).
const LATITUDE_STEP: f64 = 0.5;
/// AqrabYaum looks this many days either way: half a year.
const DAYS_SEARCHED: i64 = 183;
const SECONDS_PER_DAY: i64 = 86_400;

/// The least time between one prayer time and the next (Adhan's).
const MIN_GAP_SECONDS: i64 = 5 * 60;

/// Whether `later` is at least five minutes after `earlier`.
fn in_order(earlier: Moment, later: Moment) -> bool {
    later.since(earlier) >= MIN_GAP_SECONDS
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

    /// The prayer times of the local calendar day `date` in `zone`: those
    /// of the solar day whose Dhuhr falls on `date` there.
    ///
    /// [`new`](Self::new) takes the solar day of a UTC date, whose noon is
    /// at about 12:00 local mean time on it. That is the local date's only
    /// where the zone keeps near its meridian: Kiritimati keeps UTC+14 at
    /// 157°W, so the solar day of a UTC date has its noon on the next day
    /// there.
    pub fn on_local_date<Tz: TimeZone>(
        date: NaiveDate,
        zone: &Tz,
        coordinates: Coordinates,
        parameters: Parameters,
    ) -> Result<PrayerTimes, Error> {
        let times = PrayerTimes::new(date, coordinates, parameters)?;
        let noon = times.dhuhr.with_timezone(zone).date_naive();
        match noon == date {
            true => Ok(times),
            false => {
                let solar_date = date
                    .checked_add_signed(date - noon)
                    .ok_or(Error::DateOutOfRange(date))?;
                PrayerTimes::new(solar_date, coordinates, parameters)
            }
        }
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

    /// The middle of the night, from Maghrib to the next Fajr.
    pub fn middle_of_the_night(&self) -> DateTime<Utc> {
        self.middle_of_the_night
    }

    /// The prayer whose time has most recently come at `now`; None
    /// before this day's Fajr (the night of the day before).
    pub fn current(&self, now: DateTime<Utc>) -> Option<Prayer> {
        Prayer::IN_ORDER
            .into_iter()
            .rev()
            .find(|&prayer| self.time(prayer) <= now)
    }

    /// The next prayer to come after `now`; None once tomorrow's Fajr has
    /// come (that is the next day's times).
    pub fn next(&self, now: DateTime<Utc>) -> Option<Prayer> {
        Prayer::IN_ORDER
            .into_iter()
            .find(|&prayer| self.time(prayer) > now)
    }

    /// The time from `now` to the next prayer, as in [`next`](Self::next).
    pub fn time_remaining(&self, now: DateTime<Utc>) -> Option<TimeDelta> {
        self.next(now).map(|prayer| self.time(prayer) - now)
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
        // Isha 01:57, the last third of the night 05:59, the next Fajr
        // 08:43 (UTC).
        let times = raleigh_2015_07_12();
        let at = |d, h, m| Utc.with_ymd_and_hms(2015, 7, d, h, m, 0).unwrap();

        assert_eq!(times.current(at(12, 8, 0)), None);
        assert_eq!(times.current(at(12, 9, 0)), Some(Prayer::Fajr));
        assert_eq!(times.current(at(12, 11, 0)), Some(Prayer::Sunrise));
        assert_eq!(times.current(at(12, 19, 0)), Some(Prayer::Dhuhr));
        assert_eq!(times.current(at(12, 22, 26)), Some(Prayer::Asr));
        assert_eq!(times.current(at(13, 1, 0)), Some(Prayer::Maghrib));
        assert_eq!(times.current(at(13, 2, 0)), Some(Prayer::Isha));
        assert_eq!(times.current(at(13, 7, 0)), Some(Prayer::Qiyam));
        assert_eq!(times.current(at(13, 9, 0)), Some(Prayer::FajrTomorrow));
    }

    #[test]
    fn next_prayer_and_time_remaining_before_fajr_and_after_the_day() {
        // salah 0.7's next() and time_remaining() went through current(),
        // which panicked before Fajr (PR #23) and on any day but today,
        // reading the clock itself (issue #17).
        let times = raleigh_2015_07_12();
        let at = |d, h, m| Utc.with_ymd_and_hms(2015, 7, d, h, m, 0).unwrap();

        assert_eq!(times.next(at(12, 8, 0)), Some(Prayer::Fajr));
        assert_eq!(
            times.time_remaining(at(12, 8, 0)),
            Some(TimeDelta::minutes(42))
        );
        assert_eq!(times.next(at(12, 9, 0)), Some(Prayer::Sunrise));
        assert_eq!(times.next(at(13, 2, 0)), Some(Prayer::Qiyam));
        assert_eq!(times.next(at(13, 7, 0)), Some(Prayer::FajrTomorrow));
        assert_eq!(times.next(at(13, 9, 0)), None);
        assert_eq!(times.time_remaining(at(13, 9, 0)), None);
        assert_eq!(times.current(at(20, 0, 0)), Some(Prayer::FajrTomorrow));
        assert_eq!(times.current(at(1, 0, 0)), None);
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
            ["10:48", "12:16", "17:33", "20:19", "22:43", "0:05"]
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
            ["6:34", "8:19", "11:25", "12:35", "14:25", "16:02"]
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
    fn noon_near_the_date_line_is_on_the_date() {
        // salah 0.7 put the transit anywhere in the UTC day, so east of
        // about 176°E, when the equation of time brings noon before 00:00
        // UTC, it took the next day's: Suva's Dhuhr on 3 November (11:51
        // at UTC+12) came out on the 4th.
        let params = Configuration::with(Method::MuslimWorldLeague, Madhab::Shafi);
        let suva = Coordinates::new(-18.14, 178.44).unwrap();
        let times = PrayerTimes::new(date(2026, 11, 3), suva, params).unwrap();
        assert_eq!(
            times.time(Prayer::Dhuhr),
            Utc.with_ymd_and_hms(2026, 11, 2, 23, 51, 0).unwrap()
        );

        // On each side of the date line, noon is near 12:00 local mean
        // time on the date: 00:00 UTC that day at 180°, 24:00 at -180°.
        for (longitude, offset) in [(180.0, 0), (-180.0, 24), (179.9, 0), (-179.9, 24)] {
            let place = Coordinates::new(0.0, longitude).unwrap();
            for day in [date(2026, 2, 11), date(2026, 11, 3)] {
                let dhuhr = PrayerTimes::new(day, place, params)
                    .unwrap()
                    .time(Prayer::Dhuhr);
                let mean_noon =
                    day.and_hms_opt(0, 0, 0).unwrap().and_utc() + chrono::Duration::hours(offset);
                let off = (dhuhr - mean_noon).num_minutes().abs();
                assert!(off < 30, "{longitude} on {day}: Dhuhr {dhuhr}");
            }
        }
    }

    #[test]
    fn sunset_and_twilight_converge_near_the_polar_circle() {
        // salah 0.7 took one Newton step from the 0h declination, which
        // near the end of the midnight sun is far off: in Tromsø it put
        // sunset 16 minutes early in July and Isha 34 minutes late in
        // September. Adhan's times.
        let params = Configuration::with(Method::MuslimWorldLeague, Madhab::Shafi);
        let tromso = Coordinates::new(69.65, 18.96).unwrap();

        let july = PrayerTimes::new(date(2026, 7, 26), tromso, params).unwrap();
        assert_eq!(
            july.time(Prayer::Maghrib),
            Utc.with_ymd_and_hms(2026, 7, 26, 22, 13, 0).unwrap()
        );
        let september = PrayerTimes::new(date(2026, 9, 14), tromso, params).unwrap();
        assert_eq!(
            september.time(Prayer::Isha),
            Utc.with_ymd_and_hms(2026, 9, 14, 22, 6, 0).unwrap()
        );
    }

    #[test]
    fn an_asr_the_sun_cant_reach_is_an_error() {
        // Just south of the Arctic Circle at the December solstice the sun
        // rises but never gets high enough for the Asr shadow. salah 0.7
        // gave Dhuhr's time for Asr.
        let params = Configuration::with(Method::MuslimWorldLeague, Madhab::Shafi);
        let jokkmokk = Coordinates::new(66.61, 19.82).unwrap();
        let day = date(2026, 12, 21);
        assert_eq!(
            PrayerTimes::new(day, jokkmokk, params),
            Err(Error::NoAsr(day))
        );
    }

    #[test]
    fn times_crowding_near_the_polar_circle_are_an_error() {
        // In Lapland at the December solstice the sun barely clears the
        // horizon: salah 0.7 gave Dhuhr at 10:39 and Asr at 10:42 UTC.
        // As in Adhan, times closer than five minutes are no times.
        let params = Configuration::with(Method::MuslimWorldLeague, Madhab::Shafi);
        let lapland = Coordinates::new(66.1, 20.0).unwrap();
        let day = date(2026, 12, 21);
        assert_eq!(
            PrayerTimes::new(day, lapland, params),
            Err(Error::TimesOutOfOrder(day))
        );
    }

    #[test]
    fn moonsighting_seasons_in_the_southern_hemisphere() {
        // salah 0.7 counted the days since the southern solstice as
        // day - 172 + 365 in u32: an overflow panic before late June (in a
        // debug build), and a year too many after it. Adhan's times.
        let params = Configuration::with(Method::MoonsightingCommittee, Madhab::Shafi);
        let cape_town = Coordinates::new(-33.92, 18.42).unwrap();
        let january = PrayerTimes::new(date(2026, 1, 15), cape_town, params).unwrap();
        let september = PrayerTimes::new(date(2026, 9, 15), cape_town, params).unwrap();

        assert_eq!(
            clock(&january),
            ["2:11", "3:51", "11:01", "14:40", "18:03", "19:14"]
        );
        assert_eq!(
            clock(&september),
            ["3:23", "4:46", "10:47", "14:05", "16:41", "17:55"]
        );
    }

    #[test]
    fn polar_circle_resolutions() {
        // Issue #20, PR #21: times for the midnight sun and the polar night
        // from the nearest latitude or day that has them. Adhan's times.
        let arctic = Coordinates::new(69.65, 18.96).unwrap();
        let with = |resolution| {
            let mut params = Configuration::with(Method::MuslimWorldLeague, Madhab::Shafi);
            params.polar_circle_resolution = resolution;
            params
        };
        let balad = with(PolarCircleResolution::AqrabBalad);
        let yaum = with(PolarCircleResolution::AqrabYaum);

        let june = PrayerTimes::new(date(2026, 6, 21), arctic, balad).unwrap();
        assert_eq!(
            clock(&june),
            ["22:46", "23:05", "10:47", "15:43", "22:26", "22:46"]
        );
        let december = PrayerTimes::new(date(2026, 12, 21), arctic, balad).unwrap();
        assert_eq!(
            clock(&december),
            ["5:15", "9:10", "10:43", "10:51", "12:15", "16:00"]
        );
        let june = PrayerTimes::new(date(2026, 6, 21), arctic, yaum).unwrap();
        assert_eq!(
            clock(&june),
            ["22:27", "23:05", "10:52", "15:38", "22:13", "22:51"]
        );
        let december = PrayerTimes::new(date(2026, 12, 21), arctic, yaum).unwrap();
        assert_eq!(
            clock(&december),
            ["4:34", "8:59", "10:31", "10:35", "12:00", "16:12"]
        );
        // Fajr and sunrise on the evening before, UTC.
        assert_eq!(june.time(Prayer::Fajr).date_naive(), date(2026, 6, 20));
    }

    #[test]
    fn times_of_a_local_date_far_from_the_zones_meridian() {
        // Kiritimati keeps UTC+14 at 157°W: the solar day of a UTC date has
        // its Dhuhr on the next local date there.
        let params = Configuration::with(Method::MuslimWorldLeague, Madhab::Shafi);
        let kiritimati = Coordinates::new(1.87, -157.40).unwrap();
        let zone = chrono::FixedOffset::east_opt(14 * 3600).unwrap();
        let day = date(2026, 3, 10);
        let local =
            |times: &PrayerTimes| times.time(Prayer::Dhuhr).with_timezone(&zone).date_naive();

        let utc_day = PrayerTimes::new(day, kiritimati, params).unwrap();
        assert_eq!(local(&utc_day), date(2026, 3, 11));
        let local_day = PrayerTimes::on_local_date(day, &zone, kiritimati, params).unwrap();
        assert_eq!(local(&local_day), day);
        // Where the zone fits its meridian, it is the UTC date's.
        let mecca = Coordinates::new(21.4225, 39.8262).unwrap();
        let riyadh = chrono::FixedOffset::east_opt(3 * 3600).unwrap();
        assert_eq!(
            PrayerTimes::on_local_date(day, &riyadh, mecca, params),
            PrayerTimes::new(day, mecca, params)
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
