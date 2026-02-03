use std::{future::Future, marker::PhantomData, rc::Rc};

use actix_service::boxed::{self, BoxFuture, RcService};
use actix_utils::future::{ready, Ready};
use futures_core::future::LocalBoxFuture;

use crate::{
    body::MessageBody,
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error, FromRequest,
};

/// Wraps an async function to be used as a middleware.
///
/// # Examples
///
/// The wrapped function should have the following form:
///
/// ```
/// # use actix_web::{
/// #     App, Error,
/// #     body::MessageBody,
/// #     dev::{ServiceRequest, ServiceResponse, Service as _},
/// # };
/// use actix_web::middleware::{self, Next};
///
/// async fn my_mw(
///     req: ServiceRequest,
///     next: Next<impl MessageBody>,
/// ) -> Result<ServiceResponse<impl MessageBody>, Error> {
///     // pre-processing
///     next.call(req).await
///     // post-processing
/// }
/// # App::new().wrap(middleware::from_fn(my_mw));
/// ```
///
/// Then use in an app builder like this:
///
/// ```
/// use actix_web::{
///     App, Error,
///     dev::{ServiceRequest, ServiceResponse, Service as _},
/// };
/// use actix_web::middleware::from_fn;
/// # use actix_web::middleware::Next;
/// # async fn my_mw<B>(req: ServiceRequest, next: Next<B>) -> Result<ServiceResponse<B>, Error> {
/// #     next.call(req).await
/// # }
///
/// App::new()
///     .wrap(from_fn(my_mw))
/// # ;
/// ```
///
/// It is also possible to write a middleware that automatically uses extractors, similar to request
/// handlers, by declaring them as the first parameters. As usual, **take care with extractors that
/// consume the body stream**, since handlers will no longer be able to read it again without
/// putting the body "back" into the request object within your middleware.
///
/// ```
/// # use std::collections::HashMap;
/// # use actix_web::{
/// #     App, Error,
/// #     body::MessageBody,
/// #     dev::{ServiceRequest, ServiceResponse},
/// #     http::header::{Accept, Date},
/// #     web::{Header, Query},
/// # };
/// use actix_web::middleware::Next;
///
/// async fn my_extracting_mw(
///     accept: Header<Accept>,
///     query: Query<HashMap<String, String>>,
///     req: ServiceRequest,
///     next: Next<impl MessageBody>,
/// ) -> Result<ServiceResponse<impl MessageBody>, Error> {
///     // pre-processing
///     next.call(req).await
///     // post-processing
/// }
/// # App::new().wrap(actix_web::middleware::from_fn(my_extracting_mw));
pub fn from_fn<F, Es>(mw_fn: F) -> MiddlewareFn<F, Es> {
    MiddlewareFn {
        mw_fn: Rc::new(mw_fn),
        _phantom: PhantomData,
    }
}

/// Middleware transform for [`from_fn`].
#[allow(missing_debug_implementations)]
pub struct MiddlewareFn<F, Es> {
    mw_fn: Rc<F>,
    _phantom: PhantomData<Es>,
}

impl<S, F, Fut, B, B2> Transform<S, ServiceRequest> for MiddlewareFn<F, ()>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    F: Fn(ServiceRequest, Next<B>) -> Fut + 'static,
    Fut: Future<Output = Result<ServiceResponse<B2>, Error>>,
    B2: MessageBody,
{
    type Response = ServiceResponse<B2>;
    type Error = Error;
    type Transform = MiddlewareFnService<F, B, ()>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(MiddlewareFnService {
            service: boxed::rc_service(service),
            mw_fn: Rc::clone(&self.mw_fn),
            _phantom: PhantomData,
        }))
    }
}

/// Middleware service for [`from_fn`].
#[allow(missing_debug_implementations)]
pub struct MiddlewareFnService<F, B, Es> {
    service: RcService<ServiceRequest, ServiceResponse<B>, Error>,
    mw_fn: Rc<F>,
    _phantom: PhantomData<(B, Es)>,
}

impl<F, Fut, B, B2> Service<ServiceRequest> for MiddlewareFnService<F, B, ()>
where
    F: Fn(ServiceRequest, Next<B>) -> Fut,
    Fut: Future<Output = Result<ServiceResponse<B2>, Error>>,
    B2: MessageBody,
{
    type Response = ServiceResponse<B2>;
    type Error = Error;
    type Future = Fut;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        (self.mw_fn)(
            req,
            Next::<B> {
                service: Rc::clone(&self.service),
            },
        )
    }
}

