//! The Iso8601 type is defined here. It is used in particular within ChainHeader to enforce that
//! their timestamps are defined in a useful and consistent way.

use chrono::{offset::FixedOffset, DateTime, TimeZone};
use error::HolochainError;
use json::JsonString;
use regex::Regex;
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::{convert::TryFrom, fmt, str::FromStr, time::Duration};

/// Represents a timeout for an HDK function. The usize interface defaults to ms.  Also convertible
/// to/from a Duration at full precision.
#[derive(Clone, Deserialize, Debug, Eq, PartialEq, Hash, Serialize, DefaultJson)]
pub struct Timeout(usize);

impl Timeout {
    pub fn new(timeout_ms: usize) -> Self {
        Self(timeout_ms)
    }
}

impl Default for Timeout {
    fn default() -> Timeout {
        Timeout(60000)
    }
}

impl From<Timeout> for Duration {
    fn from(Timeout(millis): Timeout) -> Duration {
        Duration::from_millis(millis as u64)
    }
}

impl From<&Timeout> for Duration {
    fn from(Timeout(millis): &Timeout) -> Duration {
        Duration::from_millis(*millis as u64)
    }
}

impl From<usize> for Timeout {
    fn from(millis: usize) -> Timeout {
        Timeout::new(millis)
    }
}

/// A human-readable time Period, implemented as a time::Duration.  Conversion to/from and
/// Serializable to/from readable form: "1w2d3h4.567s", at full Duration precision; values > 1s w/
/// ms precision are formatted to fractional seconds w/ full precision, while values < 1s are
/// formatted to integer ms, us or ns as appropriate.  Accepts y/yr/year, w/wk/week, d/dy/day,
/// h/hr/hour, m/min/minute, s/sec/second, ms/millis/millisecond, u/μ/micros/microsecond,
/// n/nanos/nanosecond, singlular or plural.  The humantime and parse_duration crates are complex,
/// incompatible with each other, depend on crates and/or do not compile to WASM.
#[derive(Clone, Eq, PartialEq, Hash, DefaultJson)]
pub struct Period(Duration);

/// Serialization w/ serde_json to/from String.  This means that a timestamp will be deserialized to
/// an Period specification and validated, which may fail, returning a serde::de::Error.  Upon
/// serialization, the canonicalized Period specification will be used.
impl Serialize for Period {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'d> Deserialize<'d> for Period {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'d>,
    {
        let s = String::deserialize(deserializer)?;
        Period::from_str(&s).map_err(|e| de::Error::custom(e.to_string()))
    }
}

// The humantime and parse_duration periods are incompatible; We choose compatibility w/ humantime's
// 365.25 days/yr.  An actual year is about 365.242196 days =~= 31,556,925.7344 seconds.  The
// official "leap year" calculation yields (365 + 1/4 - 1/100 + 1/400) * 86,400 == 31,556,952
// seconds/yr.  We're dealing with human-scale time periods with this data structure, so use the
// simpler definition of a year, to avoid seemingly-random remainders when years are involved.
const YR: u64 = 31_557_600_u64;
const WK: u64 = 604_800_u64;
const DY: u64 = 86_400_u64;
const HR: u64 = 3_600_u64;
const MN: u64 = 60_u64;

/// Outputs the human-readable form of the Period's Duration, eg. "1y2w3d4h56m7.89s", "456ms".
/// Debug output of Period specifier instead of underlying Duration seconds.
impl fmt::Debug for Period {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Period({})", self)
    }
}

impl fmt::Display for Period {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let secs = self.0.as_secs();
        let y = secs / YR;
        if y > 0 {
            write!(f, "{}y", y)?
        }
        let y_secs = secs % YR;
        let w = y_secs / WK;
        if w > 0 {
            write!(f, "{}w", w)?
        }
        let w_secs = y_secs % WK;
        let d = w_secs / DY;
        if d > 0 {
            write!(f, "{}d", d)?
        }
        let d_secs = w_secs % DY;
        let h = d_secs / HR;
        if h > 0 {
            write!(f, "{}h", h)?
        }
        let h_secs = d_secs % HR;
        let m = h_secs / MN;
        if m > 0 {
            write!(f, "{}m", m)?
        }
        let s = h_secs % MN;
        let nsecs = self.0.subsec_nanos();
        let is_ns = (nsecs % 1000) > 0;
        let is_us = (nsecs / 1_000 % 1_000) > 0;
        let is_ms = (nsecs / 1_000_000) > 0;
        if (s > 0 && is_ms) || (is_ms && is_ns) {
            // s+ms, or both ms and ns resolution data; default to fractional.
            let ss = format!("{:0>9}", nsecs); // eg.       2100  --> "000002100"
            let ss = ss.trim_end_matches('0'); // eg. "000002100" --> "0000021"
            write!(f, "{}.{}s", s, ss)
        } else if nsecs > 0 || s > 0 {
            // Seconds, and/or sub-seconds remain; auto-scale to s/ms/us/ns, whichever is the finest
            // precision that contains data.
            if s > 0 {
                write!(f, "{}s", s)?;
            }
            if is_ns {
                write!(f, "{}ns", nsecs)
            } else if is_us {
                write!(f, "{}us", nsecs / 1_000)
            } else if is_ms {
                write!(f, "{}ms", nsecs / 1_000_000)
            } else {
                Ok(())
            }
        } else if nsecs == 0 && secs == 0 {
            // A zero Duration
            write!(f, "0s")
        } else {
            // There were either secs or nsecs output above; no further output required
            Ok(())
        }
    }
}

