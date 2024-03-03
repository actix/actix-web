use bytes::{Bytes, BytesMut};
pub use ppp::{
    v1::Addresses as V1Addresses,
    v2::{Addresses as V2Addresses, Command, Protocol, Type as TlvType},
};
use tracing::trace;

use crate::error::ParseError;

const V1_PREFIX_LEN: usize = 5;
const V1_MAX_LEN: usize = 107;
const V2_PREFIX_LEN: usize = 12;
const V2_MIN_LEN: usize = 16;
const V2_LEN_INDEX_1: usize = 14;
const V2_LEN_INDEX_2: usize = 15;

#[derive(Clone, Debug, PartialEq)]
pub enum ProxyProtocol {
    V1(ProxyProtocolV1),
    V2(ProxyProtocolV2),
}

impl ProxyProtocol {
    pub(crate) fn decode(src: &mut BytesMut) -> Result<Option<Self>, ParseError> {
        if src.len() >= V1_PREFIX_LEN
            && &src[..V1_PREFIX_LEN] == ppp::v1::PROTOCOL_PREFIX.as_bytes()
        {
            if let Some(line_end) = src.iter().position(|b| *b == b'\r') {
                if let Some(delimiter) = src.get(line_end + 1) {
                    if delimiter == &b'\n' {
                        let proxy_line = src.split_to(line_end + 2).freeze();

                        if proxy_line.len() > V1_MAX_LEN {
                            trace!("proxy protocol header too long");
                            return Err(ParseError::Header);
                        }

                        match ppp::v1::Header::try_from(&proxy_line[..]) {
                            Ok(header) => Ok(Some(
                                ProxyProtocolV1 {
                                    addresses: header.addresses,
                                }
                                .into(),
                            )),
                            Err(e) => {
                                trace!("error parsing proxy protocol v1 header: {:?}", e);
                                Err(ParseError::Header)
                            }
                        }
                    } else {
                        trace!("invalid line ending found");
                        Err(ParseError::Header)
                    }
                } else {
                    trace!("no line ending found, might be a partial request");
                    Ok(None)
                }
            } else if src.len() > V1_MAX_LEN {
                trace!("proxy protocol header too long");
                Err(ParseError::Header)
            } else {
                trace!("no line ending found, might be a partial request");
                Ok(None)
            }
        } else if src.len() >= V2_PREFIX_LEN && &src[..V2_PREFIX_LEN] == ppp::v2::PROTOCOL_PREFIX {
            if src.len() < V2_MIN_LEN {
                return Ok(None);
            }

            let total_length = V2_MIN_LEN
                + u16::from_be_bytes([src[V2_LEN_INDEX_1], src[V2_LEN_INDEX_2]]) as usize;

            if src.len() < total_length {
                return Ok(None);
            }

            let proxy_line = src.split_to(total_length).freeze();

            match ppp::v2::Header::try_from(&proxy_line[..]) {
                Ok(header) => {
                    let size_hint = header.tlvs().size_hint();
                    let mut converted_tlvs: Vec<Tlv> =
                        Vec::with_capacity(size_hint.1.unwrap_or(size_hint.0));

                    for tlv in header.tlvs() {
                        match tlv {
                            Ok(tlv) => {
                                converted_tlvs.push(tlv.into());
                            }
                            Err(e) => {
                                trace!(
                                    "error parsing proxy protocol v2 type length value: {:?}",
                                    e
                                );
                                return Err(ParseError::Header);
                            }
                        }
                    }

                    Ok(Some(
                        ProxyProtocolV2 {
                            addresses: header.addresses,
                            command: header.command,
                            protocol: header.protocol,
                            tlvs: converted_tlvs,
                        }
                        .into(),
                    ))
                }
                Err(e) => {
                    trace!("error parsing proxy protocol v1 header: {:?}", e);
                    return Err(ParseError::Header);
                }
            }
        } else if src.len() < V1_PREFIX_LEN || src.len() < V2_PREFIX_LEN {
            trace!("not enough data to parse proxy protocol header");
            Ok(None)
        } else {
            trace!("invalid proxy protocol header");
            Err(ParseError::Header)
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProxyProtocolV1 {
    pub addresses: V1Addresses,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProxyProtocolV2 {
    pub addresses: V2Addresses,
    pub command: Command,
    pub protocol: Protocol,
    pub tlvs: Vec<Tlv>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tlv {
    pub kind: TlvType,
    pub value: bytes::Bytes,
}

impl From<ProxyProtocolV1> for ProxyProtocol {
    fn from(v1: ProxyProtocolV1) -> Self {
        ProxyProtocol::V1(v1)
    }
}

impl From<ProxyProtocolV2> for ProxyProtocol {
    fn from(v2: ProxyProtocolV2) -> Self {
        ProxyProtocol::V2(v2)
    }
}

impl From<ppp::v2::TypeLengthValue<'_>> for Tlv {
    fn from(tlv: ppp::v2::TypeLengthValue<'_>) -> Self {
        Tlv {
            kind: match tlv.kind {
                0x01 => TlvType::ALPN,
                0x02 => TlvType::Authority,
                0x03 => TlvType::CRC32C,
                0x04 => TlvType::NoOp,
                0x05 => TlvType::UniqueId,
                0x20 => TlvType::SSL,
                0x21 => TlvType::SSLVersion,
                0x22 => TlvType::SSLCommonName,
                0x23 => TlvType::SSLCipher,
                0x24 => TlvType::SSLSignatureAlgorithm,
                0x25 => TlvType::SSLKeyAlgorithm,
                0x30 => TlvType::NetworkNamespace,
                _ => unreachable!(),
            },
            value: Bytes::copy_from_slice(&tlv.value),
        }
    }
}
