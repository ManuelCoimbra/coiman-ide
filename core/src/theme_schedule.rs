//! Color themes by time of day, written as `07:00 Light Modern; 19:00 Dark Modern`.

/// Times of day, in minutes, with the theme from then on, earliest first.
pub fn parse(text: &str) -> Result<Vec<(u32, String)>, String> {
    let mut list = Vec::new();
    for entry in text.split(';').map(str::trim).filter(|e| !e.is_empty()) {
        let wrong = || format!("Theme schedule: \"{entry}\" should look like 07:00 Light Modern");
        let (time, theme) = entry.split_once(char::is_whitespace).ok_or_else(wrong)?;
        let (hours, minutes) = time.split_once(':').ok_or_else(wrong)?;
        let hours: u32 = hours.parse().map_err(|_| wrong())?;
        let minutes: u32 = minutes.parse().map_err(|_| wrong())?;
        if hours > 23 || minutes > 59 || theme.trim().is_empty() {
            return Err(wrong());
        }
        list.push((hours * 60 + minutes, theme.trim().to_owned()));
    }
    list.sort_by_key(|e| e.0);
    Ok(list)
}

/// The theme for a minute of the day: the last one to start, from yesterday if none has yet.
pub fn theme_at(list: &[(u32, String)], minute: u32) -> Option<&str> {
    list.iter().rev().find(|(at, _)| *at <= minute).or_else(|| list.last()).map(|(_, theme)| theme.as_str())
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
        let list = parse(" 19:00 Dark Modern ;07:30  Light Modern; ").unwrap();
        assert_eq!(list, [(450, "Light Modern".to_owned()), (1140, "Dark Modern".to_owned())]);
        assert!(parse("").unwrap().is_empty());
    }

    #[test]
    fn says_what_is_wrong() {
        for bad in ["07:00", "seven Dark", "25:00 Dark", "07:99 Dark", "0700 Dark"] {
            assert!(parse(bad).unwrap_err().contains(bad), "{bad}");
        }
    }

    #[test]
    fn the_last_to_start_applies_and_nights_wrap() {
        let list = parse("07:00 Light; 19:00 Dark").unwrap();
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
}
