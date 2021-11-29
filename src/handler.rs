use std::future::Future;

use actix_service::{
    boxed::{self, BoxServiceFactory},
    fn_service,
};

use crate::{
    body::EitherBody,
    service::{ServiceRequest, ServiceResponse},
    Error, FromRequest, HttpResponse, Responder,
};

/// A request handler is an async function that accepts zero or more parameters that can be
/// extracted from a request (i.e., [`impl FromRequest`](crate::FromRequest)) and returns a type
/// that can be converted into an [`HttpResponse`] (that is, it impls the [`Responder`] trait).
///
/// If you got the error `the trait Handler<_, _, _> is not implemented`, then your function is not
/// a valid handler. See [Request Handlers](https://actix.rs/docs/handlers/) for more information.
pub trait Handler<T, R>: Clone + 'static
where
    R: Future,
    R::Output: Responder,
{
    fn call(&self, param: T) -> R;
}

pub fn handler_service<F, T, R>(
    handler: F,
) -> BoxServiceFactory<
    (),
    ServiceRequest,
    ServiceResponse<EitherBody<<R::Output as Responder>::Body>>,
    Error,
    (),
>
where
    F: Handler<T, R>,
    T: FromRequest,
    R: Future,
    R::Output: Responder,
{
    boxed::factory(fn_service(move |req: ServiceRequest| {
        let handler = handler.clone();

        async move {
            let (req, mut payload) = req.into_parts();
            let res = match T::from_request(&req, &mut payload).await {
                Err(err) => {
                    HttpResponse::from_error(err).map_body(|_, body| EitherBody::right(body))
                }

                Ok(data) => handler
                    .call(data)
                    .await
                    .respond_to(&req)
                    .map_body(|_, body| EitherBody::left(body)),
            };

            Ok(ServiceResponse::new(req, res))
        }
    }))
}

/// FromRequest trait impl for tuples
macro_rules! factory_tuple ({ $($param:ident)* } => {
    impl<Func, $($param,)* Res> Handler<($($param,)*), Res> for Func
    where Func: Fn($($param),*) -> Res + Clone + 'static,
          Res: Future,
          Res::Output: Responder,
    {
        #[allow(non_snake_case)]
        fn call(&self, ($($param,)*): ($($param,)*)) -> Res {
            (self)($($param,)*)
        }
    }
});

factory_tuple! {}
factory_tuple! { A }
factory_tuple! { A B }
factory_tuple! { A B C }
factory_tuple! { A B C D }
factory_tuple! { A B C D E }
factory_tuple! { A B C D E F }
factory_tuple! { A B C D E F G }
factory_tuple! { A B C D E F G H }
factory_tuple! { A B C D E F G H I }
factory_tuple! { A B C D E F G H I J }
factory_tuple! { A B C D E F G H I J K }
factory_tuple! { A B C D E F G H I J K L }
