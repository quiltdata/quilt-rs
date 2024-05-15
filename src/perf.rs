use chrono::DateTime;
use chrono::TimeDelta;
use chrono::Utc;
use std::fmt::Display;

pub struct Measure {
    delta: Option<TimeDelta>,
    now: DateTime<Utc>,
}

impl Measure {
    pub fn start() -> Self {
        Measure {
            delta: None,
            now: chrono::Utc::now(),
        }
    }

    pub fn elapsed(&self) -> Self {
        let now = chrono::Utc::now();
        Measure {
            delta: Some(now - self.now),
            now,
        }
    }
}

fn cut_days(d: TimeDelta) -> (Option<i64>, TimeDelta) {
    let days = d.num_days();
    if days > 0 {
        (Some(days), d - TimeDelta::days(days))
    } else {
        (None, d)
    }
}

fn cut_hours(d: TimeDelta) -> (Option<i64>, TimeDelta) {
    let hours = d.num_hours();
    if hours > 0 {
        (Some(hours), d - TimeDelta::hours(hours))
    } else {
        (None, d)
    }
}

fn cut_minutes(d: TimeDelta) -> (Option<i64>, TimeDelta) {
    let minutes = d.num_minutes();
    if minutes > 0 {
        (Some(minutes), d - TimeDelta::minutes(minutes))
    } else {
        (None, d)
    }
}

fn cut_seconds(d: TimeDelta) -> (Option<i64>, TimeDelta) {
    let seconds = d.num_seconds();
    if seconds > 0 {
        (Some(seconds), d - TimeDelta::seconds(seconds))
    } else {
        (None, d)
    }
}

pub fn format_delta(d: TimeDelta) -> String {
    let mut output = Vec::new();

    let (days, d) = cut_days(d);
    if let Some(days) = days {
        output.push(format!("{}d", days));
    }

    let (hours, d) = cut_hours(d);
    if let Some(hours) = hours {
        output.push(format!("{}h", hours));
    }

    let (minutes, d) = cut_minutes(d);
    if let Some(minutes) = minutes {
        output.push(format!("{}m", minutes));
    }

    let (seconds, d) = cut_seconds(d);
    if let Some(seconds) = seconds {
        output.push(format!("{}s", seconds));
    } else {
        let milliseconds = d.num_milliseconds();
        output.push(format!("{}ms", milliseconds));
    }

    output.join(" ").to_string()
}

impl Display for Measure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.delta {
            Some(delta) => write!(f, "{}", format_delta(delta)),
            None => write!(f, "No delta"),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_days() {
        let delta = TimeDelta::days(3)
            + TimeDelta::hours(19)
            + TimeDelta::minutes(45)
            + TimeDelta::seconds(39);
        assert_eq!(format_delta(delta), "3d 19h 45m 39s".to_string())
    }

    #[test]
    fn test_seconds() {
        let delta = TimeDelta::seconds(1);
        assert_eq!(format_delta(delta), "1s".to_string())
    }

    #[test]
    fn test_nanoseconds() {
        let delta = TimeDelta::milliseconds(130);
        assert_eq!(format_delta(delta), "130ms".to_string())
    }

    #[test]
    fn test_overflowing() {
        let delta = TimeDelta::days(1)
            + TimeDelta::hours(25)
            + TimeDelta::minutes(75)
            + TimeDelta::seconds(87);
        assert_eq!(format_delta(delta), "2d 2h 16m 27s".to_string())
    }
}
