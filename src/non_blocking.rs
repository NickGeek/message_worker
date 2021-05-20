use std::future::Future;
use anyhow::{Result, Error};
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio_stream::{Stream, StreamExt};

use crate::ThreadSafeContext;

/// Creates a listener with the default error handler on its own system thread. It is safe to work
/// with non-sync and non-send data in this listener. The callback (`handle_event`) will be invoked
/// whenever a new item from the `source` stream is emitted. The `context_factory` is a closure you
/// must provide that returns the initial state for the listener.
pub fn listen<Ctx, CtxFactory, Source, Event, HandleEventFuture>(
    source: Source,
    context_factory: CtxFactory,
    handle_event: fn(Arc<Ctx>, Event) -> HandleEventFuture
) -> JoinHandle<()> where
    Ctx: ThreadSafeContext,
    CtxFactory: (FnOnce() -> Ctx) + Send + 'static,
    Source: Stream<Item = Event> + Unpin + Send + 'static,
    Event: Send + 'static,
    HandleEventFuture: Future<Output = Result<()>> + Send + 'static,
{
    listen_with_error_handler(source, context_factory, handle_event, default_error_handler)
}

/// This is the same as `listen` but it allows a custom error handler to be defined.
/// The error handler callback receives the context of the listener and the error that occurred.
/// The error handler callback returns a boolean declaring if the listener should keep running or not.
pub fn listen_with_error_handler<Ctx, CtxFactory, Source, Event, HandleEventFuture, HandleErrorFuture>(
    mut source: Source,
    context_factory: CtxFactory,
    handle_event: fn(Arc<Ctx>, Event) -> HandleEventFuture,
    handle_error: fn(Arc<Ctx>, Error) -> HandleErrorFuture
) -> JoinHandle<()> where
    Ctx: ThreadSafeContext,
    CtxFactory: (FnOnce() -> Ctx) + Send + 'static,
    Source: Stream<Item = Event> + Unpin + Send + 'static,
    Event: Send + 'static,
    HandleEventFuture: Future<Output = Result<()>> + Send + 'static,
    HandleErrorFuture: Future<Output = bool> + Send + 'static
{
    tokio::spawn(async move {
        let context = Arc::new(context_factory());
        while let Some(event) = source.next().await {
            if let Err(err) = handle_event(context.clone(), event).await {
                if !handle_error(context.clone(), err).await {
                    break;
                }
            }
        }
    })
}

