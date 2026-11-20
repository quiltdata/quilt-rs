//! Time as quilt-rs writes it.
//!
//! Each place a time leaves the process keeps one text shape, since older
//! versions read the same files. Serde fields use [`json`] (or
//! [`json::option`]); the tests pin each shape.

use jiff::Timestamp;
use jiff::fmt::temporal::DateTimePrinter;

/// RFC 3339 in UTC with `Z`, sub-second digits in groups of three: none when
/// the fraction is zero, else the fewest of 3, 6 or 9 that hold it.
#[must_use]
pub fn rfc3339(time: Timestamp) -> String {
    let nanos = time.subsec_nanosecond();
    let digits = if nanos == 0 {
        0
    } else if nanos % 1_000_000 == 0 {
        3
    } else if nanos % 1_000 == 0 {
        6
    } else {
        9
    };
    DateTimePrinter::new()
        .precision(Some(digits))
        .timestamp_to_string(&time)
}

/// Whole Unix seconds, rounded down, also before 1970.
#[must_use]
pub fn whole_seconds(time: Timestamp) -> i64 {
    let seconds = time.as_second();
    if time.subsec_nanosecond() < 0 {
        seconds - 1
    } else {
        seconds
    }
}

/// `time` rounded down to the second.
#[must_use]
pub fn to_second(time: Timestamp) -> Timestamp {
    Timestamp::from_second(whole_seconds(time)).unwrap_or(time)
}

/// `time` from the standard library's clock type.
///
/// # Errors
///
/// When `time` is outside the years a [`Timestamp`] holds.
pub fn from_system(time: std::time::SystemTime) -> std::io::Result<Timestamp> {
    Timestamp::try_from(time).map_err(std::io::Error::other)
}

/// Serde for a [`Timestamp`] in the [`rfc3339`] shape. Reads any RFC 3339
/// offset.
pub mod json {
    use jiff::Timestamp;
    use serde::Deserialize;
    use serde::Deserializer;
    use serde::Serializer;

    /// # Errors
    ///
    /// When the serializer fails.
    pub fn serialize<S: Serializer>(time: &Timestamp, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&super::rfc3339(*time))
    }

    /// # Errors
    ///
    /// When the value is not an RFC 3339 time.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Timestamp, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }

    /// The same for an `Option`, `null` for `None`.
    pub mod option {
        use jiff::Timestamp;
        use serde::Deserialize;
        use serde::Deserializer;
        use serde::Serializer;

        /// # Errors
        ///
        /// When the serializer fails.
        pub fn serialize<S: Serializer>(
            time: &Option<Timestamp>,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            match time {
                Some(time) => serializer.serialize_some(&super::super::rfc3339(*time)),
                None => serializer.serialize_none(),
            }
        }

        /// # Errors
        ///
        /// When the value is neither `null` nor an RFC 3339 time.
        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<Timestamp>, D::Error> {
            <Option<std::borrow::Cow<'de, str>>>::deserialize(deserializer)?
                .map(|text| text.parse().map_err(serde::de::Error::custom))
                .transpose()
        }
    }
}

#[cfg(test)]
mod tests;
