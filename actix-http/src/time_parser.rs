use time::{Date, OffsetDateTime, PrimitiveDateTime};

/// Attempt to parse a `time` string as one of either RFC 1123, RFC 850, or asctime.
pub(crate) fn parse_http_date(time: &str) -> Option<PrimitiveDateTime> {
    try_parse_rfc_1123(time)
        .or_else(|| try_parse_rfc_850(time))
        .or_else(|| try_parse_asctime(time))
}

/// Attempt to parse a `time` string as a RFC 1123 formatted date time string.
///
/// Eg: `Fri, 12 Feb 2021 00:14:29 GMT`
fn try_parse_rfc_1123(time: &str) -> Option<PrimitiveDateTime> {
    time::parse(time, "%a, %d %b %Y %H:%M:%S").ok()
}

/// Attempt to parse a `time` string as a RFC 850 formatted date time string.
///
/// Eg: `Wednesday, 11-Jan-21 13:37:41 UTC`
fn try_parse_rfc_850(time: &str) -> Option<PrimitiveDateTime> {
    let dt = PrimitiveDateTime::parse(time, "%A, %d-%b-%y %H:%M:%S").ok()?;

    // If the `time` string contains a two-digit year, then as per RFC 2616 § 19.3,
    // we consider the year as part of this century if it's within the next 50 years,
    // otherwise we consider as part of the previous century.

    let now = OffsetDateTime::now_utc();
    let century_start_year = (now.year() / 100) * 100;
    let mut expanded_year = century_start_year + dt.year();

    if expanded_year > now.year() + 50 {
        expanded_year -= 100;
    }

    let date = Date::try_from_ymd(expanded_year, dt.month(), dt.day()).ok()?;
    Some(PrimitiveDateTime::new(date, dt.time()))
}

/// Attempt to parse a `time` string using ANSI C's `asctime` format.
///
/// Eg: `Wed Feb 13 15:46:11 2013`
fn try_parse_asctime(time: &str) -> Option<PrimitiveDateTime> {
    time::parse(time, "%a %b %_d %H:%M:%S %Y").ok()
}

#[cfg(test)]
mod tests {
    use time::{date, time};

    use super::*;

    #[test]
    fn test_rfc_850_year_shift() {
        let date = try_parse_rfc_850("Friday, 19-Nov-82 16:14:55 EST").unwrap();
        assert_eq!(date, date!(1982 - 11 - 19).with_time(time!(16:14:55)));

        let date = try_parse_rfc_850("Wednesday, 11-Jan-62 13:37:41 EST").unwrap();
        assert_eq!(date, date!(2062 - 01 - 11).with_time(time!(13:37:41)));

        let date = try_parse_rfc_850("Wednesday, 11-Jan-21 13:37:41 EST").unwrap();
        assert_eq!(date, date!(2021 - 01 - 11).with_time(time!(13:37:41)));

        let date = try_parse_rfc_850("Wednesday, 11-Jan-23 13:37:41 EST").unwrap();
        assert_eq!(date, date!(2023 - 01 - 11).with_time(time!(13:37:41)));

        let date = try_parse_rfc_850("Wednesday, 11-Jan-99 13:37:41 EST").unwrap();
        assert_eq!(date, date!(1999 - 01 - 11).with_time(time!(13:37:41)));

        let date = try_parse_rfc_850("Wednesday, 11-Jan-00 13:37:41 EST").unwrap();
        assert_eq!(date, date!(2000 - 01 - 11).with_time(time!(13:37:41)));
    }
}