impl FromStr for Period {
    type Err = HolochainError;

    fn from_str(period_str: &str) -> Result<Self, Self::Err> {
        lazy_static! {
            static ref PERIOD_RE: Regex = Regex::new(
                r"(?xi) # whitespace-mode, case-insensitive
                ^
                (?:\s*(?P<y>\d+)\s*y((((ea)?r)s?)?)?)? # y|yr|yrs|year|years
                (?:\s*(?P<w>\d+)\s*w((((ee)?k)s?)?)?)?
                (?:\s*(?P<d>\d+)\s*d((((a )?y)s?)?)?)?
                (?:\s*(?P<h>\d+)\s*h((((ou)?r)s?)?)?)?
                (?:\s*(?P<m>\d+)\s*m((in(ute)?)s?)?)?  # m|min|minute|mins|minutes
                (?:
                  (?:\s* # seconds mantissa (optional) + fraction (required)
                    (?P<s_man>\d+)?
                    [.,](?P<s_fra>\d+)\s*             s((ec(ond)?)s?)?
                  )?
                | (?:
                    (:?\s*(?P<s> \d+)\s*              s((ec(ond)?)s?)?)?
                    (?:\s*(?P<ms>\d+)\s*(m|(milli))   s((ec(ond)?)s?)?)?
                    (?:\s*(?P<us>\d+)\s*(u|μ|(micro)) s((ec(ond)?)s?)?)?
                    (?:\s*(?P<ns>\d+)\s*(n|(nano))    s((ec(ond)?)s?)?)?
                  )
                )
                \s*
                $"
            )
            .unwrap();
        }

        Ok(Period({
            PERIOD_RE.captures(period_str).map_or_else(
                || {
                    Err(HolochainError::ErrorGeneric(format!(
                        "Failed to find Period specification in {:?}",
                        period_str
                    )))
                },
                |cap| {
                    let seconds: u64 = YR
                        * cap
                            .name("y")
                            .map_or("0", |y| y.as_str())
                            .parse::<u64>()
                            .map_err(|e| {
                                HolochainError::ErrorGeneric(format!(
                                    "Invalid year(s) in period {:?}: {:?}",
                                    period_str, e
                                ))
                            })?
                        + WK * cap
                            .name("w")
                            .map_or("0", |w| w.as_str())
                            .parse::<u64>()
                            .map_err(|e| {
                                HolochainError::ErrorGeneric(format!(
                                    "Invalid week(s) in period {:?}: {:?}",
                                    period_str, e
                                ))
                            })?
                        + DY * cap
                            .name("d")
                            .map_or("0", |d| d.as_str())
                            .parse::<u64>()
                            .map_err(|e| {
                                HolochainError::ErrorGeneric(format!(
                                    "Invalid days(s) in period {:?}: {:?}",
                                    period_str, e
                                ))
                            })?
                        + HR * cap
                            .name("h")
                            .map_or("0", |w| w.as_str())
                            .parse::<u64>()
                            .map_err(|e| {
                                HolochainError::ErrorGeneric(format!(
                                    "Invalid hour(s) in period {:?}: {:?}",
                                    period_str, e
                                ))
                            })?
                        + MN * cap
                            .name("m")
                            .map_or("0", |m| m.as_str())
                            .parse::<u64>()
                            .map_err(|e| {
                                HolochainError::ErrorGeneric(format!(
                                    "Invalid minute(s) in period {:?}: {:?}",
                                    period_str, e
                                ))
                            })?
                        + cap
                            .name("s")
                            .map_or_else(
                                || cap.name("s_man").map_or("0", |s_man| s_man.as_str()),
                                |s| s.as_str(),
                            )
                            .parse::<u64>()
                            .map_err(|e| {
                                HolochainError::ErrorGeneric(format!(
                                    "Invalid seconds in period {:?}: {:?}",
                                    period_str, e
                                ))
                            })?;
                    let nanos: u64 = cap
                        .name("s_fra")
                        .map_or(Ok(0_u64), |s_fra| {
                            // ".5" ==> "500000000" (truncate/fill to exactly 9 width)
                            format!("{:0<9.9}", s_fra.as_str()).parse::<u64>()
                        })
                        .map_err(|e| {
                            HolochainError::ErrorGeneric(format!(
                                "Invalid fractional seconds in period {:?}: {:?}",
                                period_str, e
                            ))
                        })?
                        + 1_000_000
                            * cap
                                .name("ms")
                                .map_or("0", |ms| ms.as_str())
                                .parse::<u64>()
                                .map_err(|e| {
                                    HolochainError::ErrorGeneric(format!(
                                        "Invalid milliseconds in period {:?}: {:?}",
                                        period_str, e
                                    ))
                                })?
                        + 1_000
                            * cap
                                .name("us")
                                .map_or("0", |us| us.as_str())
                                .parse::<u64>()
                                .map_err(|e| {
                                    HolochainError::ErrorGeneric(format!(
                                        "Invalid microseconds in period {:?}: {:?}",
                                        period_str, e
                                    ))
                                })?
                        + cap
                            .name("ns")
                            .map_or("0", |ns| ns.as_str())
                            .parse::<u64>()
                            .map_err(|e| {
                                HolochainError::ErrorGeneric(format!(
                                    "Invalid nanoseconds in period {:?}: {:?}",
                                    period_str, e
                                ))
                            })?;
                    // Migrate nanoseconds beyond 1s into seconds, to support specifying larger
                    // Periods in terms of ms, us or ns.
                    Ok(Duration::new(
                        seconds + nanos / 1_000_000_000,
                        (nanos % 1_000_000_000) as u32,
                    ))
                },
            )?
        }))
    }
}

