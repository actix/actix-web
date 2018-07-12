//! `x-www-form-urlencoded` meets Serde

#![warn(unused_extern_crates)]

extern crate itoa;
extern crate dtoa;
#[macro_use]
extern crate serde;
extern crate url;

pub mod de;
pub mod ser;

#[doc(inline)]
pub use de::{Deserializer, from_bytes, from_reader, from_str};
#[doc(inline)]
pub use ser::{Serializer, to_string};
