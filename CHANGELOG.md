# Changelog
## v0.6
### Breaking
- The context can be anything that implements clone. To get the behaviour from v0.5.x, wrap contexts in `Arc::new`
for `non_blocking` and `Rc::new` for `blocking`.

## v0.5
### Breaking
- Switched back to guarding contexts with `Rc`/`Arc`. Interior mutability is still possible with the power of `RefCell`.
- Removed the `Context` and `ThreadSafeContext` trait. Now all Rust types that meet the lifetime/thread-safety requirements
can be used as contexts without any extra trait implementations.
  - `listen(stream, || EmptyCtx, handler)` can be replaced with either `listen(stream, || (), handler)` or `listen(stream, empty_ctx, handler)`. 
- I moved to an [error-framework agnostic error handling approach](https://github.com/printfn/ees),
  removing the dependency on [anyhow](https://docs.rs/anyhow/1.0.40/anyhow/).

## v0.4
**I pulled this release because [printfn](https://github.com/printfn) found that via thread-local storage UB could be
entered by storing multiple mutable references to the context and also being able to keep the references after dropping
the context.**

### Breaking
- Switched to using plain references over `Rc`/`Arc`

### Features
- Added benchmark tests