use std::future::Future;

use actix_service::{
    boxed::{self, BoxServiceFactory},
    fn_service,
};

use crate::{
    service::{ServiceRequest, ServiceResponse},
    Error, FromRequestX, HttpResponse, Responder,
};

// TODO inaccessible docs
/// A request handler is an async function that accepts zero or more parameters that can be
/// extracted from a request (i.e., [`impl FromRequest`](crate::FromRequest)) and returns a type
/// that can be converted into an [`HttpResponse`] (that is, it impls the [`Responder`] trait).
///
/// If you got the error `the trait Handler<_, _, _> is not implemented`, then your function is not
/// a valid handler. See [Request Handlers](https://actix.rs/docs/handlers/) for more information.
pub trait Handler<'a, T: FromRequestX<'a>>: Clone + 'static {
    // TODO why 'static ??
    type Response: Responder + 'static;
    type Future: Future<Output = Self::Response>;

    fn handle(&'a self, _: T::Output) -> Self::Future;
}

impl<'a, F, T, Fut, Resp> Handler<'a, T> for F
where
    F: FnX<T>,
    F: FnX<T::Output, Output = Fut>,
    F: Clone + 'static,
    T: FromRequestX<'a>,
    Fut: Future<Output = Resp>,
    Resp: Responder + 'static,
{
    type Response = Resp;
    type Future = Fut;

    fn handle(&'a self, data: T::Output) -> Self::Future {
        self.call(data)
    }
}

pub fn handler_service<H, T>(
    handler: H,
) -> BoxServiceFactory<(), ServiceRequest, ServiceResponse, Error, ()>
where
    H: for<'a> Handler<'a, T>,
    T: for<'a> FromRequestX<'a>,
{
    boxed::factory(fn_service(move |req: ServiceRequest| {
        let handler = handler.clone();
        async move {
            let (req, mut payload) = req.into_parts();
            let res = match T::from_request(&req, &mut payload).await {
                Err(err) => HttpResponse::from_error(err),
                Ok(data) => handler.handle(data).await.respond_to(&req),
            };
            Ok(ServiceResponse::new(req, res))
        }
    }))
}

/// Same as [`std::ops::Fn`]
pub trait FnX<Args> {
    type Output;
    fn call(&self, args: Args) -> Self::Output;
}

/// FromRequest trait impl for tuples
macro_rules! fn_tuple ({ $($param:ident)* } => {
    impl<Func, $($param,)* O> FnX<($($param,)*)> for Func
    where Func: Fn($($param),*) -> O,
    {
        type Output = O;

        #[allow(non_snake_case)]
        fn call(&self, ($($param,)*): ($($param,)*)) -> O {
            (self)($($param,)*)
        }
    }
});

fn_tuple! {}
fn_tuple! { A }
fn_tuple! { A B }
fn_tuple! { A B C }
fn_tuple! { A B C D }
fn_tuple! { A B C D E }
fn_tuple! { A B C D E F }
fn_tuple! { A B C D E F G }
fn_tuple! { A B C D E F G H }
fn_tuple! { A B C D E F G H I }
fn_tuple! { A B C D E F G H I J }
fn_tuple! { A B C D E F G H I J K }
fn_tuple! { A B C D E F G H I J K L }

#[cfg(test)]
mod test {
    use serde::Deserialize;

    use super::*;
    use crate::{
        dev::Service,
        http::StatusCode,
        test::{init_service, TestRequest},
        web, App, HttpRequest, HttpResponse,
    };

    #[derive(Deserialize)]
    struct Params<'a> {
        name: &'a str,
    }

    #[derive(Deserialize)]
    struct ParamsOwned {
        name: String,
    }

    async fn handler(
        req: &HttpRequest,
        (data, params1): (Option<&web::Data<usize>>, web::Path<ParamsOwned>),
    ) -> HttpResponse {
        let params2 = web::Path::<Params<'_>>::extract(req).await.unwrap();
        assert_eq!(params1.name, "named");
        assert_eq!(params2.name, "named");

        assert_eq!(data.unwrap().as_ref(), &42);

        HttpResponse::Ok().finish()
    }

    #[actix_rt::test]
    async fn test_borrowed_extractor() {
        let srv = init_service(
            App::new().service(
                web::resource("/{name}")
                    .app_data(web::Data::new(42usize))
                    .route(web::get().to(handler)),
            ),
        )
        .await;

        let req = TestRequest::with_uri("/named").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
