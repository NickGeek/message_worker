use std::future::Future;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio_stream::{Stream, StreamExt};

use crate::{ThreadSafeContext, HandlerResult};
use std::cell::RefCell;
use std::error::Error;

/// Creates a listener with the default error handler on its own system thread. It is safe to work
/// with non-sync and non-send data in this listener. The callback (`handle_message`) will be invoked
/// whenever a new item from the `source` stream is emitted. The `context_factory` is a closure you
/// must provide that returns the initial state for the listener.
pub fn listen<Ctx, CtxFactory, Source, Message, HandleMessageFuture, E>(
    source: Source,
    context_factory: CtxFactory,
    handle_message: fn(Ctx, Message) -> HandleMessageFuture
) -> JoinHandle<()> where
    Ctx: ThreadSafeContext,
    CtxFactory: (FnOnce() -> Ctx) + Send + 'static,
    Source: Stream<Item =Message> + Unpin + Send + 'static,
    Message: Send + 'static,
    HandleMessageFuture: Future<Output = HandlerResult<Ctx, E>> + Send + 'static,
    E: Into<anyhow::Error> + Send + 'static
{
    listen_with_error_handler(source, context_factory, handle_message, default_error_handler)
}

/// This is the same as `listen` but it allows a custom error handler to be defined.
/// The error handler callback receives the context of the listener and the error that occurred.
/// The error handler callback returns a boolean declaring if the listener should keep running or not.
pub fn listen_with_error_handler<Ctx, CtxFactory, Source, Message, HandleMessageFuture, HandleErrorFuture, E>(
    mut source: Source,
    context_factory: CtxFactory,
    handle_message: fn(Ctx, Message) -> HandleMessageFuture,
    handle_error: fn(E) -> HandleErrorFuture
) -> JoinHandle<()> where
    Ctx: ThreadSafeContext,
    CtxFactory: (FnOnce() -> Ctx) + Send + 'static,
    Source: Stream<Item = Message> + Unpin + Send + 'static,
    Message: Send + 'static,
    HandleMessageFuture: Future<Output = HandlerResult<Ctx, E>> + Send + 'static,
    HandleErrorFuture: Future<Output = bool> + Send + 'static,
    E: Into<anyhow::Error> + Send + 'static
{
    tokio::spawn(async move {
        let mut context = context_factory();

        while let Some(message) = source.next().await {
            match handle_message(context, message).await.into() {
                Ok(new_ctx) => context = new_ctx,
                Err((new_ctx, err)) => {
                    if !handle_error(err).await {
                        break;
                    }
                    context = new_ctx;
                }
            }
        }
    })
}

async fn default_error_handler<E: Into<anyhow::Error>>(err: E) -> bool {
    eprintln!("There was an error running the message worker: {:?}", err.into());
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, Arc};
    use std::borrow::Cow;
    use anyhow::{bail, anyhow};

    use crate::{Context, HandlerResult, attempt};
    use tokio_stream::StreamExt;
    use tokio_stream::wrappers::ReceiverStream;
    use std::error::Error;
    use tokio::sync::mpsc::error::SendError;

    #[tokio::test]
    async fn should_be_able_read_ctx_from_handler() {
        // Arrange
        const EXPECTED: u32 = 1337;

        struct MockCtx {
            internal_state: u32,
            test_res: tokio::sync::mpsc::Sender<u32>
        }
        impl Context for MockCtx {}
        impl ThreadSafeContext for MockCtx {}

        let (ctx, mut test_res) = {
            let (tx, rx) = tokio::sync::mpsc::channel::<u32>(1);
            let stream = ReceiverStream::new(rx);

            (MockCtx {
                internal_state: EXPECTED,
                test_res: tx
            }, stream)
        };

        let (tx, rx) = tokio::sync::mpsc::channel::<()>(1);
        let stream = ReceiverStream::new(rx);

        async fn mock_handle(ctx: MockCtx, _msg: ()) -> HandlerResult<MockCtx, SendError<u32>> {
            match ctx.test_res.send(ctx.internal_state).await {
                Ok(_) => Ok(ctx),
                Err(err) => Err((ctx, err))
            }.into()
        }

        // Act
        listen(stream, move || ctx, mock_handle);
        tx.send(()).await.unwrap();

        // Assert
        assert_eq!(test_res.next().await, Some(EXPECTED))
    }