async fn default_error_handler<C: ThreadSafeContext>(_ctx: Arc<C>, err: Error) -> bool {
    eprintln!("There was an error running the message worker: {:?}", err);
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::RwLock;
    use std::borrow::Cow;
    use anyhow::bail;

    use crate::Context;
    use tokio_stream::StreamExt;
    use tokio_stream::wrappers::ReceiverStream;

    #[tokio::test]
    async fn should_be_able_read_ctx_from_handler() {
        // Arrange
        const EXPECTED: u32 = 1337;

        struct MockCtx {
            internal_state: u32,
            test_res: RwLock<tokio::sync::mpsc::Sender<u32>>
        }
        impl Context for MockCtx {}
        impl ThreadSafeContext for MockCtx {}

        let (ctx, mut test_res) = {
            let (tx, rx) = tokio::sync::mpsc::channel::<u32>(1);
            let stream = ReceiverStream::new(rx);

            (MockCtx {
                internal_state: EXPECTED,
                test_res: RwLock::new(tx)
            }, stream)
        };

        let (tx, rx) = tokio::sync::mpsc::channel::<()>(1);
        let stream = ReceiverStream::new(rx);

        async fn mock_handle<'a>(ctx: Arc<MockCtx>, _event: ()) -> Result<()> {
            ctx.test_res.write().await.send(ctx.internal_state).await?;
            Ok(())
        }

        // Act
        listen(stream, move || ctx, mock_handle);
        tx.send(()).await.unwrap();

        // Assert
        assert_eq!(test_res.next().await, Some(EXPECTED))
    }

    #[tokio::test]
    async fn should_be_able_to_read_the_event() {
        // Arrange
        const EXPECTED: u32 = 1337;

        struct MockCtx {
            test_res: RwLock<tokio::sync::mpsc::Sender<u32>>
        }
        impl Context for MockCtx {}
        impl ThreadSafeContext for MockCtx {}

        let (ctx, mut test_res) = {
            let (tx, rx) = tokio::sync::mpsc::channel::<u32>(1);
            let stream = ReceiverStream::new(rx);

            (MockCtx {
                test_res: RwLock::new(tx)
            }, stream)
        };

        let (tx, rx) = tokio::sync::mpsc::channel::<u32>(1);
        let stream = ReceiverStream::new(rx);

        async fn mock_handle<'a>(ctx: Arc<MockCtx>, event: u32) -> Result<()> {
            ctx.test_res.write().await.send(event).await?;
            Ok(())
        }

        // Act
        listen(stream, move || ctx, mock_handle);
        tx.send(EXPECTED).await.unwrap();

        // Assert
        assert_eq!(test_res.next().await, Some(EXPECTED))
    }

    #[tokio::test]
    async fn should_handle_errors_with_the_callback() {
        // Arrange
        const EXPECTED: &str = "rip";

        struct MockCtx {
            test_res: RwLock<tokio::sync::mpsc::Sender<Cow<'static, str>>>
        }
        impl Context for MockCtx {}
        impl ThreadSafeContext for MockCtx {}

        let (ctx, mut test_res) = {
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            let stream = ReceiverStream::new(rx);

            (MockCtx {
                test_res: RwLock::new(tx)
            }, stream)
        };

        let (tx, rx) = tokio::sync::mpsc::channel::<()>(1);
        let stream = ReceiverStream::new(rx);

        async fn mock_handle<'a>(_ctx: Arc<MockCtx>, _event: ()) -> Result<()> {
            bail!("rip")
        }

        async fn mock_handle_error<'a>(ctx: Arc<MockCtx>, error: Error) -> bool {
            ctx.test_res.write().await.send(error.to_string().into()).await.unwrap();
            false
        }

        // Act
        listen_with_error_handler(stream, move || ctx, mock_handle, mock_handle_error);
        tx.send(()).await.unwrap();

        // Assert
        assert_eq!(test_res.next().await.unwrap(), Cow::Borrowed(EXPECTED))
    }

    #[tokio::test]
    async fn should_keep_processing_events_if_the_error_handler_returns_true() {
        // Arrange
        const EXPECTED1: &str = "rip";
        const EXPECTED2: &str = "oh no";

        struct MockCtx {
            test_res: RwLock<tokio::sync::mpsc::Sender<Cow<'static, str>>>
        }
        impl Context for MockCtx {}
        impl ThreadSafeContext for MockCtx {}

        let (ctx, mut test_res) = {
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            let stream = ReceiverStream::new(rx);

            (MockCtx {
                test_res: RwLock::new(tx)
            }, stream)
        };

        let (tx, rx) = tokio::sync::mpsc::channel::<Cow<'static, str>>(1);
        let stream = ReceiverStream::new(rx);

        async fn mock_handle<'a>(_ctx: Arc<MockCtx>, event: Cow<'static, str>) -> Result<()> {
            bail!(event)
        }

        async fn mock_handle_error<'a>(ctx: Arc<MockCtx>, error: Error) -> bool {
            ctx.test_res.write().await.send(error.to_string().into()).await.unwrap();
            true
        }

        // Act
        listen_with_error_handler(stream, move || ctx, mock_handle, mock_handle_error);
        tx.send(EXPECTED1.into()).await.unwrap();
        tx.send(EXPECTED2.into()).await.unwrap();

        // Assert
        assert_eq!(test_res.next().await.unwrap(), Cow::Borrowed(EXPECTED1));
        assert_eq!(test_res.next().await.unwrap(), Cow::Borrowed(EXPECTED2));
    }
}