impl TryFrom<String> for Period {
    type Error = HolochainError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Period::from_str(&s)
    }
}

impl TryFrom<&str> for Period {
    type Error = HolochainError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Period::from_str(s)
    }
}

// Conversion of a Period into a Timeout, in ms.  This is an infallible conversion; if the number of
// ms. exceeds the capacity of a usize, default to the maximum possible duration.  Since usize is
// likely a 64-bit value, this will essentially be forever.  Even on 32-bit systems, it will be a
// long duration, but not forever: 2^32/1000 =~= 2^22 seconds =~= 48 days.  We don't want to use
// Duration.as_millis(), because its u128 return type are not supported by WASM.
impl From<Period> for Timeout {
    fn from(p: Period) -> Self {
        Timeout(if p.0.as_secs() as usize >= usize::max_value() / 1000 {
            // The # of seconds overflows the ms-capacity of a usize.  Eg. say a usize could only
            // contain 123,000 ms.; if the number of seconds was >= 123, then 123 * 1000 + 999 ==
            // 123,999 would overflow the capacity, while 122,999 wouldn't.
            usize::max_value()
        } else {
            // We know that secs + 1000 * millis won't overflow a usize.
            (p.0.as_secs() as usize) * 1000 + (p.0.subsec_millis() as usize)
        })
    }
}

// Conversion of Period into Duration
impl From<Period> for Duration {
    fn from(p: Period) -> Self {
        p.0
    }
}

impl From<&Period> for Duration {
    fn from(p: &Period) -> Self {
        p.0.to_owned()
    }
}

/// This struct represents datetime data recovered from a string in the ISO 8601 and RFC 3339 (more
/// restrictive) format.  Invalid try_from conversions fails w/ Result<DateTime<FixedOffset>,
/// HolochainError>.
///
/// Iso8601 wraps a DateTime<FixedOffset>, and its Display/Debug formats default to the ISO 8601 /
/// RFC 3339 format, respectively:
///
///    Display: 2018-10-11T03:23:38+00:00
///    Debug:   Iso8601(2018-10-11T03:23:38+00:00)
///
/// More info on the relevant [wikipedia article](https://en.wikipedia.org/wiki/ISO_8601).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, DefaultJson)]
pub struct Iso8601(DateTime<FixedOffset>);

/// Infallible conversions into and from an Iso8601.  The only infallible way to create an Iso8601
/// is from a Unix timestamp.  An Iso8601 may be converted infallibly to its underlying DateTime<Fixed>

impl From<&Iso8601> for DateTime<FixedOffset> {
    fn from(lhs: &Iso8601) -> DateTime<FixedOffset> {
        lhs.0.to_owned()
    }
}

impl From<Iso8601> for DateTime<FixedOffset> {
    fn from(lhs: Iso8601) -> DateTime<FixedOffset> {
        lhs.0
    }
}

