use std::{future::Future, str, task::Poll};

use actix_service::fn_service;
use actix_utils::future::{ready, Ready};
use bytes::Bytes;
use futures_util::future::lazy;

use actix_codec::Framed;
use actix_service::Service;
use bytes::{Buf, BytesMut};

use super::dispatcher::{Dispatcher, DispatcherState, DispatcherStateProj, Flags};
use crate::{
    body::MessageBody,
    config::ServiceConfig,
    h1::{Codec, ExpectHandler, UpgradeHandler},
    service::HttpFlow,
    test::{TestBuffer, TestSeqBuffer},
    Error, HttpMessage, KeepAlive, Method, OnConnectData, Request, Response,
};

fn find_slice(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    memchr::memmem::find(&haystack[from..], needle)
}

fn stabilize_date_header(payload: &mut [u8]) {
    let mut from = 0;
    while let Some(pos) = find_slice(payload, b"date", from) {
        payload[(from + pos)..(from + pos + 35)]
            .copy_from_slice(b"date: Thu, 01 Jan 1970 12:34:56 UTC");
        from += 35;
    }
}

fn ok_service() -> impl Service<Request, Response = Response<impl MessageBody>, Error = Error> {
    fn_service(|_req: Request| ready(Ok::<_, Error>(Response::ok())))
}

fn echo_path_service(
) -> impl Service<Request, Response = Response<impl MessageBody>, Error = Error> {
    fn_service(|req: Request| {
        let path = req.path().as_bytes();
        ready(Ok::<_, Error>(
            Response::ok().set_body(Bytes::copy_from_slice(path)),
        ))
    })
}

fn echo_payload_service() -> impl Service<Request, Response = Response<Bytes>, Error = Error> {
    fn_service(|mut req: Request| {
        Box::pin(async move {
            use futures_util::stream::StreamExt as _;

            let mut pl = req.take_payload();
            let mut body = BytesMut::new();
            while let Some(chunk) = pl.next().await {
                body.extend_from_slice(chunk.unwrap().chunk())
            }

            Ok::<_, Error>(Response::ok().set_body(body.freeze()))
        })
    })
}

#[actix_rt::test]
#[ignore]
async fn test_keep_alive() {
    lazy(|cx| {
        let buf = TestBuffer::new("GET /abcd HTTP/1.1\r\n\r\n");

        let cfg = ServiceConfig::new(KeepAlive::Timeout(1), 100, 0, false, None);
        let services = HttpFlow::new(echo_path_service(), ExpectHandler, None);

        let h1 = Dispatcher::<_, _, _, _, UpgradeHandler>::new(
            buf.clone(),
            services,
            cfg,
            None,
            OnConnectData::default(),
        );
        actix_rt::pin!(h1);

        assert!(matches!(&h1.inner, DispatcherState::Normal { .. }));

        assert!(
            h1.as_mut().poll(cx).is_pending(),
            "keep-alive should prevent poll from resolving"
        );

        // polls: initial
        assert_eq!(h1.poll_count, 1);

        let mut res = buf.write_buf_slice_mut();
        stabilize_date_header(&mut res);
        let res = &res[..];

        let exp = b"\
                HTTP/1.1 200 OK\r\n\
                content-length: 5\r\n\
                date: Thu, 01 Jan 1970 12:34:56 UTC\r\n\r\n\
                /abcd\
                ";

        assert_eq!(
            res,
            exp,
            "\nexpected response not in write buffer:\n\
                     response: {:?}\n\
                     expected: {:?}",
            String::from_utf8_lossy(res),
            String::from_utf8_lossy(exp)
        );
    })
    .await;
}

#[actix_rt::test]
async fn test_req_parse_err() {
    lazy(|cx| {
        let buf = TestBuffer::new("GET /test HTTP/1\r\n\r\n");

        let services = HttpFlow::new(ok_service(), ExpectHandler, None);

        let h1 = Dispatcher::<_, _, _, _, UpgradeHandler>::new(
            buf.clone(),
            services,
            ServiceConfig::default(),
            None,
            OnConnectData::default(),
        );

        actix_rt::pin!(h1);

        match h1.as_mut().poll(cx) {
            Poll::Pending => panic!(),
            Poll::Ready(res) => assert!(res.is_err()),
        }

        if let DispatcherStateProj::Normal { inner } = h1.project().inner.project() {
            assert!(inner.flags.contains(Flags::READ_DISCONNECT));
            assert_eq!(
                &buf.write_buf_slice()[..26],
                b"HTTP/1.1 400 Bad Request\r\n"
            );
        }
    })
    .await;
}

