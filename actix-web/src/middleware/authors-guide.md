# Middleware Author's Guide

## What Is A Middleware?

Middleware in Actix Web is a powerful mechanism that allows you to add additional behavior to request/response processing. It enables you to:

- Pre-process incoming requests (e.g., path normalization, authentication)
- Post-process outgoing responses (e.g., logging, compression)
- Modify application state through ServiceRequest
- Access external services (e.g., sessions, caching)

Middleware is registered for each App, Scope, or Resource and executed in the reverse order of registration. This means the last registered middleware is the first to process the request.

## Middleware Traits

Actix Web's middleware system is built on two main traits:

1. `Transform<S, Req>`: The builder trait that creates the actual Service. It's responsible for:
   - Creating new middleware instances
   - Assembling the middleware chain
   - Handling initialization errors

2. `Service<Req>`: The trait that represents the actual middleware functionality. It:
   - Processes requests and responses
   - Can modify both request and response
   - Can short-circuit request processing
   - Must be implemented for the middleware to work

## Understanding Body Types

When working with middleware, it's important to understand body types:

- Middleware can work with different body types for requests and responses
- The `MessageBody` trait is used to handle different body types
- You can use `EitherBody` when you need to handle multiple body types
- Be careful with body consumption - once a body is consumed, it cannot be read again

## Best Practices

1. Keep middleware focused and single-purpose
2. Handle errors appropriately and propagate them correctly
3. Be mindful of performance impact
4. Use appropriate body types and handle them correctly
5. Consider middleware ordering carefully
6. Document your middleware's behavior and requirements
7. Test your middleware thoroughly

## Error Propagation

Proper error handling is crucial in middleware:

1. Always propagate errors from the inner service
2. Use appropriate error types
3. Handle initialization errors
4. Consider using custom error types for specific middleware errors
5. Document error conditions and handling

## When To (Not) Use Middleware

Use middleware when you need to:

- Add cross-cutting concerns
- Modify requests/responses globally
- Add authentication/authorization
- Add logging or monitoring
- Handle compression or caching

Avoid middleware when:

- The functionality is specific to a single route
- The operation is better handled by a service
- The overhead would be too high
- The functionality can be implemented more simply

## Author's References

- `EitherBody` + when is middleware appropriate: https://discord.com/channels/771444961383153695/952016890723729428
- Actix Web Documentation: https://docs.rs/actix-web
- Service Trait Documentation: https://docs.rs/actix-service
- MessageBody Trait Documentation: https://docs.rs/actix-web/latest/actix_web/body/trait.MessageBody.html
