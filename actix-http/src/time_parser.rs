use time::{Date, OffsetDateTime, PrimitiveDateTime};

/// Attempt to parse a `time` string as one of either RFC 1123, RFC 850, or asctime.
pub fn parse_http_date(time: &str) -> Option<PrimitiveDateTime> {
    try_parse_rfc_1123(time)
        .or_else(|| try_parse_rfc_850(time))
        .or_else(|| try_parse_asctime(time))
}

/// Attempt to parse a `time` string as a RFC 1123 formatted date time string.
fn try_parse_rfc_1123(time: &str) -> Option<PrimitiveDateTime> {
    time::parse(time, "%a, %d %b %Y %H:%M:%S").ok()
}

/// Attempt to parse a `time` string as a RFC 850 formatted date time string.
fn try_parse_rfc_850(time: &str) -> Option<PrimitiveDateTime> {
    match PrimitiveDateTime::parse(time, "%A, %d-%b-%y %H:%M:%S") {
        Ok(dt) => {
            // If the `time` string contains a two-digit year, then as per RFC 2616 § 19.3,
            // we consider the year as part of this century if it's within the next 50 years,
            // otherwise we consider as part of the previous century.
            let now = OffsetDateTime::now_utc();
            let century_start_year = (now.year() / 100) * 100;
            let mut expanded_year = century_start_year + dt.year();

            if expanded_year > now.year() + 50 {
                expanded_year -= 100;
            }

            match Date::try_from_ymd(expanded_year, dt.month(), dt.day()) {
                Ok(date) => Some(PrimitiveDateTime::new(date, dt.time())),
                Err(_) => None,
            }
        }
        Err(_) => None,
    }
}

/// Attempt to parse a `time` string using ANSI C's `asctime` format.
fn try_parse_asctime(time: &str) -> Option<PrimitiveDateTime> {
    time::parse(time, "%a %b %_d %H:%M:%S %Y").ok()
}
