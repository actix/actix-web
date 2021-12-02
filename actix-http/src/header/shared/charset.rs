use std::{fmt, str};

use self::Charset::*;

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
    /// An arbitrary charset specified as a string
    Ext(String),
}

impl Charset {
    fn label(&self) -> &str {
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
            Ext(ref s) => s,
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

    fn from_str(s: &str) -> Result<Charset, crate::Error> {
        Ok(match s.to_ascii_uppercase().as_ref() {
            "US-ASCII" => Us_Ascii,
            "ISO-8859-1" => Iso_8859_1,
            "ISO-8859-2" => Iso_8859_2,
            "ISO-8859-3" => Iso_8859_3,
            "ISO-8859-4" => Iso_8859_4,
            "ISO-8859-5" => Iso_8859_5,
            "ISO-8859-6" => Iso_8859_6,
            "ISO-8859-7" => Iso_8859_7,
            "ISO-8859-8" => Iso_8859_8,
            "ISO-8859-9" => Iso_8859_9,
            "ISO-8859-10" => Iso_8859_10,
            "SHIFT-JIS" => Shift_Jis,
            "EUC-JP" => Euc_Jp,
            "ISO-2022-KR" => Iso_2022_Kr,
            "EUC-KR" => Euc_Kr,
            "ISO-2022-JP" => Iso_2022_Jp,
            "ISO-2022-JP-2" => Iso_2022_Jp_2,
            "ISO-8859-6-E" => Iso_8859_6_E,
            "ISO-8859-6-I" => Iso_8859_6_I,
            "ISO-8859-8-E" => Iso_8859_8_E,
            "ISO-8859-8-I" => Iso_8859_8_I,
            "GB2312" => Gb2312,
            "BIG5" => Big5,
            "KOI8-R" => Koi8_R,
            s => Ext(s.to_owned()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse() {
        assert_eq!(Us_Ascii, "us-ascii".parse().unwrap());
        assert_eq!(Us_Ascii, "US-Ascii".parse().unwrap());
        assert_eq!(Us_Ascii, "US-ASCII".parse().unwrap());
        assert_eq!(Shift_Jis, "Shift-JIS".parse().unwrap());
        assert_eq!(Ext("ABCD".to_owned()), "abcd".parse().unwrap());
    }

    #[test]
    fn test_display() {
        assert_eq!("US-ASCII", format!("{}", Us_Ascii));
        assert_eq!("ABCD", format!("{}", Ext("ABCD".to_owned())));
    }
}
