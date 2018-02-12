#![allow(dead_code)]
use std::io;
use serde_json as json;
use byteorder::{BigEndian , ByteOrder};
use bytes::{BytesMut, BufMut};
use tokio_io::codec::{Encoder, Decoder};

/// Client request
#[derive(Serialize, Deserialize, Debug, Message)]
#[serde(tag="cmd", content="data")]
pub enum ChatRequest {
    /// List rooms
    List,
    /// Join rooms
    Join(String),
    /// Send message
    Message(String),
    /// Ping
    Ping
}

/// Server response
#[derive(Serialize, Deserialize, Debug, Message)]
#[serde(tag="cmd", content="data")]
pub enum ChatResponse {
    Ping,

    /// List of rooms
    Rooms(Vec<String>),

    /// Joined
    Joined(String),

    /// Message
    Message(String),
}

/// Codec for Client -> Server transport
pub struct ChatCodec;

impl Decoder for ChatCodec
{
    type Item = ChatRequest;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let size = {
            if src.len() < 2 {
                return Ok(None)
            }
            BigEndian::read_u16(src.as_ref()) as usize
        };

        if src.len() >= size + 2 {
            src.split_to(2);
            let buf = src.split_to(size);
            Ok(Some(json::from_slice::<ChatRequest>(&buf)?))
        } else {
            Ok(None)
        }
    }
}

impl Encoder for ChatCodec
{
    type Item = ChatResponse;
    type Error = io::Error;

    fn encode(&mut self, msg: ChatResponse, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let msg = json::to_string(&msg).unwrap();
        let msg_ref: &[u8] = msg.as_ref();

        dst.reserve(msg_ref.len() + 2);
        dst.put_u16::<BigEndian>(msg_ref.len() as u16);
        dst.put(msg_ref);

        Ok(())
    }
}


/// Codec for Server -> Client transport
pub struct ClientChatCodec;

impl Decoder for ClientChatCodec
{
    type Item = ChatResponse;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let size = {
            if src.len() < 2 {
                return Ok(None)
            }
            BigEndian::read_u16(src.as_ref()) as usize
        };

        if src.len() >= size + 2 {
            src.split_to(2);
            let buf = src.split_to(size);
            Ok(Some(json::from_slice::<ChatResponse>(&buf)?))
        } else {
            Ok(None)
        }
    }
}

impl Encoder for ClientChatCodec
{
    type Item = ChatRequest;
    type Error = io::Error;

    fn encode(&mut self, msg: ChatRequest, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let msg = json::to_string(&msg).unwrap();
        let msg_ref: &[u8] = msg.as_ref();

        dst.reserve(msg_ref.len() + 2);
        dst.put_u16::<BigEndian>(msg_ref.len() as u16);
        dst.put(msg_ref);

        Ok(())
    }
}
