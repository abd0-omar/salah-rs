//! Prayer times for each case on stdin, in the format of
//! tests/fixtures/adhan/grid.ts, to compare with Adhan.
//!
//! cargo run --example grid < cases.csv > salah.csv

use std::io::{BufRead, Write};

use salah::prelude::*;

fn method(name: &str) -> Method {
    match name {
        "MuslimWorldLeague" => Method::MuslimWorldLeague,
        "Egyptian" => Method::Egyptian,
        "Karachi" => Method::Karachi,
        "UmmAlQura" => Method::UmmAlQura,
        "Dubai" => Method::Dubai,
        "MoonsightingCommittee" => Method::MoonsightingCommittee,
        "NorthAmerica" => Method::NorthAmerica,
        "Kuwait" => Method::Kuwait,
        "Qatar" => Method::Qatar,
        "Singapore" => Method::Singapore,
        "Tehran" => Method::Tehran,
        "Turkey" => Method::Turkey,
        other => panic!("unknown method {other}"),
    }
}

fn main() {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        let f: Vec<&str> = line.split(',').collect();
        let coordinates = Coordinates::new(f[0].parse().unwrap(), f[1].parse().unwrap()).unwrap();
        let date = NaiveDate::from_ymd_opt(
            f[2].parse().unwrap(),
            f[3].parse().unwrap(),
            f[4].parse().unwrap(),
        )
        .unwrap();
        let mut params = Configuration::with(
            method(f[5]),
            if f[6] == "hanafi" {
                Madhab::Hanafi
            } else {
                Madhab::Shafi
            },
        );
        params.high_latitude_rule = match f[7] {
            "middleofthenight" => HighLatitudeRule::MiddleOfTheNight,
            "seventhofthenight" => HighLatitudeRule::SeventhOfTheNight,
            _ => HighLatitudeRule::TwilightAngle,
        };
        let times = match PrayerTimes::new(date, coordinates, params) {
            Ok(t) => [
                Prayer::Fajr,
                Prayer::Sunrise,
                Prayer::Dhuhr,
                Prayer::Asr,
                Prayer::Maghrib,
                Prayer::Isha,
            ]
            .map(|p| t.time(p).timestamp().to_string())
            .join(","),
            Err(_) => ["invalid"; 6].join(","),
        };
        writeln!(out, "{line},{times}").unwrap();
    }
}