/*
    #[tokio::test]
    async fn should_be_able_to_internally_mutate_the_ctx() {
        // Arrange
        const EXPECTED: &str = "foo";

        struct MockCtx {
            internal_state: Arc<Mutex<String>>,
            test_res: tokio::sync::mpsc::Sender<()>
        }
        impl Context for MockCtx {}
        impl ThreadSafeContext for MockCtx {}

        let shared_state = Arc::new(Mutex::new("bar".to_string()));

        let (ctx, mut test_res) = {
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            let stream = ReceiverStream::new(rx);

            (MockCtx {
                internal_state: shared_state.clone(),
                test_res: tx
            }, stream)
        };

        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let stream = ReceiverStream::new(rx);

        async fn mock_handle(ctx: Arc<MockCtx>, _event: ()) -> Result<()> {
            {
                let mut str = ctx.internal_state
                    .lock()
                    .map_err(|err| anyhow!("Locking error: {:?}", err))?;

                *str = EXPECTED.to_string();
            }

            ctx.test_res.send(()).await?;
            Ok(())
        }

        // Act
        listen(stream, move || ctx, mock_handle);
        tx.send(()).await.unwrap();
        test_res.next().await;

        // Assert
        let ctx_internal_state = shared_state.lock().unwrap();
        assert_eq!(ctx_internal_state.as_str(), EXPECTED);
    }

    #[tokio::test]
    async fn should_be_able_to_read_the_event() {
        // Arrange
        const EXPECTED: u32 = 1337;

        struct MockCtx {
            test_res: tokio::sync::mpsc::Sender<u32>
        }
        impl Context for MockCtx {}
        impl ThreadSafeContext for MockCtx {}

        let (ctx, mut test_res) = {
            let (tx, rx) = tokio::sync::mpsc::channel::<u32>(1);
            let stream = ReceiverStream::new(rx);

            (MockCtx {
                test_res: tx
            }, stream)
        };

        let (tx, rx) = tokio::sync::mpsc::channel::<u32>(1);
        let stream = ReceiverStream::new(rx);

        async fn mock_handle(ctx: Arc<MockCtx>, event: u32) -> Result<()> {
            ctx.test_res.send(event).await?;
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
            test_res: tokio::sync::mpsc::Sender<Cow<'static, str>>
        }
        impl Context for MockCtx {}
        impl ThreadSafeContext for MockCtx {}

        let (ctx, mut test_res) = {
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            let stream = ReceiverStream::new(rx);

            (MockCtx {
                test_res: tx
            }, stream)
        };

        let (tx, rx) = tokio::sync::mpsc::channel::<()>(1);
        let stream = ReceiverStream::new(rx);

        async fn mock_handle(_ctx: Arc<MockCtx>, _event: ()) -> Result<()> {
            bail!("rip")
        }

        async fn mock_handle_error(ctx: Arc<MockCtx>, error: Error) -> bool {
            ctx.test_res.send(error.to_string().into()).await.unwrap();
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
            test_res: tokio::sync::mpsc::Sender<Cow<'static, str>>
        }
        impl Context for MockCtx {}
        impl ThreadSafeContext for MockCtx {}

        let (ctx, mut test_res) = {
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            let stream = ReceiverStream::new(rx);

            (MockCtx {
                test_res: tx
            }, stream)
        };

        let (tx, rx) = tokio::sync::mpsc::channel::<Cow<'static, str>>(1);
        let stream = ReceiverStream::new(rx);

        async fn mock_handle(_ctx: Arc<MockCtx>, event: Cow<'static, str>) -> Result<()> {
            bail!(event)
        }

        async fn mock_handle_error(ctx: Arc<MockCtx>, error: Error) -> bool {
            ctx.test_res.send(error.to_string().into()).await.unwrap();
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
 */
}