impl From<i64> for Iso8601 {
    fn from(secs: i64) -> Self {
        Self::new(secs, 0)
    }
}

impl From<u64> for Iso8601 {
    fn from(secs: u64) -> Self {
        Self::new(secs as i64, 0)
    }
}

impl From<i32> for Iso8601 {
    fn from(secs: i32) -> Self {
        Self::new(secs.into(), 0)
    }
}

impl From<u32> for Iso8601 {
    fn from(secs: u32) -> Self {
        Self::new(secs.into(), 0)
    }
}

impl Iso8601 {
    fn new(secs: i64, nsecs: u32) -> Self {
        Self(FixedOffset::east(0).timestamp(secs, nsecs))
    }
}

/*
 * Note that the WASM target does not have a reliable and consistent means to obtain the local time,
 * so chrono `now()` methods are unusable: https://github.com/chronotope/chrono/issues/243
 * Therefore, we do not implement a `Iso8601::default()` or `::now()` method at this time.  In
 * addition, supporting internal generated current timestamps is an easy path to non-determinism in
 * holochain Zome functions.  All times should be externally generated, and only *evaluated* by the
 * Zome functions, not generated by them.
 *
 * /// Iso8601::now() and default() return the current Utc time.
 * impl Iso8601 {
 *     pub fn now() -> Iso8601 {
 *         Iso8601::from(Utc::now().to_rfc3339())
 *     }
 * }
 *
 * impl Default for Iso8601 {
 *     fn default() -> Iso8601 {
 *         Iso8601::now()
 *     }
 * }
 */

/// Serialization w/ serde_json to/from String.  This means that a timestamp will be deserialized to
/// an Iso8601 and validated, which may fail, returning a serde::de::Error.  Upon serialization, the
/// canonicalized ISO 8601 / RFC 3339 version of the timestamp will be used.
impl Serialize for Iso8601 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'d> Deserialize<'d> for Iso8601 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'d>,
    {
        let s = String::deserialize(deserializer)?;
        Iso8601::from_str(&s).map_err(|e| de::Error::custom(e.to_string()))
    }
}

/// Outputs the canonicalized ISO 8601 / RFC 3339 form for a valid timestamp.
impl fmt::Display for Iso8601 {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0.to_rfc3339())
    }
}

/// Conversions try_from on String/&str on an Iso8601 are fallible conversions, which may produce a
/// HolochainError if the timestamp is not valid ISO 8601 / RFC 3339.  We will allow some
/// flexibilty; strip surrounding whitespace, a bare timestamp missing any timezone specifier will
/// be assumed to be UTC "Zulu", make internal separators optional if unambiguous.  If you keep to
/// straight RFC 3339 timestamps, then parsing will be quick, otherwise we'll employ a regular
/// expression to parse a more flexible subset of the ISO 8601 standard from your supplied
/// timestamp, and then use the RFC 3339 parser again.  We only do this validation once; at the
/// creation of an Iso8601 from a String/&str.
impl TryFrom<String> for Iso8601 {
    type Error = HolochainError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Iso8601::from_str(&s)
    }
}

impl TryFrom<&str> for Iso8601 {
    type Error = HolochainError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Iso8601::from_str(s)
    }
}

impl FromStr for Iso8601 {
    type Err = HolochainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        lazy_static! {
            static ref ISO8601_RE: Regex = Regex::new(
                r"(?x)         # whitespace-mode
                ^
                \s*
                (?P<Y>\d{4})
                (?:            # Always require 4-digit year and double-digit mon/day YYYY[[-]MM[[-]DD]]
                  -?
                  (?P<M>
                     0[1-9]
                   | 1[012]
                  )?
                  (?:
                    -?
                    (?P<D>
                        0[1-9]
                      | [12][0-9]
                      | 3[01]
                    )?
                  )?
                )?
                (?:
                  (?:           # Optional T or space(s)
                    [Tt]
                  | \s+
                  )
                  (?P<h>        # Requires two-digit HH[[:]MM[[:]SS]] w/ consistent optional separators
                    [01][0-9]
                  | 2[0-3]      # but do not support 24:00:00 to designate end-of-day midnight
                  )
                  (?:
                    :?
                    (?P<m>
                      [0-5][0-9]
                    )
                    (?:         # The whole seconds group is optional, implies 00
                      :?
                      (?P<s>
                        (?:
                          [0-5][0-9]
                        | 60    # Support leap-seconds for standards compliance
                        )
                      )
                      (?:
                        [.,]    # Optional subseconds, separated by either ./, (always supply ., below)
                        (?P<ss>
                          \d+
                        )
                      )?
                    )?
                  )?
                )?
                \s*
                (?P<Z>          # no timezone specifier implies Z
                   [Zz]
                 | (?P<Zsgn>[+-−]) # Zone sign allows UTF8 minus or ASCII hyphen as per RFC/ISO
                   (?P<Zhrs>\d{2}) # and always double-digit hours offset required
                   (?:             # but if double-digit minutes supplied, colon optional
                     :?
                     (?P<Zmin>\d{2})
                   )?
                )?
                \s*
                $"
            )
            .unwrap();
        }

