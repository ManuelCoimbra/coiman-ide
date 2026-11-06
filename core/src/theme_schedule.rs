//! Color themes by time of day, written as `07:00 Light Modern; sunset Dark Modern`.
//! Sunrise and sunset are worked out here, from a location typed in settings.

/// What the sun does here today.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sun {
    /// No location is set.
    Unknown,
    /// It doesn't rise and set today: far north or south.
    Polar,
    /// Sunrise and sunset, in minutes of the local day.
    Times(u32, u32),
}

/// Times of day, in minutes, with the theme from then on, earliest first.
pub fn parse(text: &str, sun: Sun) -> Result<Vec<(u32, String)>, String> {
    let mut list = Vec::new();
    for entry in text.split(';').map(str::trim).filter(|e| !e.is_empty()) {
        let wrong = || format!("Theme schedule: \"{entry}\" should look like 07:00 Light Modern or sunset Dark Modern");
        let (time, theme) = entry.split_once(char::is_whitespace).ok_or_else(wrong)?;
        if theme.trim().is_empty() {
            return Err(wrong());
        }
        let minute = match (time.to_lowercase().as_str(), sun) {
            ("sunrise" | "sunset", Sun::Unknown) => {
                return Err("Theme schedule: sunrise and sunset need a location, like 38.7, -9.1".to_owned());
            }
            // With no sunrise today, what's left of the list applies.
            ("sunrise" | "sunset", Sun::Polar) => continue,
            ("sunrise", Sun::Times(rise, _)) => rise,
            ("sunset", Sun::Times(_, set)) => set,
            _ => {
                let (hours, minutes) = time.split_once(':').ok_or_else(wrong)?;
                let hours: u32 = hours.parse().map_err(|_| wrong())?;
                let minutes: u32 = minutes.parse().map_err(|_| wrong())?;
                if hours > 23 || minutes > 59 {
                    return Err(wrong());
                }
                hours * 60 + minutes
            }
        };
        list.push((minute, theme.trim().to_owned()));
    }
    list.sort_by_key(|e| e.0);
    Ok(list)
}

/// The theme for a minute of the day: the last one to start, from yesterday if none has yet.
pub fn theme_at(list: &[(u32, String)], minute: u32) -> Option<&str> {
    list.iter().rev().find(|(at, _)| *at <= minute).or_else(|| list.last()).map(|(_, theme)| theme.as_str())
}

/// A location as typed: `38.7, -9.1`, latitude first.
pub fn parse_location(text: &str) -> Result<Option<(f64, f64)>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let wrong = || format!("Theme location: \"{text}\" should be latitude, longitude, like 38.7, -9.1");
    let (latitude, longitude) = text.split_once(',').ok_or_else(wrong)?;
    let latitude: f64 = latitude.trim().parse().map_err(|_| wrong())?;
    let longitude: f64 = longitude.trim().parse().map_err(|_| wrong())?;
    if latitude.abs() > 90.0 || longitude.abs() > 180.0 {
        return Err(wrong());
    }
    Ok(Some((latitude, longitude)))
}

/// Sunrise and sunset by NOAA's approximation, good to a few minutes. `day` is the day of the
/// year from 1, and `offset` the local clock's minutes ahead of UTC.
pub fn sun_times(latitude: f64, longitude: f64, day: u32, offset: i32) -> Sun {
    let year = 2.0 * std::f64::consts::PI / 365.0 * (f64::from(day) - 1.0);
    let equation = 229.18
        * (0.000075 + 0.001868 * year.cos()
            - 0.032077 * year.sin()
            - 0.014615 * (2.0 * year).cos()
            - 0.040849 * (2.0 * year).sin());
    let declination = 0.006918 - 0.399912 * year.cos() + 0.070257 * year.sin() - 0.006758 * (2.0 * year).cos()
        + 0.000907 * (2.0 * year).sin()
        - 0.002697 * (3.0 * year).cos()
        + 0.00148 * (3.0 * year).sin();
    let phi = latitude.to_radians();
    let cos_hour = 90.833_f64.to_radians().cos() / (phi.cos() * declination.cos()) - phi.tan() * declination.tan();
    if cos_hour.abs() > 1.0 {
        return Sun::Polar;
    }
    let hour = cos_hour.acos().to_degrees();
    let local = |utc: f64| (utc.round() as i32 + offset).rem_euclid(1440) as u32;
    Sun::Times(local(720.0 - 4.0 * (longitude + hour) - equation), local(720.0 - 4.0 * (longitude - hour) - equation))
}