macro_rules! impl_middleware_fn_service {
    ($($ext_type:ident),*) => {
        impl<S, F, Fut, B, B2, $($ext_type),*> Transform<S, ServiceRequest> for MiddlewareFn<F, ($($ext_type),*,)>
        where
            S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
            F: Fn($($ext_type),*, ServiceRequest, Next<B>) -> Fut + 'static,
            $($ext_type: FromRequest + 'static,)*
            Fut: Future<Output = Result<ServiceResponse<B2>, Error>> + 'static,
            B: MessageBody + 'static,
            B2: MessageBody + 'static,
        {
            type Response = ServiceResponse<B2>;
            type Error = Error;
            type Transform = MiddlewareFnService<F, B, ($($ext_type,)*)>;
            type InitError = ();
            type Future = Ready<Result<Self::Transform, Self::InitError>>;

            fn new_transform(&self, service: S) -> Self::Future {
                ready(Ok(MiddlewareFnService {
                    service: boxed::rc_service(service),
                    mw_fn: Rc::clone(&self.mw_fn),
                    _phantom: PhantomData,
                }))
            }
        }

        impl<F, $($ext_type),*, Fut, B: 'static, B2> Service<ServiceRequest>
            for MiddlewareFnService<F, B, ($($ext_type),*,)>
        where
            F: Fn(
                $($ext_type),*,
                ServiceRequest,
                Next<B>
            ) -> Fut + 'static,
            $($ext_type: FromRequest + 'static,)*
            Fut: Future<Output = Result<ServiceResponse<B2>, Error>> + 'static,
            B2: MessageBody + 'static,
        {
            type Response = ServiceResponse<B2>;
            type Error = Error;
            type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

            forward_ready!(service);

            #[allow(nonstandard_style)]
            fn call(&self, mut req: ServiceRequest) -> Self::Future {
                let mw_fn = Rc::clone(&self.mw_fn);
                let service = Rc::clone(&self.service);

                Box::pin(async move {
                    let ($($ext_type,)*) = req.extract::<($($ext_type,)*)>().await?;

                    (mw_fn)($($ext_type),*, req, Next::<B> { service }).await
                })
            }
        }
    };
}

impl_middleware_fn_service!(E1);
impl_middleware_fn_service!(E1, E2);
impl_middleware_fn_service!(E1, E2, E3);
impl_middleware_fn_service!(E1, E2, E3, E4);
impl_middleware_fn_service!(E1, E2, E3, E4, E5);
impl_middleware_fn_service!(E1, E2, E3, E4, E5, E6);
impl_middleware_fn_service!(E1, E2, E3, E4, E5, E6, E7);
impl_middleware_fn_service!(E1, E2, E3, E4, E5, E6, E7, E8);
impl_middleware_fn_service!(E1, E2, E3, E4, E5, E6, E7, E8, E9);

/// Wraps the "next" service in the middleware chain.
#[allow(missing_debug_implementations)]
pub struct Next<B> {
    service: RcService<ServiceRequest, ServiceResponse<B>, Error>,
}

impl<B> Next<B> {
    /// Equivalent to `Service::call(self, req)`.
    pub fn call(&self, req: ServiceRequest) -> <Self as Service<ServiceRequest>>::Future {
        Service::call(self, req)
    }
}

impl<B> Service<ServiceRequest> for Next<B> {
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = BoxFuture<Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        self.service.call(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        http::header::{self, HeaderValue},
        middleware::{Compat, Logger},
        test, web, App, HttpResponse,
    };

    async fn noop<B>(req: ServiceRequest, next: Next<B>) -> Result<ServiceResponse<B>, Error> {
        next.call(req).await
    }

    async fn add_res_header<B>(
        req: ServiceRequest,
        next: Next<B>,
    ) -> Result<ServiceResponse<B>, Error> {
        let mut res = next.call(req).await?;
        res.headers_mut()
            .insert(header::WARNING, HeaderValue::from_static("42"));
        Ok(res)
    }

    async fn mutate_body_type(
        req: ServiceRequest,
        next: Next<impl MessageBody + 'static>,
    ) -> Result<ServiceResponse<impl MessageBody>, Error> {
        let res = next.call(req).await?;
        Ok(res.map_into_left_body::<()>())
    }

    struct MyMw(bool);

    impl MyMw {
        async fn mw_cb(
            &self,
            req: ServiceRequest,
            next: Next<impl MessageBody + 'static>,
        ) -> Result<ServiceResponse<impl MessageBody>, Error> {
            let mut res = match self.0 {
                true => req.into_response("short-circuited").map_into_right_body(),
                false => next.call(req).await?.map_into_left_body(),
            };
            res.headers_mut()
                .insert(header::WARNING, HeaderValue::from_static("42"));
            Ok(res)
        }

        pub fn into_middleware<S, B>(
            self,
        ) -> impl Transform<
            S,
            ServiceRequest,
            Response = ServiceResponse<impl MessageBody>,
            Error = Error,
            InitError = (),
        >
        where
            S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
            B: MessageBody + 'static,
        {
            let this = Rc::new(self);
            from_fn(move |req, next| {
                let this = Rc::clone(&this);
                async move { Self::mw_cb(&this, req, next).await }
            })
        }
    }

    #[actix_rt::test]
    async fn compat_compat() {
        let _ = App::new().wrap(Compat::new(from_fn(noop)));
        let _ = App::new().wrap(Compat::new(from_fn(mutate_body_type)));
    }

    #[actix_rt::test]
    async fn permits_different_in_and_out_body_types() {
        let app = test::init_service(
            App::new()
                .wrap(from_fn(mutate_body_type))
                .wrap(from_fn(add_res_header))
                .wrap(Logger::default())
                .wrap(from_fn(noop))
                .default_service(web::to(HttpResponse::NotFound)),
        )
        .await;

        let req = test::TestRequest::default().to_request();
        let res = test::call_service(&app, req).await;
        assert!(res.headers().contains_key(header::WARNING));
    }

    #[actix_rt::test]
    async fn closure_capture_and_return_from_fn() {
        let app = test::init_service(
            App::new()
                .wrap(Logger::default())
                .wrap(MyMw(true).into_middleware())
                .wrap(Logger::default()),
        )
        .await;

        let req = test::TestRequest::default().to_request();
        let res = test::call_service(&app, req).await;
        assert!(res.headers().contains_key(header::WARNING));
    }
}
