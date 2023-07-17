//! For middleware documentation, see [`ConditionOption`].

use futures_core::future::LocalBoxFuture;
use futures_util::future::FutureExt as _;

use crate::{
    body::EitherBody,
    dev::{Service, ServiceResponse, Transform},
    middleware::condition::ConditionMiddleware,
};

/// Middleware for conditionally enabling other middleware in an [`Option`].
///
/// Uses [`Condition`](crate::middleware::condition::Condition) under the hood.
///
/// # Example
/// ```
/// use actix_web::middleware::{ConditionOption, NormalizePath};
/// use actix_web::App;
///
/// let normalize: ConditionOption<_> = Some(NormalizePath::default()).into();
/// let app = App::new()
///     .wrap(normalize);
/// ```
pub struct ConditionOption<T>(Option<T>);

impl<T> From<Option<T>> for ConditionOption<T> {
    fn from(value: Option<T>) -> Self {
        Self(value)
    }
}

impl<S, T, Req, BE, BD, Err> Transform<S, Req> for ConditionOption<T>
where
    S: Service<Req, Response = ServiceResponse<BD>, Error = Err> + 'static,
    T: Transform<S, Req, Response = ServiceResponse<BE>, Error = Err>,
    T::Future: 'static,
    T::InitError: 'static,
    T::Transform: 'static,
{
    type Response = ServiceResponse<EitherBody<BE, BD>>;
    type Error = Err;
    type Transform = ConditionMiddleware<T::Transform, S>;
    type InitError = T::InitError;
    type Future = LocalBoxFuture<'static, Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        match &self.0 {
            Some(transformer) => {
                let fut = transformer.new_transform(service);
                async move {
                    let wrapped_svc = fut.await?;
                    Ok(ConditionMiddleware::Enable(wrapped_svc))
                }
                .boxed_local()
            }
            None => async move { Ok(ConditionMiddleware::Disable(service)) }.boxed_local(),
        }
    }
}
