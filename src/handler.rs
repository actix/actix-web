use std::{fmt, future::Future};

use actix_service::{boxed, fn_service};

use crate::{
    body::MessageBody,
    service::{BoxedHttpServiceFactory, ServiceRequest, ServiceResponse},
    FromRequest, HttpResponse, Responder,
};

/// A request handler is an async function that accepts zero or more parameters that can be
/// extracted from a request (i.e., [`impl FromRequest`]) and returns a type that can be converted
/// into an [`HttpResponse`] (that is, it impls the [`Responder`] trait).
///
/// If you got the error `the trait Handler<_, _, _> is not implemented`, then your function is not
/// a valid handler. See <https://actix.rs/docs/handlers> for more information.
///
/// [`impl FromRequest`]: crate::FromRequest
pub trait Handler<T, R>: Clone + 'static
where
    R: Future,
    R::Output: Responder,
{
    fn call(&self, param: T) -> R;
}

pub(crate) fn handler_service<F, T, R>(handler: F) -> BoxedHttpServiceFactory
where
    F: Handler<T, R>,
    T: FromRequest,
    R: Future,
    R::Output: Responder,
    <R::Output as Responder>::Body: MessageBody,
    <<R::Output as Responder>::Body as MessageBody>::Error: fmt::Debug,
{
    boxed::factory(fn_service(move |req: ServiceRequest| {
        let handler = handler.clone();

        async move {
            let (req, mut payload) = req.into_parts();

            let res = match T::from_request(&req, &mut payload).await {
                Err(err) => HttpResponse::from_error(err),

                Ok(data) => handler
                    .call(data)
                    .await
                    .respond_to(&req)
                    .map_into_boxed_body(),
            };

            Ok(ServiceResponse::new(req, res))
        }
    }))
}

/// Generates a [`Handler`] trait impl for N-ary functions where N is specified with a sequence of
/// space separated type parameters.
///
/// # Examples
/// ```ignore
/// factory_tuple! {}         // implements Handler for types: fn() -> Res
/// factory_tuple! { A B C }  // implements Handler for types: fn(A, B, C) -> Res
/// ```
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