        Ok(Iso8601(
            DateTime::parse_from_rfc3339(s)
                .or_else(
                    |_| ISO8601_RE.captures(s)
                        .map_or_else(
                            || Err(HolochainError::ErrorGeneric(
                                format!("Failed to find ISO 3339 or RFC 8601 timestamp in {:?}", s))),
                            |cap| {
                                let timestamp = &format!(
                                    "{:0>4}-{:0>2}-{:0>2}T{:0>2}:{:0>2}:{:0>2}{}{}",
                                    &cap["Y"],
                                    cap.name("M").map_or( "1", |m| m.as_str()),
                                    cap.name("D").map_or( "1", |m| m.as_str()),
                                    cap.name("h").map_or( "0", |m| m.as_str()),
                                    cap.name("m").map_or( "0", |m| m.as_str()),
                                    cap.name("s").map_or( "0", |m| m.as_str()),
                                    cap.name("ss").map_or( "".to_string(), |m| format!(".{}", m.as_str())),
                                    cap.name("Z").map_or( "Z".to_string(), |m| match m.as_str() {
                                        "Z"|"z" => "Z".to_string(),
                                        _ => format!(
                                            "{}{}:{}",
                                            match &cap["Zsgn"] { "+" => "+", _ => "-" },
                                            &cap["Zhrs"],
                                            &cap.name("Zmin").map_or( "00", |m| m.as_str()))
                                    }));

                                DateTime::parse_from_rfc3339(timestamp)
                                    .map_err(|_| HolochainError::ErrorGeneric(
                                        format!("Attempting to convert RFC 3339 timestamp {:?} from ISO 8601 {:?} to a DateTime",
                                                timestamp, s)))
                            }
                        )
                )?
        ))
    }
}

