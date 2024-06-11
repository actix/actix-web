use std::{future::Future, str, task::Poll, time::Duration};

use actix_codec::Framed;
use actix_rt::{pin, time::sleep};
use actix_service::{fn_service, Service};
use actix_utils::future::{ready, Ready};
use bytes::{Buf, Bytes, BytesMut};
use futures_util::future::lazy;

use super::dispatcher::{Dispatcher, DispatcherState, DispatcherStateProj, Flags};
use crate::{
    body::MessageBody,
    config::ServiceConfig,
    h1::{Codec, ExpectHandler, UpgradeHandler},
    service::HttpFlow,
    test::{TestBuffer, TestSeqBuffer},
    Error, HttpMessage, KeepAlive, Method, OnConnectData, Request, Response, StatusCode,
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
    status_service(StatusCode::OK)
}

fn status_service(
    status: StatusCode,
) -> impl Service<Request, Response = Response<impl MessageBody>, Error = Error> {
    fn_service(move |_req: Request| ready(Ok::<_, Error>(Response::new(status))))
}

fn echo_path_service() -> impl Service<Request, Response = Response<impl MessageBody>, Error = Error>
{
    fn_service(|req: Request| {
        let path = req.path().as_bytes();
        ready(Ok::<_, Error>(
            Response::ok().set_body(Bytes::copy_from_slice(path)),
        ))
    })
}

fn drop_payload_service() -> impl Service<Request, Response = Response<&'static str>, Error = Error>
{
    fn_service(|mut req: Request| async move {
        let _ = req.take_payload();
        Ok::<_, Error>(Response::with_body(StatusCode::OK, "payload dropped"))
    })
}

