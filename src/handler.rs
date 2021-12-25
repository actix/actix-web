use std::future::Future;

use actix_service::{boxed, fn_service};

use crate::{
    body::MessageBody,
    service::{BoxedHttpServiceFactory, ServiceRequest, ServiceResponse},
    BoxError, FromRequest, HttpResponse, Responder,
};

/// The interface for request handlers.
///
/// # What Is A Request Handler
/// A request handler has three requirements:
/// 1. It is an async function (or a function/closure that returns an appropriate future);
/// 1. The function accepts zero or more parameters that implement [`FromRequest`];
/// 1. The async function (or future) resolves to a type that can be converted into an
///   [`HttpResponse`] (i.e., it implements the [`Responder`] trait).
///
/// # Compiler Errors
/// If you get the error `the trait Handler<_, _, _> is not implemented`, then your handler does not
/// fulfill one or more of the above requirements.
///
/// Unfortunately we cannot provide a better compile error message (while keeping the trait's
/// flexibility) unless a stable alternative to [`#[rustc_on_unimplemented]`][on_unimpl] is added
/// to Rust.
///
/// # How Do Handlers Receive Variable Numbers Of Arguments
/// Rest assured there is no macro magic here; it's just traits.
///
/// The first thing to note is that [`FromRequest`] is implemented for tuples (up to 12 in length).
///
/// Secondly, the `Handler` trait is implemented for functions (up to an [arity] of 12) in a way
/// that aligns their parameter positions with a corresponding tuple of types (becoming the `Args`
/// type parameter for this trait).
///
/// Thanks to Rust's type system, Actix Web can infer the function parameter types. During the
/// extraction step, the parameter types are described as a tuple type, [`from_request`] is run on
/// that tuple, and the `Handler::call` implementation for that particular function arity
/// destructures the tuple into it's component types and calls your handler function with them.
///
/// In pseudo-code the process looks something like this:
/// ```ignore
/// async fn my_handler(body: String, state: web::Data<MyState>) -> impl Responder {
///     ...
/// }
///
/// // the function params above described as a tuple, names do not matter, only position
/// type InferredMyHandlerArgs = (String, web::Data<MyState>);
///
/// // create tuple of arguments to be passed to handler
/// let args = InferredMyHandlerArgs::from_request(&request, &payload).await;
///
/// // call handler with argument tuple
/// let response = Handler::call(&my_handler, args).await;
///
/// // which is effectively...
///
/// let (body, state) = args;
/// let response = my_handler(body, state).await;
/// ```
///
/// This is the source code for the 2-parameter implementation of `Handler` to help illustrate the
/// bounds of the handler call after argument extraction:
/// ```ignore
/// impl<Func, Arg1, Arg2, R> Handler<(Arg1, Arg2), R> for Func
/// where
///     Func: Fn(Arg1, Arg2) -> R + Clone + 'static,
///     R: Future,
///     R::Output: Responder,
/// {
///     fn call(&self, (arg1, arg2): (Arg1, Arg2)) -> R {
///         (self)(arg1, arg2)
///     }
/// }
/// ```
///
/// [arity]: https://en.wikipedia.org/wiki/Arity
/// [`from_request`]: FromRequest::from_request
/// [on_unimpl]: https://github.com/rust-lang/rust/issues/29628
pub trait Handler<Args, R>: Clone + 'static
where
    R: Future,
    R::Output: Responder,
{
    fn call(&self, args: Args) -> R;
}

pub(crate) fn handler_service<F, Args, R>(handler: F) -> BoxedHttpServiceFactory
where
    F: Handler<Args, R>,
    Args: FromRequest,
    R: Future,
    R::Output: Responder,
    <R::Output as Responder>::Body: MessageBody,
    <<R::Output as Responder>::Body as MessageBody>::Error: Into<BoxError>,
{
    boxed::factory(fn_service(move |req: ServiceRequest| {
        let handler = handler.clone();

        async move {
            let (req, mut payload) = req.into_parts();

            let res = match Args::from_request(&req, &mut payload).await {
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
/// factory_tuple! {}         // implements Handler for types: fn() -> R
/// factory_tuple! { A B C }  // implements Handler for types: fn(A, B, C) -> R
/// ```
macro_rules! factory_tuple ({ $($param:ident)* } => {
    impl<Func, $($param,)* R> Handler<($($param,)*), R> for Func
    where Func: Fn($($param),*) -> R + Clone + 'static,
          R: Future,
          R::Output: Responder,
    {
        #[inline]
        #[allow(non_snake_case)]
        fn call(&self, ($($param,)*): ($($param,)*)) -> R {
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
