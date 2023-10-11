use std::future::Future;

use actix_service::{boxed, fn_service};

use crate::{
    service::{BoxedHttpServiceFactory, ServiceRequest, ServiceResponse},
    FromRequest, HttpResponse, Responder,
};

/// The interface for request handlers.
///
/// # What Is A Request Handler
///
/// In short, a handler is just an async function that receives request-based arguments, in any
/// order, and returns something that can be converted to a response.
///
/// In particular, a request handler has three requirements:
///
/// 1. It is an async function (or a function/closure that returns an appropriate future);
/// 1. The function parameters (up to 12) implement [`FromRequest`];
/// 1. The async function (or future) resolves to a type that can be converted into an
///   [`HttpResponse`] (i.e., it implements the [`Responder`] trait).
///
///
/// # Compiler Errors
///
/// If you get the error `the trait Handler<_> is not implemented`, then your handler does not
/// fulfill the _first_ of the above requirements. (It could also mean that you're attempting to use
/// a macro-routed handler in a manual routing context like `web::get().to(handler)`, which is not
/// supported). Breaking the other requirements manifests as errors on implementing [`FromRequest`]
/// and [`Responder`], respectively.
///
/// # How Do Handlers Receive Variable Numbers Of Arguments
///
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
/// destructures the tuple into its component types and calls your handler function with them.
///
/// In pseudo-code the process looks something like this:
///
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
/// impl<Func, Arg1, Arg2, Fut> Handler<(Arg1, Arg2)> for Func
/// where
///     Func: Fn(Arg1, Arg2) -> Fut + Clone + 'static,
///     Fut: Future,
/// {
///     type Output = Fut::Output;
///     type Future = Fut;
///
///     fn call(&self, (arg1, arg2): (Arg1, Arg2)) -> Self::Future {
///         (self)(arg1, arg2)
///     }
/// }
/// ```
///
/// [arity]: https://en.wikipedia.org/wiki/Arity
/// [`from_request`]: FromRequest::from_request
pub trait Handler<Args>: Clone + 'static {
    type Output;
    type Future: Future<Output = Self::Output>;

    fn call(&self, args: Args) -> Self::Future;
}

pub(crate) fn handler_service<F, Args>(handler: F) -> BoxedHttpServiceFactory
where
    F: Handler<Args>,
    Args: FromRequest,
    F::Output: Responder,
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
    impl<Func, Fut, $($param,)*> Handler<($($param,)*)> for Func
    where
        Func: Fn($($param),*) -> Fut + Clone + 'static,
        Fut: Future,
    {
        type Output = Fut::Output;
        type Future = Fut;

        #[inline]
        #[allow(non_snake_case)]
        fn call(&self, ($($param,)*): ($($param,)*)) -> Self::Future {
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
factory_tuple! { A B C D E F G H I J K L M }
factory_tuple! { A B C D E F G H I J K L M N }
factory_tuple! { A B C D E F G H I J K L M N O }
factory_tuple! { A B C D E F G H I J K L M N O P }

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_impl_handler<T: FromRequest>(_: impl Handler<T>) {}

    #[test]
    fn arg_number() {
        async fn handler_min() {}

        #[rustfmt::skip]
        #[allow(clippy::too_many_arguments, clippy::just_underscores_and_digits, clippy::let_unit_value)]
        async fn handler_max(
            _01: (), _02: (), _03: (), _04: (), _05: (), _06: (),
            _07: (), _08: (), _09: (), _10: (), _11: (), _12: (),
            _13: (), _14: (), _15: (), _16: (),
        ) {}

        assert_impl_handler(handler_min);
        assert_impl_handler(handler_max);
    }
}
