use std::{
    io::{Read, Write},
    net::{Ipv4Addr, Shutdown, TcpStream},
};

use actix_http::proxy_protocol::{
    Command, Protocol, ProxyProtocol, ProxyProtocolV1, ProxyProtocolV2, TlvType, V1Addresses,
    V2Addresses,
};
use actix_test::TestServerConfig;
use actix_web::{get, App, HttpMessage, HttpRequest, HttpResponse, Responder};

#[get("/v1")]
async fn proxy_protocol_v1(req: HttpRequest) -> impl Responder {
    let extensions = req.extensions();
    let proxy_protocol = extensions.get::<ProxyProtocol>().unwrap();

    if let ProxyProtocol::V1(ProxyProtocolV1 {
        addresses: V1Addresses::Tcp4(addr),
    }) = proxy_protocol
    {
        if addr.source_address == Ipv4Addr::new(127, 0, 1, 2)
            && addr.destination_address == Ipv4Addr::new(192, 168, 1, 101)
            && addr.source_port == 80
            && addr.destination_port == 443
        {
            return HttpResponse::Ok().body(format!("{:?}", proxy_protocol));
        }
    }

    HttpResponse::NotFound().finish()
}

#[get("/v2")]
async fn proxy_protocol_v2(req: HttpRequest) -> impl Responder {
    let extensions = req.extensions();
    let proxy_protocol = extensions.get::<ProxyProtocol>().unwrap();

    if let ProxyProtocol::V2(ProxyProtocolV2 {
        addresses: V2Addresses::IPv4(addr),
        command,
        protocol,
        tlvs,
    }) = proxy_protocol
    {
        if addr.source_address == Ipv4Addr::new(127, 0, 1, 2)
            && addr.destination_address == Ipv4Addr::new(192, 168, 1, 101)
            && addr.source_port == 80
            && addr.destination_port == 443
            && matches!(command, Command::Proxy)
            && matches!(protocol, Protocol::Datagram)
            && tlvs.len() == 1
            && tlvs[0].kind == TlvType::NoOp
            && tlvs[0].value[..] == [42]
        {
            return HttpResponse::Ok().body(format!("{:?}", proxy_protocol));
        }
    }

    HttpResponse::NotFound().finish()
}

#[actix_rt::test]
async fn test_parse_proxy_protocol_v1() {
    let srv = actix_test::start_with(TestServerConfig::default().h1().proxy_protocol(), || {
        App::new().service(proxy_protocol_v1)
    });

    let mut stream = TcpStream::connect(srv.addr()).unwrap();
    stream
        .write_all(
            b"PROXY TCP4 127.0.1.2 192.168.1.101 80 443\r\nGET /v1 HTTP/1.1\r\n\r\n".as_ref(),
        )
        .unwrap();

    let mut buf = [0; 1024];
    let n = stream.read(&mut buf).unwrap();

    let response = String::from_utf8_lossy(&buf[..n]);
    println!("{}", response);
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));

    stream.shutdown(Shutdown::Both).unwrap();

    srv.stop().await;
}

#[actix_rt::test]
async fn test_parse_proxy_protocol_v2() {
    let srv = actix_test::start_with(TestServerConfig::default().h1().proxy_protocol(), || {
        App::new().service(proxy_protocol_v2)
    });

    let mut stream = TcpStream::connect(srv.addr()).unwrap();
    stream.write_all(b"\r\n\r\n\0\r\nQUIT\n".as_ref()).unwrap();
    stream
        .write_all(&[
            0x21, 0x12, 0, 16, 127, 0, 1, 2, 192, 168, 1, 101, 0, 80, 1, 187, 4, 0, 1, 42,
        ])
        .unwrap();
    stream.write_all(b"GET /v2 HTTP/1.1\r\n\r\n").unwrap();

    let mut buf = [0; 1024];
    let n = stream.read(&mut buf).unwrap();

    let response = String::from_utf8_lossy(&buf[..n]);
    println!("{}", response);
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));

    stream.shutdown(Shutdown::Both).unwrap();

    srv.stop().await;
}