// The only infallible conversions are from an i64 UNIX timestamp.  There are no conversions from
// String or &str that are infallible.
//
//     $ date  --date="2018-10-11T03:23:38+00:00" +%s
//     1539228218
//     $ date --date=@1539228218 --rfc-3339=seconds --utc
//     2018-10-11 03:23:38+00:00
//
pub fn test_iso_8601() -> Iso8601 {
    Iso8601::from(1539228218) // 2018-10-11T03:23:38+00:00
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use serde_json;
    use std::convert::TryInto;

    #[test]
    fn test_period_basic() {
        assert_eq!(format!("{}", Period(Duration::from_millis(0))), "0s");
        assert_eq!(format!("{}", Period(Duration::from_millis(123))), "123ms");
        assert_eq!(format!("{}", Period(Duration::from_nanos(120000))), "120us");
        assert_eq!(format!("{}", Period(Duration::from_nanos(100))), "100ns");
        assert_eq!(
            format!("{}", Period(Duration::from_millis(1000 * 604_800 + 1123))),
            "1w1.123s"
        );
        assert_eq!(
            format!("{}", Period(Duration::from_millis(1000 * 604_800 + 123))),
            "1w123ms"
        );
        assert_eq!(
            format!(
                "{}",
                Period(Duration::from_nanos(
                    (2 * YR + 3 * WK + 4 * DY + 5 * HR + 6 * MN + 7) * 1_000_000_000_u64
                        + 123456789
                ))
            ),
            "2y3w4d5h6m7.123456789s"
        );

        // Errors; cannot mix fractional seconds and ms/ns/us
        assert_eq!(
            Period::from_str("1.23s456ns"),
            Err(HolochainError::ErrorGeneric(
                "Failed to find Period specification in \"1.23s456ns\"".to_string()
            ))
        );
        // time scale ordering cannot be mixed up
        assert_eq!(
            Period::from_str("456ns123us"),
            Err(HolochainError::ErrorGeneric(
                "Failed to find Period specification in \"456ns123us\"".to_string()
            ))
        );

        // Canonicalization, incl. case insensitivity, long names, plurals
        vec![
            // Elide empty smaller timespans
            ("1 week", Duration::new(1 * WK, 0), "1w"),
            // 1y == 364.25d
            (
                "123w456ns",
                Duration::new(123 * WK, 456_u32),
                "2y18w4d12h456ns",
            ),
            (
                "2y18w4d12h0.000003456s",
                Duration::new(123 * WK, 3456_u32),
                "2y18w4d12h3456ns",
            ),
            (
                "2 years 18 Weeks 4 dy 12 hrs 0.000456 SEC",
                Duration::new(123 * WK, 456_u32 * 1000),
                "2y18w4d12h456us",
            ),
            // Truncation beyond ns precision
            (
                "2y18w4d12h0.00000345678s",
                Duration::new(123 * WK, 3456_u32),
                "2y18w4d12h3456ns",
            ),
            // ms/us/ns beyond 1s supported
            (
                "1y60000ms25μs100nanos",
                Duration::new(1 * YR + 60, 25100_u32),
                "1y1m25100ns",
            ),
            // sub-second ranging into appropriate ms/us/ns.
            (
                "600millisecond25usecs100nanos",
                Duration::new(0, 600025100_u32),
                "0.6000251s",
            ),
            ("25us100ns", Duration::new(0, 25100_u32), "25100ns"),
            (".0000251s", Duration::new(0, 25100_u32), "25100ns"),
            (".000025s", Duration::new(0, 25000_u32), "25us"),
            (
                "1y2w3d4h5m6s7ms8us9ns",
                Duration::new(YR + 2 * WK + 3 * DY + 4 * HR + 5 * MN + 6, 7008009),
                "1y2w3d4h5m6.007008009s",
            ),
            (
                "1yr2wk3dy4hr5min6sec7msec8μsec9nsec",
                Duration::new(YR + 2 * WK + 3 * DY + 4 * HR + 5 * MN + 6, 7008009),
                "1y2w3d4h5m6.007008009s",
            ),
            (
                "1year2week3day4hour5minute6second7msecond8usecond9nsecond",
                Duration::new(YR + 2 * WK + 3 * DY + 4 * HR + 5 * MN + 6, 7008009),
                "1y2w3d4h5m6.007008009s",
            ),
            (
                "1years2weeks3days4hours5minutes6seconds7milliseconds8microseconds9nanoseconds",
                Duration::new(YR + 2 * WK + 3 * DY + 4 * HR + 5 * MN + 6, 7008009),
                "1y2w3d4h5m6.007008009s",
            ),
            (
                "1 yrs 2 wks 3 dys 4 hrs 5 mins 6 secs 7 millis 8 micros 9 nanos ",
                Duration::new(YR + 2 * WK + 3 * DY + 4 * HR + 5 * MN + 6, 7008009),
                "1y2w3d4h5m6.007008009s",
            ),
        ]
        .iter()
        .map(|(ps, dur, ps_out)| {
            let period = Period::try_from(*ps)?;

            // Conversion into/from Duration
            let duration: Duration = period.clone().into();
            assert_eq!(duration, *dur);
            let period_from_duration = Period(duration);
            assert_eq!(period_from_duration, period);

            // Debug formatting just encapsulates canonicalized human-readable period specifier
            assert_eq!(format!("{:?}", period), format!("Period({})", ps_out));

            // Basic to/from String serialization
            assert_eq!(period, Period(*dur));
            assert_eq!(&period.to_string(), ps_out);

            // JSON serialization via serde_json
            let serialized = serde_json::to_string(&period)?;
            assert_eq!(serialized.to_string(), format!("\"{}\"", ps_out));
            let deserialized: Period = serde_json::from_str(&serialized.to_string())?;
            assert_eq!(&deserialized.to_string(), ps_out);

            // JSON serialization via JsonSring
            let period_ser = JsonString::from(&period);
            assert_eq!(period_ser.to_string(), format!("\"{}\"", ps_out));
            let period_des = Period::try_from(period_ser);
            assert!(period_des.is_ok());
            assert_eq!(&period_des.unwrap().to_string(), ps_out);

            // JSON round-tripping w/o serde or intermediates
            assert_eq!(Period::try_from(JsonString::from(period))?, Period(*dur));

            Ok(())
        })
        .collect::<Result<(()), HolochainError>>()
        .map_err(|e| panic!("Unexpected failure of checked Period::try_from: {:?}", e))
        .unwrap();
    }

    #[test]
    fn test_period_timeout() {
        let period = Period::try_from("1w1.23s").unwrap();

        // We can specify timeouts in human-readable Periods
        assert_eq!(Timeout::from(period), Timeout(1230 + 1000 * WK as usize));
    }

    #[test]
    fn test_iso_8601_basic() {
        // Different ways of specifying UTC "Zulu".  A bare timestamp will be defaulted to "Zulu".
        vec![
            "2018-10-11T03:23:38 +00:00",
            "2018-10-11T03:23:38Z",
            "2018-10-11T03:23:38",
            "2018-10-11T03:23:38+00",
            "2018-10-11 03:23:38",
        ]
        .iter()
        .map(|ts| {
            // Check that mapping from an Iso8601::(&str) to a DateTime yields the expected RFC 3339
            // / ISO 8601 timestamp, via its DateTime<FixedOffset>, its fmt::Display, to_string()
            // and JSON round-trip.
            Iso8601::try_from(*ts)
                .and_then(|iso| {
                    assert_eq!(iso.to_string(), "2018-10-11T03:23:38+00:00");
                    Ok(iso)
                })
                .and_then(|iso| {
                    assert_eq!(
                        format!(
                            "{}",
                            DateTime::<FixedOffset>::from(iso.clone()).to_rfc3339()
                        ),
                        "2018-10-11T03:23:38+00:00"
                    );
                    Ok(iso)
                })
                .and_then(|iso| {
                    assert_eq!(iso.to_string(), "2018-10-11T03:23:38+00:00");
                    Ok(iso)
                })
                .and_then(|iso| {
                    // JSON serialization via serde_json
                    let serialized = serde_json::to_string(&iso)?;
                    assert_eq!(serialized.to_string(), "\"2018-10-11T03:23:38+00:00\"");
                    let deserialized: Iso8601 = serde_json::from_str(&serialized.to_string())?;
                    assert_eq!(deserialized.to_string(), "2018-10-11T03:23:38+00:00");

                    // JSON serialization via JsonSring
                    let iso_8601_ser = JsonString::from(&iso);
                    assert_eq!(iso_8601_ser.to_string(), "\"2018-10-11T03:23:38+00:00\"");
                    let iso_8601_des = Iso8601::try_from(iso_8601_ser);
                    assert!(iso_8601_des.is_ok());
                    assert_eq!(
                        iso_8601_des.unwrap().to_string(),
                        "2018-10-11T03:23:38+00:00"
                    );

                    // JSON round-tripping w/o serde or intermediates
                    assert_eq!(
                        Iso8601::try_from(JsonString::from(iso)),
                        Iso8601::try_from("2018-10-11T03:23:38+00:00")
                    );

                    Ok(())
                })
        })
        .collect::<Result<(()), HolochainError>>()
        .map_err(|e| {
            panic!(
                "Unexpected failure of checked DateTime<FixedOffset> try_from: {:?}",
                e
            )
        })
        .unwrap();

        vec![
            "20180101 0323",
            "2018-01-01 0323",
            "2018 0323",
            "2018-- 0323",
            "2018-01-01 032300",
            "2018-01-01 03:23",
            "2018-01-01 03:23:00",
            "2018-01-01 03:23:00 Z",
            "2018-01-01 03:23:00 +00",
            "2018-01-01 03:23:00 +00:00",
        ]
        .iter()
        .map(|ts| {
            Iso8601::try_from(*ts)
                .and_then(|iso| Ok(assert_eq!(iso.to_string(), "2018-01-01T03:23:00+00:00")))
        })
        .collect::<Result<(()), HolochainError>>()
        .map_err(|e| {
            panic!(
                "Unexpected failure of checked DateTime<FixedOffset> try_from: {:?}",
                e
            )
        })
        .unwrap();

        // Leap-seconds and sub-second times, in both native RFC 3339 and (Regex-based) ISO 8601.
        // Also exercise the HHMM60 methods for specifying times that extend into the following time
        // period.  Specifically does not support the "24:00:00" times.  Also tests the use of UTF8
        // minus in addition to ASCII hyphen.
        vec![
            "2015-02-18T23:59:60.234567-05:00",
            "2015-02-18T23:59:60.234567−05:00",
            "2015-02-18 235960.234567 -05",
            "20150218 235960.234567 −05",
            "20150218 235960,234567 −05",
        ]
        .iter()
        .map(|ts| {
            let iso_8601 = Iso8601::try_from(*ts)?;
            let dt = DateTime::<FixedOffset>::from(&iso_8601); // from &Iso8601
            Ok(assert_eq!(
                dt.to_rfc3339().to_string(),
                "2015-02-18T23:59:60.234567-05:00"
            ))
        })
        .collect::<Result<(()), HolochainError>>()
        .map_err(|e| {
            panic!(
                "Unexpected failure of checked DateTime<FixedOffset> try_from: {:?}",
                e
            )
        })
        .unwrap();

        // Now test a bunch that should fail
        vec![
            "boo",
            "2015-02-18T23:59:60.234567-5",
            "2015-02-18 3:59:60-05",
            "2015-2-18 03:59:60-05",
            "2015-2-18 03:59:60+25",
        ]
        .iter()
        .map(|ts| match Iso8601::try_from(*ts) {
            Ok(iso) => Err(HolochainError::ErrorGeneric(format!(
                "Should not have succeeded in parsing {:?} into {:?}",
                ts, iso
            ))),
            Err(_) => Ok(()),
        })
        .collect::<Result<(()), HolochainError>>()
        .map_err(|e| {
            panic!(
                "Unexpected success of invalid checked DateTime<FixedOffset> try_from: {:?}",
                e
            )
        })
        .unwrap();

        // PartialEq and PartialOrd Comparison operators
        assert!(
            Iso8601::try_from("2018-10-11T03:23:38+00:00").unwrap()
                == Iso8601::try_from("2018-10-11T03:23:38Z").unwrap()
        );
        assert!(
            Iso8601::try_from("2018-10-11T03:23:38").unwrap()
                == Iso8601::try_from("2018-10-11T03:23:38Z").unwrap()
        );
        assert!(
            Iso8601::try_from(" 20181011  0323  Z ").unwrap()
                == Iso8601::try_from("2018-10-11T03:23:00Z").unwrap()
        );

        // Fixed-offset ISO 8601 are comparable to UTC times
        assert!(
            Iso8601::try_from("2018-10-11T03:23:38-08:00").unwrap()
                == Iso8601::try_from("2018-10-11T11:23:38Z").unwrap()
        );
        assert!(
            Iso8601::try_from("2018-10-11T03:23:39-08:00").unwrap()
                > Iso8601::try_from("2018-10-11T11:23:38Z").unwrap()
        );
        assert!(
            Iso8601::try_from("2018-10-11T03:23:37-08:00").unwrap()
                < Iso8601::try_from("2018-10-11T11:23:38Z").unwrap()
        );

        match Iso8601::try_from("boo") {
            Ok(iso) => panic!(
                "Unexpected success of checked DateTime<FixedOffset> try_from: {:?}",
                iso
            ),
            Err(e) => assert_eq!(
                e.to_string(),
                "Failed to find ISO 3339 or RFC 8601 timestamp in \"boo\""
            ),
        }
    }

    #[test]
    fn test_iso_8601_sorting() {
        // Different ways of specifying UTC "Zulu".  A bare timestamp will be defaulted to "Zulu".
        let mut v: Vec<Iso8601> = vec![
            "2018-10-11T03:23:39-08:00".try_into().unwrap(),
            "2018-10-11T03:23:39-07:00".try_into().unwrap(),
            "2018-10-11 03:23:39+03:00".try_into().unwrap(),
            "2018-10-11T03:23:39-06:00".try_into().unwrap(),
            "20181011 032339 +04:00".try_into().unwrap(),
            "2018-10-11T03:23:39−09:00".try_into().unwrap(), // note the UTF8 minus instead of ASCII hyphen
            "2018-10-11T03:23:39+11:00".try_into().unwrap(),
            "2018-10-11 03:23:39Z".try_into().unwrap(),
            "2018-10-11 03:23:40".try_into().unwrap(),
        ];
        v.sort_by(|a, b| {
            let cmp = a.cmp(b);
            //println!( "{} {:?} {}", a, cmp, b );
            cmp
        });
        assert_eq!(
            v.iter()
                .map(|ts| format!("{:?}", &ts).to_string())
                .collect::<Vec<String>>()
                .join(", "),
            concat!(
                "Iso8601(2018-10-11T03:23:39+11:00), ",
                "Iso8601(2018-10-11T03:23:39+04:00), ",
                "Iso8601(2018-10-11T03:23:39+03:00), ",
                "Iso8601(2018-10-11T03:23:39+00:00), ",
                "Iso8601(2018-10-11T03:23:40+00:00), ",
                "Iso8601(2018-10-11T03:23:39-06:00), ",
                "Iso8601(2018-10-11T03:23:39-07:00), ",
                "Iso8601(2018-10-11T03:23:39-08:00), ",
                "Iso8601(2018-10-11T03:23:39-09:00)"
            )
        );

        v.sort_by(|a, b| b.cmp(a)); // reverse
        assert_eq!(
            v.iter()
                .map(|ts| format!("{:?}", &ts).to_string())
                .collect::<Vec<String>>()
                .join(", "),
            concat!(
                "Iso8601(2018-10-11T03:23:39-09:00), ",
                "Iso8601(2018-10-11T03:23:39-08:00), ",
                "Iso8601(2018-10-11T03:23:39-07:00), ",
                "Iso8601(2018-10-11T03:23:39-06:00), ",
                "Iso8601(2018-10-11T03:23:40+00:00), ",
                "Iso8601(2018-10-11T03:23:39+00:00), ",
                "Iso8601(2018-10-11T03:23:39+03:00), ",
                "Iso8601(2018-10-11T03:23:39+04:00), ",
                "Iso8601(2018-10-11T03:23:39+11:00)"
            )
        );
    }
}