#[actix_rt::test]
async fn pipelining_ok_then_ok() {
    lazy(|cx| {
        let buf = TestBuffer::new(
            "\
                GET /abcd HTTP/1.1\r\n\r\n\
                GET /def HTTP/1.1\r\n\r\n\
                ",
        );

        let cfg = ServiceConfig::new(KeepAlive::Disabled, 1, 1, false, None);

        let services = HttpFlow::new(echo_path_service(), ExpectHandler, None);

        let h1 = Dispatcher::<_, _, _, _, UpgradeHandler>::new(
            buf.clone(),
            services,
            cfg,
            None,
            OnConnectData::default(),
        );

        actix_rt::pin!(h1);

        assert!(matches!(&h1.inner, DispatcherState::Normal { .. }));

        match h1.as_mut().poll(cx) {
            Poll::Pending => panic!("first poll should not be pending"),
            Poll::Ready(res) => assert!(res.is_ok()),
        }

        // polls: initial => shutdown
        assert_eq!(h1.poll_count, 2);

        let mut res = buf.write_buf_slice_mut();
        stabilize_date_header(&mut res);
        let res = &res[..];

        let exp = b"\
                HTTP/1.1 200 OK\r\n\
                content-length: 5\r\n\
                connection: close\r\n\
                date: Thu, 01 Jan 1970 12:34:56 UTC\r\n\r\n\
                /abcd\
                HTTP/1.1 200 OK\r\n\
                content-length: 4\r\n\
                connection: close\r\n\
                date: Thu, 01 Jan 1970 12:34:56 UTC\r\n\r\n\
                /def\
                ";

        assert_eq!(
            res,
            exp,
            "\nexpected response not in write buffer:\n\
               response: {:?}\n\
               expected: {:?}",
            String::from_utf8_lossy(res),
            String::from_utf8_lossy(exp)
        );
    })
    .await;
}

#[actix_rt::test]
async fn pipelining_ok_then_bad() {
    lazy(|cx| {
        let buf = TestBuffer::new(
            "\
                GET /abcd HTTP/1.1\r\n\r\n\
                GET /def HTTP/1\r\n\r\n\
                ",
        );

        let cfg = ServiceConfig::new(KeepAlive::Disabled, 1, 1, false, None);

        let services = HttpFlow::new(echo_path_service(), ExpectHandler, None);

        let h1 = Dispatcher::<_, _, _, _, UpgradeHandler>::new(
            buf.clone(),
            services,
            cfg,
            None,
            OnConnectData::default(),
        );

        actix_rt::pin!(h1);

        assert!(matches!(&h1.inner, DispatcherState::Normal { .. }));

        match h1.as_mut().poll(cx) {
            Poll::Pending => panic!("first poll should not be pending"),
            Poll::Ready(res) => assert!(res.is_err()),
        }

        // polls: initial => shutdown
        assert_eq!(h1.poll_count, 1);

        let mut res = buf.write_buf_slice_mut();
        stabilize_date_header(&mut res);
        let res = &res[..];

        let exp = b"\
                HTTP/1.1 200 OK\r\n\
                content-length: 5\r\n\
                connection: close\r\n\
                date: Thu, 01 Jan 1970 12:34:56 UTC\r\n\r\n\
                /abcd\
                HTTP/1.1 400 Bad Request\r\n\
                content-length: 0\r\n\
                connection: close\r\n\
                date: Thu, 01 Jan 1970 12:34:56 UTC\r\n\r\n\
                ";

        assert_eq!(
            res,
            exp,
            "\nexpected response not in write buffer:\n\
               response: {:?}\n\
               expected: {:?}",
            String::from_utf8_lossy(res),
            String::from_utf8_lossy(exp)
        );
    })
    .await;
}