/// What the sun does today at a location, by this computer's clock.
pub fn sun_today(location: Option<(f64, f64)>) -> Sun {
    use chrono::{Datelike, Offset};
    let Some((latitude, longitude)) = location else { return Sun::Unknown };
    let now = chrono::Local::now();
    sun_times(latitude, longitude, now.ordinal(), now.offset().fix().local_minus_utc() / 60)
}

/// The minute of the day here, by this computer's clock.
pub fn minute_now() -> u32 {
    use chrono::Timelike;
    chrono::Local::now().num_seconds_from_midnight() / 60
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_schedule_in_any_order() {
        let list = parse(" 19:00 Dark Modern ;07:30  Light Modern; ", Sun::Unknown).unwrap();
        assert_eq!(list, [(450, "Light Modern".to_owned()), (1140, "Dark Modern".to_owned())]);
        assert!(parse("", Sun::Unknown).unwrap().is_empty());
    }

    #[test]
    fn says_what_is_wrong() {
        for bad in ["07:00", "seven Dark", "25:00 Dark", "07:99 Dark", "0700 Dark"] {
            assert!(parse(bad, Sun::Unknown).unwrap_err().contains(bad), "{bad}");
        }
    }

    #[test]
    fn the_last_to_start_applies_and_nights_wrap() {
        let list = parse("07:00 Light; 19:00 Dark", Sun::Unknown).unwrap();
        assert_eq!(theme_at(&list, 7 * 60), Some("Light"));
        assert_eq!(theme_at(&list, 18 * 60 + 59), Some("Light"));
        assert_eq!(theme_at(&list, 19 * 60), Some("Dark"));
        assert_eq!(theme_at(&list, 3 * 60), Some("Dark"));
        assert_eq!(theme_at(&[], 3), None);
    }

    #[test]
    fn the_clock_gives_a_minute_of_the_day() {
        assert!(minute_now() < 24 * 60);
    }

    #[test]
    fn sunrise_and_sunset_stand_for_their_times() {
        let list = parse("sunset Dark; SUNRISE Light", Sun::Times(400, 1200)).unwrap();
        assert_eq!(list, [(400, "Light".to_owned()), (1200, "Dark".to_owned())]);
        assert!(parse("sunset Dark", Sun::Unknown).unwrap_err().contains("location"));
        let polar = parse("sunrise Light; 12:00 Mid", Sun::Polar).unwrap();
        assert_eq!(polar, [(720, "Mid".to_owned())]);
    }

    #[test]
    fn reads_a_location() {
        assert_eq!(parse_location(" 38.7, -9.1 ").unwrap(), Some((38.7, -9.1)));
        assert_eq!(parse_location("").unwrap(), None);
        for bad in ["38.7", "x, y", "91, 0", "0, 181"] {
            assert!(parse_location(bad).is_err(), "{bad}");
        }
    }

    fn near(sun: Sun, rise: u32, set: u32) {
        let Sun::Times(found_rise, found_set) = sun else { panic!("{sun:?}") };
        assert!(found_rise.abs_diff(rise) <= 5, "sunrise {found_rise}, about {rise}");
        assert!(found_set.abs_diff(set) <= 5, "sunset {found_set}, about {set}");
    }

    #[test]
    fn the_sun_keeps_its_published_times() {
        // London at midsummer: 03:43 and 20:21 UTC.
        near(sun_times(51.5, -0.12, 172, 0), 223, 1221);
        // The same in British Summer Time.
        near(sun_times(51.5, -0.12, 172, 60), 283, 1281);
        // Sydney at its midwinter: 07:00 and 16:54 local, ten hours ahead.
        near(sun_times(-33.87, 151.21, 172, 600), 420, 1014);
    }

    #[test]
    fn far_north_has_no_sunset_in_summer() {
        assert_eq!(sun_times(78.2, 15.6, 172, 120), Sun::Polar);
        assert_eq!(sun_today(None), Sun::Unknown);
    }
}
