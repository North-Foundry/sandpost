use std::{io, time::Duration};

/// The number of iterations represented by one burst.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Quantity {
    /// Run a fixed number of iterations.
    Fixed(u64),
    /// Choose an inclusive random iteration count between these bounds.
    Random { minimum: u64, maximum: u64 },
}

/// One iteration count and the delay before the next burst.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Burst {
    /// Fixed or randomly selected iteration count.
    pub quantity: Quantity,
    /// Delay after this burst; the executor may ignore it after the final burst.
    pub delay: Duration,
}

/// Parse a positive integer count with an optional decimal `k` or `K` multiplier.
pub fn parse_count(value: &str) -> crate::Result<u64> {
    let value = value.trim();
    let (digits, multiplier) = match value.strip_suffix(['k', 'K']) {
        Some(digits) => (digits, 1_000),
        None => (value, 1),
    };
    let count = parse_digits(digits.trim(), "count")?
        .checked_mul(multiplier)
        .ok_or_else(|| invalid("count overflows u64"))?;
    if count == 0 {
        return Err(invalid("count must be positive"));
    }
    Ok(count)
}

/// Parse a nonnegative integer duration with an `ms`, `s`, or `m` unit.
pub fn parse_duration(value: &str) -> crate::Result<Duration> {
    let value = value.trim();
    let (digits, multiplier) = if let Some(digits) = value.strip_suffix("ms") {
        (digits, 1_u64)
    } else if let Some(digits) = value.strip_suffix('s') {
        (digits, 1_000)
    } else if let Some(digits) = value.strip_suffix('m') {
        (digits, 60_000)
    } else {
        return Err(invalid("duration requires ms, s, or m units"));
    };
    let milliseconds = parse_digits(digits, "duration")?
        .checked_mul(multiplier)
        .ok_or_else(|| invalid("duration overflows u64 milliseconds"))?;
    Ok(Duration::from_millis(milliseconds))
}

/// Parse comma-separated fixed or random counts with optional following delays.
pub fn parse_pattern(value: &str, interval: Duration) -> crate::Result<Vec<Burst>> {
    let tokens = split_tokens(value)?;
    let mut bursts = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let quantity = parse_quantity(tokens[index])?;
        let mut delay = interval;
        index += 1;
        if index < tokens.len() && is_duration_token(tokens[index]) {
            delay = parse_duration(tokens[index])?;
            index += 1;
        }
        bursts.push(Burst { quantity, delay });
    }
    Ok(bursts)
}

/// Parse an unsigned decimal integer without accepting signs or whitespace within it.
fn parse_digits(value: &str, kind: &str) -> crate::Result<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid(format!("invalid {kind}")));
    }
    value
        .parse()
        .map_err(|_| invalid(format!("{kind} overflows u64")))
}

/// Create an invalid-input error suitable for the crate's boxed error result.
fn invalid(message: impl Into<String>) -> Box<dyn std::error::Error> {
    io::Error::new(io::ErrorKind::InvalidInput, message.into()).into()
}

