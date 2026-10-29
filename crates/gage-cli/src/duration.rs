//! The duration grammar and spelling shared by the settings, the clipboard
//! notice, enrollment TTLs and `vault ls`-style idle readouts. The library
//! takes `std::time::Duration`; the text form is the frontend's.

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationError {
    Malformed,
    Negative,
}

/// The duration grammar the settings are documented in: an optional sign, then one or more `<decimal><unit>`
/// terms (`1h30m`, `1.5h`), with units `ns`, `us`/`µs`, `ms`, `s`, `m`,
/// `h`; or a bare `0`. `humantime` is not used because it also accepts
/// `10min`, which this grammar rejects.
pub fn parse(s: &str) -> Result<Duration, DurationError> {
    const UNITS: [(&str, u128); 8] = [
        ("ns", 1),
        ("us", 1_000),
        ("µs", 1_000),
        ("μs", 1_000),
        ("ms", 1_000_000),
        ("s", 1_000_000_000),
        ("m", 60_000_000_000),
        ("h", 3_600_000_000_000),
    ];
    let (negative, mut rest) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    if rest == "0" {
        return Ok(Duration::ZERO);
    }
    if rest.is_empty() {
        return Err(DurationError::Malformed);
    }
    let mut total: u128 = 0;
    while !rest.is_empty() {
        let int_len = rest.bytes().take_while(u8::is_ascii_digit).count();
        let (int, after) = rest.split_at(int_len);
        let (frac, after) = match after.strip_prefix('.') {
            Some(after) => {
                let len = after.bytes().take_while(u8::is_ascii_digit).count();
                after.split_at(len)
            }
            None => ("", after),
        };
        if int.is_empty() && frac.is_empty() {
            return Err(DurationError::Malformed);
        }
        let unit_len = after
            .find(|c: char| c.is_ascii_digit() || c == '.')
            .unwrap_or(after.len());
        let (unit, after) = after.split_at(unit_len);
        let scale = UNITS
            .iter()
            .find(|(name, _)| *name == unit)
            .map(|(_, scale)| *scale)
            .ok_or(DurationError::Malformed)?;
        let whole: u128 = if int.is_empty() {
            0
        } else {
            int.parse().map_err(|_| DurationError::Malformed)?
        };
        let mut term = whole.checked_mul(scale).ok_or(DurationError::Malformed)?;
        // Digits past nanosecond precision change nothing.
        let mut place = scale;
        for digit in frac.bytes() {
            place /= 10;
            term += u128::from(digit - b'0') * place;
        }
        total = total.checked_add(term).ok_or(DurationError::Malformed)?;
        rest = after;
    }
    if negative && total > 0 {
        return Err(DurationError::Negative);
    }
    let secs = u64::try_from(total / 1_000_000_000).map_err(|_| DurationError::Malformed)?;
    Ok(Duration::new(secs, (total % 1_000_000_000) as u32))
}

/// A duration spelled at second scale and above: `45s`, `1m30s`, `1h0m0s`,
/// `1.5s`.
pub fn format(d: Duration) -> String {
    let secs = d.as_secs();
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    let mut out = String::new();
    if h > 0 {
        out.push_str(&format!("{h}h"));
    }
    if h > 0 || m > 0 {
        out.push_str(&format!("{m}m"));
    }
    out.push_str(&s.to_string());
    let nanos = d.subsec_nanos();
    if nanos > 0 {
        let frac = format!("{nanos:09}");
        out.push('.');
        out.push_str(frac.trim_end_matches('0'));
    }
    out.push('s');
    out
}

/// [`format`] after rounding to the nearest second, half up: `5s`, `2m5s`,
/// `1h0m0s`.
pub fn format_rounded(d: Duration) -> String {
    let secs = d.as_secs() + u64::from(d.subsec_millis() >= 500);
    format(Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_spells_durations() {
        for (d, want) in [
            (Duration::from_secs(45), "45s"),
            (Duration::from_secs(1), "1s"),
            (Duration::from_secs(90), "1m30s"),
            (Duration::from_secs(3600), "1h0m0s"),
            (Duration::from_millis(1500), "1.5s"),
            (Duration::from_millis(61_250), "1m1.25s"),
        ] {
            assert_eq!(format(d), want, "{d:?}");
        }
    }

    #[test]
    fn format_rounded_rounds_half_up() {
        for (d, want) in [
            (Duration::from_millis(1499), "1s"),
            (Duration::from_millis(1500), "2s"),
            (Duration::from_millis(59_500), "1m0s"),
            (Duration::from_millis(3_599_500), "1h0m0s"),
            (Duration::from_secs(125), "2m5s"),
        ] {
            assert_eq!(format_rounded(d), want, "{d:?}");
        }
    }

    #[test]
    fn parse_accepts_the_grammar() {
        let secs = Duration::from_secs;
        for (input, want) in [
            ("10m", secs(600)),
            ("90s", secs(90)),
            ("1h", secs(3600)),
            ("1h30m", secs(5400)),
            ("1.5h", secs(5400)),
            (".5m", secs(30)),
            ("1.m", secs(60)),
            ("+5m", secs(300)),
            ("-0", Duration::ZERO),
            ("0", Duration::ZERO),
            ("0s", Duration::ZERO),
            ("250ms", Duration::from_millis(250)),
            ("3us", Duration::from_micros(3)),
            ("3µs", Duration::from_micros(3)),
            ("7ns", Duration::from_nanos(7)),
        ] {
            assert_eq!(parse(input), Ok(want), "{input:?}");
        }
        for bad in [
            "", "5", "m", ".", "10min", "1d", "1 h", "h1", "1h-5m", "--5m", "later",
        ] {
            assert_eq!(parse(bad), Err(DurationError::Malformed), "{bad:?}");
        }
        assert_eq!(parse("-5m"), Err(DurationError::Negative));
    }
}
