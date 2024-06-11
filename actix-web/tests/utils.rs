// compiling some tests will trigger unused function warnings even though other tests use them
#![allow(dead_code)]

use std::io::{Read as _, Write as _};

pub mod gzip {
    use flate2::{read::GzDecoder, write::GzEncoder, Compression};

    use super::*;

    pub fn encode(bytes: impl AsRef<[u8]>) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(bytes.as_ref()).unwrap();
        encoder.finish().unwrap()
    }

    pub fn decode(bytes: impl AsRef<[u8]>) -> Vec<u8> {
        let mut decoder = GzDecoder::new(bytes.as_ref());
        let mut buf = Vec::new();
        decoder.read_to_end(&mut buf).unwrap();
        buf
    }
}

pub mod deflate {
    use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression};

    use super::*;

    pub fn encode(bytes: impl AsRef<[u8]>) -> Vec<u8> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(bytes.as_ref()).unwrap();
        encoder.finish().unwrap()
    }

    pub fn decode(bytes: impl AsRef<[u8]>) -> Vec<u8> {
        let mut decoder = ZlibDecoder::new(bytes.as_ref());
        let mut buf = Vec::new();
        decoder.read_to_end(&mut buf).unwrap();
        buf
    }
}

pub mod brotli {
    use ::brotli::{reader::Decompressor as BrotliDecoder, CompressorWriter as BrotliEncoder};

    use super::*;

    pub fn encode(bytes: impl AsRef<[u8]>) -> Vec<u8> {
        let mut encoder = BrotliEncoder::new(
            Vec::new(),
            8 * 1024, // 32 KiB buffer
            3,        // BROTLI_PARAM_QUALITY
            22,       // BROTLI_PARAM_LGWIN
        );
        encoder.write_all(bytes.as_ref()).unwrap();
        encoder.flush().unwrap();
        encoder.into_inner()
    }

    pub fn decode(bytes: impl AsRef<[u8]>) -> Vec<u8> {
        let mut decoder = BrotliDecoder::new(bytes.as_ref(), 8_096);
        let mut buf = Vec::new();
        decoder.read_to_end(&mut buf).unwrap();
        buf
    }
}

pub mod zstd {
    use ::zstd::stream::{read::Decoder, write::Encoder};

    use super::*;

    pub fn encode(bytes: impl AsRef<[u8]>) -> Vec<u8> {
        let mut encoder = Encoder::new(Vec::new(), 3).unwrap();
        encoder.write_all(bytes.as_ref()).unwrap();
        encoder.finish().unwrap()
    }

    pub fn decode(bytes: impl AsRef<[u8]>) -> Vec<u8> {
        let mut decoder = Decoder::new(bytes.as_ref()).unwrap();
        let mut buf = Vec::new();
        decoder.read_to_end(&mut buf).unwrap();
        buf
    }
}