/// Split commas outside random-count parentheses and validate their balance.
fn split_tokens(value: &str) -> crate::Result<Vec<&str>> {
    let mut tokens = Vec::new();
    let mut start = 0;
    let mut depth = 0;
    for (index, character) in value.char_indices() {
        match character {
            '(' if depth == 0 => depth = 1,
            '(' => return Err(invalid("nested parentheses are not allowed")),
            ')' if depth == 1 => depth = 0,
            ')' => return Err(invalid("unbalanced parentheses")),
            ',' if depth == 0 => {
                tokens.push(value[start..index].trim());
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err(invalid("unbalanced parentheses"));
    }
    tokens.push(value[start..].trim());
    if tokens.iter().any(|token| token.is_empty()) {
        return Err(invalid("pattern contains an empty token"));
    }
    Ok(tokens)
}

/// Parse a fixed count or an inclusive `rand(minimum, maximum)` count.
fn parse_quantity(value: &str) -> crate::Result<Quantity> {
    let value = value.trim();
    if let Some(arguments) = value
        .strip_prefix("rand(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let (minimum, maximum) = arguments
            .split_once(',')
            .ok_or_else(|| invalid("rand requires exactly two bounds"))?;
        if maximum.contains(',') {
            return Err(invalid("rand requires exactly two bounds"));
        }
        let minimum = parse_count(minimum)?;
        let maximum = parse_count(maximum)?;
        if minimum > maximum {
            return Err(invalid("rand minimum exceeds maximum"));
        }
        Ok(Quantity::Random { minimum, maximum })
    } else {
        Ok(Quantity::Fixed(parse_count(value)?))
    }
}

/// Return whether a token has the shape of a supported duration.
fn is_duration_token(value: &str) -> bool {
    let value = value.trim();
    value.ends_with("ms") || value.ends_with('s') || value.ends_with('m')
}

#[cfg(test)]
mod tests {
    use super::{Burst, Quantity, parse_count, parse_duration, parse_pattern};
    use std::time::Duration;

    /// Accept fixed counts, suffixes, and the documented sequence delays.
    #[test]
    fn parses_fixed_patterns_and_delays() {
        let interval = Duration::from_millis(25);
        assert_eq!(parse_count("2k").unwrap(), 2_000);
        assert_eq!(
            parse_pattern("50,10,1,50", interval).unwrap(),
            vec![
                Burst {
                    quantity: Quantity::Fixed(50),
                    delay: interval
                },
                Burst {
                    quantity: Quantity::Fixed(10),
                    delay: interval
                },
                Burst {
                    quantity: Quantity::Fixed(1),
                    delay: interval
                },
                Burst {
                    quantity: Quantity::Fixed(50),
                    delay: interval
                },
            ]
        );
        assert_eq!(
            parse_pattern("50,10s,10,1s,rand(10,100)", interval).unwrap(),
            vec![
                Burst {
                    quantity: Quantity::Fixed(50),
                    delay: Duration::from_secs(10)
                },
                Burst {
                    quantity: Quantity::Fixed(10),
                    delay: Duration::from_secs(1)
                },
                Burst {
                    quantity: Quantity::Random {
                        minimum: 10,
                        maximum: 100
                    },
                    delay: interval
                },
            ]
        );
    }

    /// Accept whitespace, all duration units, zero durations, and random bounds.
    #[test]
    fn parses_random_counts_and_duration_units() {
        assert_eq!(parse_count(" 1 K ").unwrap(), 1_000);
        assert_eq!(parse_duration("0ms").unwrap(), Duration::ZERO);
        assert_eq!(parse_duration("2s").unwrap(), Duration::from_secs(2));
        assert_eq!(parse_duration("3m").unwrap(), Duration::from_secs(180));
        assert_eq!(
            parse_pattern(" rand( 1k , 2K ) ", Duration::ZERO).unwrap()[0].quantity,
            Quantity::Random {
                minimum: 1_000,
                maximum: 2_000
            }
        );
    }

    /// Reject malformed quantities, delimiters, random bounds, and overflows.
    #[test]
    fn rejects_malformed_patterns_and_overflows() {
        for value in [
            "",
            ",1",
            "1,",
            "1,,2",
            "1s,2",
            "1s,2s,3",
            "1.5",
            "0",
            "rand(1)",
            "rand(1,2,3)",
            "rand(2,1)",
            "rand(0,1)",
            "rand((1),2)",
            "rand(1,2",
            "rand(1,2))",
            "18446744073709551616",
            "18446744073709552k",
        ] {
            assert!(parse_pattern(value, Duration::ZERO).is_err(), "{value}");
        }
        assert!(parse_duration("18446744073709551615m").is_err());
        assert!(parse_duration("1.5s").is_err());
        assert!(parse_duration("10").is_err());
    }
}
