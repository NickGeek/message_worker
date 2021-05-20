use std::future::Future;
use futures_core::Stream;
use tokio::stream::StreamExt;
use std::fmt::Debug;
use anyhow::{Result, Error};
use std::sync::Arc;

use crate::ThreadSafeContext;

pub fn listen<Ctx, CtxFactory, Source, Event, HandleEventFuture>(
    source: Source,
    context_factory: CtxFactory,
    handle_event: fn(Arc<Ctx>, Event) -> HandleEventFuture
) where
    Ctx: ThreadSafeContext,
    CtxFactory: (FnOnce() -> Ctx) + Send + 'static,
    Source: Stream<Item = Event> + Unpin + Send + 'static,
    Event: Send + Debug + 'static,
    HandleEventFuture: Future<Output = Result<()>> + Send + 'static,
{
    listen_with_error_handler(source, context_factory, handle_event, default_error_handler)
}

pub fn listen_with_error_handler<Ctx, CtxFactory, Source, Event, HandleEventFuture, HandleErrorFuture>(
    mut source: Source,
    context_factory: CtxFactory,
    handle_event: fn(Arc<Ctx>, Event) -> HandleEventFuture,
    handle_error: fn(Arc<Ctx>, Error) -> HandleErrorFuture
) where
    Ctx: ThreadSafeContext,
    CtxFactory: (FnOnce() -> Ctx) + Send + 'static,
    Source: Stream<Item = Event> + Unpin + Send + 'static,
    Event: Send + Debug + 'static,
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
    });
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
            let stream = Box::pin(rx);

            (MockCtx {
                internal_state: EXPECTED,
                test_res: RwLock::new(tx)
            }, stream)
        };

        let (mut tx, rx) = tokio::sync::mpsc::channel::<()>(1);
        let stream = Box::pin(rx);

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
            let stream = Box::pin(rx);

            (MockCtx {
                test_res: RwLock::new(tx)
            }, stream)
        };

        let (mut tx, rx) = tokio::sync::mpsc::channel::<u32>(1);
        let stream = Box::pin(rx);

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
            let stream = Box::pin(rx);

            (MockCtx {
                test_res: RwLock::new(tx)
            }, stream)
        };

        let (mut tx, rx) = tokio::sync::mpsc::channel::<()>(1);
        let stream = Box::pin(rx);

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
            let stream = Box::pin(rx);

            (MockCtx {
                test_res: RwLock::new(tx)
            }, stream)
        };

        let (mut tx, rx) = tokio::sync::mpsc::channel::<Cow<'static, str>>(1);
        let stream = Box::pin(rx);

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