#[actix_rt::test]
async fn test_expect() {
    lazy(|cx| {
        let mut buf = TestSeqBuffer::empty();
        let cfg = ServiceConfig::new(KeepAlive::Disabled, 0, 0, false, None);

        let services = HttpFlow::new(echo_payload_service(), ExpectHandler, None);

        let h1 = Dispatcher::<_, _, _, _, UpgradeHandler>::new(
            buf.clone(),
            services,
            cfg,
            None,
            OnConnectData::default(),
        );

        buf.extend_read_buf(
            "\
                POST /upload HTTP/1.1\r\n\
                Content-Length: 5\r\n\
                Expect: 100-continue\r\n\
                \r\n\
                ",
        );

        actix_rt::pin!(h1);

        assert!(h1.as_mut().poll(cx).is_pending());
        assert!(matches!(&h1.inner, DispatcherState::Normal { .. }));

        // polls: manual
        assert_eq!(h1.poll_count, 1);
        eprintln!("poll count: {}", h1.poll_count);

        if let DispatcherState::Normal { ref inner } = h1.inner {
            let io = inner.io.as_ref().unwrap();
            let res = &io.write_buf()[..];
            assert_eq!(
                str::from_utf8(res).unwrap(),
                "HTTP/1.1 100 Continue\r\n\r\n"
            );
        }

        buf.extend_read_buf("12345");
        assert!(h1.as_mut().poll(cx).is_ready());

        // polls: manual manual shutdown
        assert_eq!(h1.poll_count, 3);

        if let DispatcherState::Normal { ref inner } = h1.inner {
            let io = inner.io.as_ref().unwrap();
            let mut res = (&io.write_buf()[..]).to_owned();
            stabilize_date_header(&mut res);

            assert_eq!(
                str::from_utf8(&res).unwrap(),
                "\
                    HTTP/1.1 100 Continue\r\n\
                    \r\n\
                    HTTP/1.1 200 OK\r\n\
                    content-length: 5\r\n\
                    connection: close\r\n\
                    date: Thu, 01 Jan 1970 12:34:56 UTC\r\n\
                    \r\n\
                    12345\
                    "
            );
        }
    })
    .await;
}

#[actix_rt::test]
async fn test_eager_expect() {
    lazy(|cx| {
        let mut buf = TestSeqBuffer::empty();
        let cfg = ServiceConfig::new(KeepAlive::Disabled, 0, 0, false, None);

        let services = HttpFlow::new(echo_path_service(), ExpectHandler, None);

        let h1 = Dispatcher::<_, _, _, _, UpgradeHandler>::new(
            buf.clone(),
            services,
            cfg,
            None,
            OnConnectData::default(),
        );

        buf.extend_read_buf(
            "\
                POST /upload HTTP/1.1\r\n\
                Content-Length: 5\r\n\
                Expect: 100-continue\r\n\
                \r\n\
                ",
        );

        actix_rt::pin!(h1);

        assert!(h1.as_mut().poll(cx).is_ready());
        assert!(matches!(&h1.inner, DispatcherState::Normal { .. }));

        // polls: manual shutdown
        assert_eq!(h1.poll_count, 2);

        if let DispatcherState::Normal { ref inner } = h1.inner {
            let io = inner.io.as_ref().unwrap();
            let mut res = (&io.write_buf()[..]).to_owned();
            stabilize_date_header(&mut res);

            // Despite the content-length header and even though the request payload has not
            // been sent, this test expects a complete service response since the payload
            // is not used at all. The service passed to dispatcher is path echo and doesn't
            // consume payload bytes.
            assert_eq!(
                str::from_utf8(&res).unwrap(),
                "\
                    HTTP/1.1 100 Continue\r\n\
                    \r\n\
                    HTTP/1.1 200 OK\r\n\
                    content-length: 7\r\n\
                    connection: close\r\n\
                    date: Thu, 01 Jan 1970 12:34:56 UTC\r\n\
                    \r\n\
                    /upload\
                    "
            );
        }
    })
    .await;
}

#[actix_rt::test]
async fn test_upgrade() {
    struct TestUpgrade;

    impl<T> Service<(Request, Framed<T, Codec>)> for TestUpgrade {
        type Response = ();
        type Error = Error;
        type Future = Ready<Result<Self::Response, Self::Error>>;

        actix_service::always_ready!();

        fn call(&self, (req, _framed): (Request, Framed<T, Codec>)) -> Self::Future {
            assert_eq!(req.method(), Method::GET);
            assert!(req.upgrade());
            assert_eq!(req.headers().get("upgrade").unwrap(), "websocket");
            ready(Ok(()))
        }
    }

    lazy(|cx| {
        let mut buf = TestSeqBuffer::empty();
        let cfg = ServiceConfig::new(KeepAlive::Disabled, 0, 0, false, None);

        let services = HttpFlow::new(ok_service(), ExpectHandler, Some(TestUpgrade));

        let h1 = Dispatcher::<_, _, _, _, TestUpgrade>::new(
            buf.clone(),
            services,
            cfg,
            None,
            OnConnectData::default(),
        );

        buf.extend_read_buf(
            "\
                GET /ws HTTP/1.1\r\n\
                Connection: Upgrade\r\n\
                Upgrade: websocket\r\n\
                \r\n\
                ",
        );

        actix_rt::pin!(h1);

        assert!(h1.as_mut().poll(cx).is_ready());
        assert!(matches!(&h1.inner, DispatcherState::Upgrade { .. }));

        // polls: manual shutdown
        assert_eq!(h1.poll_count, 2);
    })
    .await;
}
