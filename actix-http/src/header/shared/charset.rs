use std::{fmt, str};

/// A MIME character set.
///
/// The string representation is normalized to upper case.
///
/// See <http://www.iana.org/assignments/character-sets/character-sets.xhtml>.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum Charset {
    /// US ASCII
    Us_Ascii,

    /// ISO-8859-1
    Iso_8859_1,

    /// ISO-8859-2
    Iso_8859_2,

    /// ISO-8859-3
    Iso_8859_3,

    /// ISO-8859-4
    Iso_8859_4,

    /// ISO-8859-5
    Iso_8859_5,

    /// ISO-8859-6
    Iso_8859_6,

    /// ISO-8859-7
    Iso_8859_7,

    /// ISO-8859-8
    Iso_8859_8,

    /// ISO-8859-9
    Iso_8859_9,

    /// ISO-8859-10
    Iso_8859_10,

    /// Shift_JIS
    Shift_Jis,

    /// EUC-JP
    Euc_Jp,

    /// ISO-2022-KR
    Iso_2022_Kr,

    /// EUC-KR
    Euc_Kr,

    /// ISO-2022-JP
    Iso_2022_Jp,

    /// ISO-2022-JP-2
    Iso_2022_Jp_2,

    /// ISO-8859-6-E
    Iso_8859_6_E,

    /// ISO-8859-6-I
    Iso_8859_6_I,

    /// ISO-8859-8-E
    Iso_8859_8_E,

    /// ISO-8859-8-I
    Iso_8859_8_I,

    /// GB2312
    Gb2312,

    /// Big5
    Big5,

    /// KOI8-R
    Koi8_R,

    /// An arbitrary charset specified as a string.
    Ext(String),
}

impl Charset {
    fn label(&self) -> &str {
        use Charset::*;

        match *self {
            Us_Ascii => "US-ASCII",
            Iso_8859_1 => "ISO-8859-1",
            Iso_8859_2 => "ISO-8859-2",
            Iso_8859_3 => "ISO-8859-3",
            Iso_8859_4 => "ISO-8859-4",
            Iso_8859_5 => "ISO-8859-5",
            Iso_8859_6 => "ISO-8859-6",
            Iso_8859_7 => "ISO-8859-7",
            Iso_8859_8 => "ISO-8859-8",
            Iso_8859_9 => "ISO-8859-9",
            Iso_8859_10 => "ISO-8859-10",
            Shift_Jis => "Shift-JIS",
            Euc_Jp => "EUC-JP",
            Iso_2022_Kr => "ISO-2022-KR",
            Euc_Kr => "EUC-KR",
            Iso_2022_Jp => "ISO-2022-JP",
            Iso_2022_Jp_2 => "ISO-2022-JP-2",
            Iso_8859_6_E => "ISO-8859-6-E",
            Iso_8859_6_I => "ISO-8859-6-I",
            Iso_8859_8_E => "ISO-8859-8-E",
            Iso_8859_8_I => "ISO-8859-8-I",
            Gb2312 => "GB2312",
            Big5 => "Big5",
            Koi8_R => "KOI8-R",
            Ext(ref cs) => cs,
        }
    }
}

impl fmt::Display for Charset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

impl str::FromStr for Charset {
    type Err = crate::Error;

    #[inline]
    fn from_str(cs: &str) -> Result<Charset, crate::Error> {
        use Charset::*;

        Ok(match () {
            _ if cs.eq_ignore_ascii_case("US-ASCII") => Us_Ascii,
            _ if cs.eq_ignore_ascii_case("ISO-8859-1") => Iso_8859_1,
            _ if cs.eq_ignore_ascii_case("ISO-8859-2") => Iso_8859_2,
            _ if cs.eq_ignore_ascii_case("ISO-8859-3") => Iso_8859_3,
            _ if cs.eq_ignore_ascii_case("ISO-8859-4") => Iso_8859_4,
            _ if cs.eq_ignore_ascii_case("ISO-8859-5") => Iso_8859_5,
            _ if cs.eq_ignore_ascii_case("ISO-8859-6") => Iso_8859_6,
            _ if cs.eq_ignore_ascii_case("ISO-8859-7") => Iso_8859_7,
            _ if cs.eq_ignore_ascii_case("ISO-8859-8") => Iso_8859_8,
            _ if cs.eq_ignore_ascii_case("ISO-8859-9") => Iso_8859_9,
            _ if cs.eq_ignore_ascii_case("ISO-8859-10") => Iso_8859_10,
            _ if cs.eq_ignore_ascii_case("SHIFT-JIS") => Shift_Jis,
            _ if cs.eq_ignore_ascii_case("EUC-JP") => Euc_Jp,
            _ if cs.eq_ignore_ascii_case("ISO-2022-KR") => Iso_2022_Kr,
            _ if cs.eq_ignore_ascii_case("EUC-KR") => Euc_Kr,
            _ if cs.eq_ignore_ascii_case("ISO-2022-JP") => Iso_2022_Jp,
            _ if cs.eq_ignore_ascii_case("ISO-2022-JP-2") => Iso_2022_Jp_2,
            _ if cs.eq_ignore_ascii_case("ISO-8859-6-E") => Iso_8859_6_E,
            _ if cs.eq_ignore_ascii_case("ISO-8859-6-I") => Iso_8859_6_I,
            _ if cs.eq_ignore_ascii_case("ISO-8859-8-E") => Iso_8859_8_E,
            _ if cs.eq_ignore_ascii_case("ISO-8859-8-I") => Iso_8859_8_I,
            _ if cs.eq_ignore_ascii_case("GB2312") => Gb2312,
            _ if cs.eq_ignore_ascii_case("BIG5") => Big5,
            _ if cs.eq_ignore_ascii_case("KOI8-R") => Koi8_R,
            _ => Ext(cs.to_ascii_uppercase()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Charset::*;

    #[test]
    fn test_parse() {
        assert_eq!(Us_Ascii, "us-ascii".parse().unwrap());
        assert_eq!(Us_Ascii, "US-Ascii".parse().unwrap());
        assert_eq!(Us_Ascii, "US-ASCII".parse().unwrap());
        assert_eq!(Shift_Jis, "Shift-JIS".parse().unwrap());
        assert_eq!(Ext("ABCD".to_owned()), "abcd".parse().unwrap());
        assert_eq!(Ext("UTF-8".to_owned()), "utf-8".parse().unwrap());
        assert_eq!(Ext(String::new()), "".parse().unwrap());
        assert_eq!(Ext("éBC".to_owned()), "ébc".parse().unwrap());
    }

    #[test]
    fn test_display() {
        assert_eq!("US-ASCII", Us_Ascii.to_string());
        assert_eq!("ABCD", Ext("ABCD".to_owned()).to_string());
    }
}