fn echo_payload_service() -> impl Service<Request, Response = Response<Bytes>, Error = Error> {
    fn_service(|mut req: Request| {
        Box::pin(async move {
            use futures_util::StreamExt as _;

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
async fn late_request() {
    let mut buf = TestBuffer::empty();

    let cfg = ServiceConfig::new(
        KeepAlive::Disabled,
        Duration::from_millis(100),
        Duration::ZERO,
        false,
        None,
    );
    let services = HttpFlow::new(ok_service(), ExpectHandler, None);

    let h1 = Dispatcher::<_, _, _, _, UpgradeHandler>::new(
        buf.clone(),
        services,
        cfg,
        None,
        OnConnectData::default(),
    );
    pin!(h1);

    lazy(|cx| {
        assert!(matches!(&h1.inner, DispatcherState::Normal { .. }));

        match h1.as_mut().poll(cx) {
            Poll::Ready(_) => panic!("first poll should not be ready"),
            Poll::Pending => {}
        }

        // polls: initial
        assert_eq!(h1.poll_count, 1);

        buf.extend_read_buf("GET /abcd HTTP/1.1\r\nConnection: close\r\n\r\n");

        match h1.as_mut().poll(cx) {
            Poll::Pending => panic!("second poll should not be pending"),
            Poll::Ready(res) => assert!(res.is_ok()),
        }

        // polls: initial pending => handle req => shutdown
        assert_eq!(h1.poll_count, 3);

        let mut res = buf.take_write_buf().to_vec();
        stabilize_date_header(&mut res);
        let res = &res[..];

        let exp = b"\
                HTTP/1.1 200 OK\r\n\
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
async fn oneshot_connection() {
    let buf = TestBuffer::new("GET /abcd HTTP/1.1\r\n\r\n");

    let cfg = ServiceConfig::new(
        KeepAlive::Disabled,
        Duration::from_millis(100),
        Duration::ZERO,
        false,
        None,
    );
    let services = HttpFlow::new(echo_path_service(), ExpectHandler, None);

    let h1 = Dispatcher::<_, _, _, _, UpgradeHandler>::new(
        buf.clone(),
        services,
        cfg,
        None,
        OnConnectData::default(),
    );
    pin!(h1);

    lazy(|cx| {
        assert!(matches!(&h1.inner, DispatcherState::Normal { .. }));

        match h1.as_mut().poll(cx) {
            Poll::Pending => panic!("first poll should not be pending"),
            Poll::Ready(res) => assert!(res.is_ok()),
        }

        // polls: initial => shutdown
        assert_eq!(h1.poll_count, 2);

        let mut res = buf.take_write_buf().to_vec();
        stabilize_date_header(&mut res);
        let res = &res[..];

        let exp = http_msg(
            r"
            HTTP/1.1 200 OK
            content-length: 5
            connection: close
            date: Thu, 01 Jan 1970 12:34:56 UTC

            /abcd
            ",
        );

        assert_eq!(
            res,
            exp,
            "\nexpected response not in write buffer:\n\
               response: {:?}\n\
               expected: {:?}",
            String::from_utf8_lossy(res),
            String::from_utf8_lossy(&exp)
        );
    })
    .await;
}

#[actix_rt::test]
async fn keep_alive_timeout() {
    let buf = TestBuffer::new("GET /abcd HTTP/1.1\r\n\r\n");

    let cfg = ServiceConfig::new(
        KeepAlive::Timeout(Duration::from_millis(200)),
        Duration::from_millis(100),
        Duration::ZERO,
        false,
        None,
    );
    let services = HttpFlow::new(echo_path_service(), ExpectHandler, None);

    let h1 = Dispatcher::<_, _, _, _, UpgradeHandler>::new(
        buf.clone(),
        services,
        cfg,
        None,
        OnConnectData::default(),
    );
    pin!(h1);

    lazy(|cx| {
        assert!(matches!(&h1.inner, DispatcherState::Normal { .. }));

        assert!(
            h1.as_mut().poll(cx).is_pending(),
            "keep-alive should prevent poll from resolving"
        );

        // polls: initial
        assert_eq!(h1.poll_count, 1);

        let mut res = buf.take_write_buf().to_vec();
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

    // sleep slightly longer than keep-alive timeout
    sleep(Duration::from_millis(250)).await;

    lazy(|cx| {
        assert!(
            h1.as_mut().poll(cx).is_ready(),
            "keep-alive should have resolved",
        );

        // polls: initial => keep-alive wake-up shutdown
        assert_eq!(h1.poll_count, 2);

        if let DispatcherStateProj::Normal { inner } = h1.project().inner.project() {
            // connection closed
            assert!(inner.flags.contains(Flags::SHUTDOWN));
            assert!(inner.flags.contains(Flags::WRITE_DISCONNECT));
            // and nothing added to write buffer
            assert!(buf.write_buf_slice().is_empty());
        }
    })
    .await;
}

#[actix_rt::test]
async fn keep_alive_follow_up_req() {
    let mut buf = TestBuffer::new("GET /abcd HTTP/1.1\r\n\r\n");

    let cfg = ServiceConfig::new(
        KeepAlive::Timeout(Duration::from_millis(500)),
        Duration::from_millis(100),
        Duration::ZERO,
        false,
        None,
    );
    let services = HttpFlow::new(echo_path_service(), ExpectHandler, None);

    let h1 = Dispatcher::<_, _, _, _, UpgradeHandler>::new(
        buf.clone(),
        services,
        cfg,
        None,
        OnConnectData::default(),
    );
    pin!(h1);

    lazy(|cx| {
        assert!(matches!(&h1.inner, DispatcherState::Normal { .. }));

        assert!(
            h1.as_mut().poll(cx).is_pending(),
            "keep-alive should prevent poll from resolving"
        );

        // polls: initial
        assert_eq!(h1.poll_count, 1);

        let mut res = buf.take_write_buf().to_vec();
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

    // sleep for less than KA timeout
    sleep(Duration::from_millis(100)).await;

    lazy(|cx| {
        assert!(
            h1.as_mut().poll(cx).is_pending(),
            "keep-alive should not have resolved dispatcher yet",
        );

        // polls: initial => manual
        assert_eq!(h1.poll_count, 2);

        if let DispatcherStateProj::Normal { inner } = h1.as_mut().project().inner.project() {
            // connection not closed
            assert!(!inner.flags.contains(Flags::SHUTDOWN));
            assert!(!inner.flags.contains(Flags::WRITE_DISCONNECT));
            // and nothing added to write buffer
            assert!(buf.write_buf_slice().is_empty());
        }
    })
    .await;

    lazy(|cx| {
        buf.extend_read_buf(
            "\
            GET /efg HTTP/1.1\r\n\
            Connection: close\r\n\
            \r\n\r\n",
        );

        assert!(
            h1.as_mut().poll(cx).is_ready(),
            "connection close header should override keep-alive setting",
        );

        // polls: initial => manual => follow-up req => shutdown
        assert_eq!(h1.poll_count, 4);

        if let DispatcherStateProj::Normal { inner } = h1.as_mut().project().inner.project() {
            // connection closed
            assert!(inner.flags.contains(Flags::SHUTDOWN));
            assert!(!inner.flags.contains(Flags::WRITE_DISCONNECT));
        }

        let mut res = buf.take_write_buf().to_vec();
        stabilize_date_header(&mut res);
        let res = &res[..];

        let exp = b"\
                HTTP/1.1 200 OK\r\n\
                content-length: 4\r\n\
                connection: close\r\n\
                date: Thu, 01 Jan 1970 12:34:56 UTC\r\n\r\n\
                /efg\
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
async fn req_parse_err() {
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

        pin!(h1);

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

        let cfg = ServiceConfig::new(
            KeepAlive::Disabled,
            Duration::from_millis(1),
            Duration::from_millis(1),
            false,
            None,
        );

        let services = HttpFlow::new(echo_path_service(), ExpectHandler, None);

        let h1 = Dispatcher::<_, _, _, _, UpgradeHandler>::new(
            buf.clone(),
            services,
            cfg,
            None,
            OnConnectData::default(),
        );

        pin!(h1);

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

        let cfg = ServiceConfig::new(
            KeepAlive::Disabled,
            Duration::from_millis(1),
            Duration::from_millis(1),
            false,
            None,
        );

        let services = HttpFlow::new(echo_path_service(), ExpectHandler, None);

        let h1 = Dispatcher::<_, _, _, _, UpgradeHandler>::new(
            buf.clone(),
            services,
            cfg,
            None,
            OnConnectData::default(),
        );

        pin!(h1);

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
async fn expect_handling() {
    lazy(|cx| {
        let mut buf = TestSeqBuffer::empty();
        let cfg = ServiceConfig::new(
            KeepAlive::Disabled,
            Duration::ZERO,
            Duration::ZERO,
            false,
            None,
        );

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

        pin!(h1);

        assert!(h1.as_mut().poll(cx).is_pending());
        assert!(matches!(&h1.inner, DispatcherState::Normal { .. }));

        // polls: manual
        assert_eq!(h1.poll_count, 1);

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
            let mut res = io.write_buf()[..].to_owned();
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
async fn expect_eager() {
    lazy(|cx| {
        let mut buf = TestSeqBuffer::empty();
        let cfg = ServiceConfig::new(
            KeepAlive::Disabled,
            Duration::ZERO,
            Duration::ZERO,
            false,
            None,
        );

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

        pin!(h1);

        assert!(h1.as_mut().poll(cx).is_ready());
        assert!(matches!(&h1.inner, DispatcherState::Normal { .. }));

        // polls: manual shutdown
        assert_eq!(h1.poll_count, 2);

        if let DispatcherState::Normal { ref inner } = h1.inner {
            let io = inner.io.as_ref().unwrap();
            let mut res = io.write_buf()[..].to_owned();
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
async fn upgrade_handling() {
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
        let cfg = ServiceConfig::new(
            KeepAlive::Disabled,
            Duration::ZERO,
            Duration::ZERO,
            false,
            None,
        );

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

        pin!(h1);

        assert!(h1.as_mut().poll(cx).is_ready());
        assert!(matches!(&h1.inner, DispatcherState::Upgrade { .. }));

        // polls: manual shutdown
        assert_eq!(h1.poll_count, 2);
    })
    .await;
}

// fix in #2624 reverted temporarily
// complete fix tracked in #2745
#[ignore]
#[actix_rt::test]
async fn handler_drop_payload() {
    let _ = env_logger::try_init();

    let mut buf = TestBuffer::new(http_msg(
        r"
        POST /drop-payload HTTP/1.1
        Content-Length: 3
        
        abc
        ",
    ));

    let services = HttpFlow::new(
        drop_payload_service(),
        ExpectHandler,
        None::<UpgradeHandler>,
    );

    let h1 = Dispatcher::new(
        buf.clone(),
        services,
        ServiceConfig::default(),
        None,
        OnConnectData::default(),
    );
    pin!(h1);

    lazy(|cx| {
        assert!(h1.as_mut().poll(cx).is_pending());

        // polls: manual
        assert_eq!(h1.poll_count, 1);

        let mut res = BytesMut::from(buf.take_write_buf().as_ref());
        stabilize_date_header(&mut res);
        let res = &res[..];

        let exp = http_msg(
            r"
            HTTP/1.1 200 OK
            content-length: 15
            date: Thu, 01 Jan 1970 12:34:56 UTC

            payload dropped
            ",
        );

        assert_eq!(
            res,
            exp,
            "\nexpected response not in write buffer:\n\
               response: {:?}\n\
               expected: {:?}",
            String::from_utf8_lossy(res),
            String::from_utf8_lossy(&exp)
        );

        if let DispatcherStateProj::Normal { inner } = h1.as_mut().project().inner.project() {
            assert!(inner.state.is_none());
        }
    })
    .await;

    lazy(|cx| {
        // add message that claims to have payload longer than provided
        buf.extend_read_buf(http_msg(
            r"
            POST /drop-payload HTTP/1.1
            Content-Length: 200
            
            abc
            ",
        ));

        assert!(h1.as_mut().poll(cx).is_pending());

        // polls: manual => manual
        assert_eq!(h1.poll_count, 2);

        let mut res = BytesMut::from(buf.take_write_buf().as_ref());
        stabilize_date_header(&mut res);
        let res = &res[..];

        // expect response immediately even though request side has not finished reading payload
        let exp = http_msg(
            r"
            HTTP/1.1 200 OK
            content-length: 15
            date: Thu, 01 Jan 1970 12:34:56 UTC

            payload dropped
            ",
        );

        assert_eq!(
            res,
            exp,
            "\nexpected response not in write buffer:\n\
               response: {:?}\n\
               expected: {:?}",
            String::from_utf8_lossy(res),
            String::from_utf8_lossy(&exp)
        );
    })
    .await;

    lazy(|cx| {
        assert!(h1.as_mut().poll(cx).is_ready());

        // polls: manual => manual => manual
        assert_eq!(h1.poll_count, 3);

        let mut res = BytesMut::from(buf.take_write_buf().as_ref());
        stabilize_date_header(&mut res);
        let res = &res[..];

        // expect that unrequested error response is sent back since connection could not be cleaned
        let exp = http_msg(
            r"
            HTTP/1.1 500 Internal Server Error
            content-length: 0
            connection: close
            date: Thu, 01 Jan 1970 12:34:56 UTC

            ",
        );

        assert_eq!(
            res,
            exp,
            "\nexpected response not in write buffer:\n\
               response: {:?}\n\
               expected: {:?}",
            String::from_utf8_lossy(res),
            String::from_utf8_lossy(&exp)
        );
    })
    .await;
}

fn http_msg(msg: impl AsRef<str>) -> BytesMut {
    let mut msg = msg
        .as_ref()
        .trim()
        .split('\n')
        .map(|line| [line.trim_start(), "\r"].concat())
        .collect::<Vec<_>>()
        .join("\n");

    // remove trailing \r
    msg.pop();

    if !msg.is_empty() && !msg.contains("\r\n\r\n") {
        msg.push_str("\r\n\r\n");
    }

    BytesMut::from(msg.as_bytes())
}

#[test]
fn http_msg_creates_msg() {
    assert_eq!(http_msg(r""), "");

    assert_eq!(
        http_msg(
            r"
            POST / HTTP/1.1
            Content-Length: 3
            
            abc
            "
        ),
        "POST / HTTP/1.1\r\nContent-Length: 3\r\n\r\nabc"
    );

    assert_eq!(
        http_msg(
            r"
            GET / HTTP/1.1
            Content-Length: 3
            
            "
        ),
        "GET / HTTP/1.1\r\nContent-Length: 3\r\n\r\n"
    );
}
