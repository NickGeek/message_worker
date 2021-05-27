use std::future::Future;
use anyhow::{Result, Error};
use std::rc::Rc;
use tokio::task::JoinHandle;
use tokio_stream::{Stream, StreamExt};

use crate::Context;

/// Creates a listener with the default error handler on its own system thread. It is safe to work
/// with non-sync and non-send data in this listener. The callback (`handle_event`) will be invoked
/// whenever a new item from the `source` stream is emitted. The `context_factory` is a closure you
/// must provide that returns the initial state for the listener.
///
/// # Example
/// ```
/// use message_worker::blocking::listen;
/// use message_worker::Context;
/// use tokio::sync::RwLock;
/// use std::rc::Rc;
/// use anyhow::Result;
/// use tokio_stream::StreamExt;
/// use tokio_stream::wrappers::ReceiverStream;
///
/// # let mut rt = tokio::runtime::Runtime::new().unwrap();
/// # rt.block_on(async {
/// // Arrange
/// const EXPECTED: &str = "foo";
///
/// struct MockCtx {
///     // `Rc` is not threadsafe but we can safely use it here
///     internal_state: std::rc::Rc<String>,
///     test_res: tokio::sync::mpsc::Sender<String>
/// }
/// impl Context for MockCtx {}
///
/// let (mut tx, rx) = tokio::sync::mpsc::channel::<()>(1);
/// let stream = ReceiverStream::new(rx);
///
/// let (test_res_tx, mut test_res) = {
///     let (tx, rx) = tokio::sync::mpsc::channel::<String>(1);
///     (tx, ReceiverStream::new(rx))
/// };
///
/// async fn mock_handle(ctx: Rc<MockCtx>, _event: ()) -> Result<()> {
///     // Accessing from an `Rc`
///     let str = (&*ctx.internal_state).clone();
///
///     ctx.test_res.send(str).await?;
///     Ok(())
/// }
///
/// // Act
/// listen(stream, move || MockCtx {
///     internal_state: std::rc::Rc::new(EXPECTED.to_string()),
///     test_res: test_res_tx
/// }, mock_handle);
/// tx.send(()).await.unwrap();
///
/// // Assert
/// assert_eq!(test_res.next().await, Some(EXPECTED.to_string()))
/// # })
/// ```
pub fn listen<Ctx, CtxFactory, Source, Message, HandleEventFuture>(
    source: Source,
    context_factory: CtxFactory,
    handle_message: fn(Rc<Ctx>, Message) -> HandleEventFuture
) -> JoinHandle<()> where
    Ctx: Context,
    CtxFactory: (FnOnce() -> Ctx) + Send + 'static,
    Source: Stream<Item = Message> + Unpin + Send + 'static,
    Message: 'static,
    HandleEventFuture: Future<Output = Result<()>> + 'static,
{
    listen_with_error_handler(source, context_factory, handle_message, default_error_handler)
}

/// This is the same as `listen` but it allows a custom error handler to be defined.
/// The error handler callback receives the context of the listener and the error that occurred.
/// The error handler callback returns a boolean declaring if the listener should keep running or not.
///
/// # Example
/// ```
/// use message_worker::blocking::listen_with_error_handler;
/// use message_worker::Context;
/// use std::borrow::Cow;
/// use std::rc::Rc;
/// use anyhow::{Error, Result, bail};
/// use tokio_stream::StreamExt;
/// use tokio_stream::wrappers::ReceiverStream;
///
/// # #[tokio::main]
/// # async fn main() {
/// // Arrange
/// const EXPECTED1: &str = "rip";
/// const EXPECTED2: &str = "oh no";
///
/// struct MockCtx {
///     test_res: tokio::sync::mpsc::Sender<Cow<'static, str>>
/// }
/// impl Context for MockCtx {}
///
/// let (ctx, mut test_res) = {
///     let (tx, rx) = tokio::sync::mpsc::channel(1);
///     let stream = ReceiverStream::new(rx);
///
///     (MockCtx {
///         test_res: tx
///     }, stream)
/// };
///
/// let (mut tx, rx) = tokio::sync::mpsc::channel::<Cow<'static, str>>(1);
/// let stream = ReceiverStream::new(rx);
///
/// async fn mock_handle(_ctx: Rc<MockCtx>, event: Cow<'static, str>) -> Result<()> {
///     bail!(event)
/// }
///
/// async fn mock_handle_error(ctx: Rc<MockCtx>, error: Error) -> bool {
///     ctx.test_res.send(error.to_string().into()).await.unwrap();
///     true
/// }
///
/// // Act
/// listen_with_error_handler(stream, move || ctx, mock_handle, mock_handle_error);
/// tx.send(EXPECTED1.into()).await.unwrap();
/// tx.send(EXPECTED2.into()).await.unwrap();
///
/// // Assert
/// assert_eq!(test_res.next().await.unwrap(), Cow::Borrowed(EXPECTED1));
/// // This keeps going because the custom error handler returned `true`
/// assert_eq!(test_res.next().await.unwrap(), Cow::Borrowed(EXPECTED2));
/// # }
/// ```
pub fn listen_with_error_handler<Ctx, CtxFactory, Source, Message, HandleEventFuture, HandleErrorFuture>(
    mut source: Source,
    context_factory: CtxFactory,
    handle_message: fn(Rc<Ctx>, Message) -> HandleEventFuture,
    handle_error: fn(Rc<Ctx>, Error) -> HandleErrorFuture
) -> JoinHandle<()> where
    Ctx: Context,
    CtxFactory: (FnOnce() -> Ctx) + Send + 'static,
    Source: Stream<Item = Message> + Unpin + Send + 'static,
    Message: 'static,
    HandleEventFuture: Future<Output = Result<()>> + 'static,
    HandleErrorFuture: Future<Output = bool> + 'static
{
    tokio::task::spawn_blocking(move || {
        let context = Rc::new(context_factory());

        let tokio_rt = tokio::runtime::Handle::current();
        tokio_rt.block_on(async {
            while let Some(message) = source.next().await {
                if let Err(err) = handle_message(context.clone(), message).await {
                    if !handle_error(context.clone(), err).await {
                        break;
                    }
                }
            }
        })
    })
}

async fn default_error_handler<Ctx: Context>(_ctx: Rc<Ctx>, err: Error) -> bool {
    eprintln!("There was an error running the message worker: {:?}", err);
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;
    use anyhow::{bail, anyhow};
    use tokio_stream::wrappers::ReceiverStream;

    #[tokio::test]
    async fn should_be_able_read_ctx_from_handler() {
        // Arrange
        const EXPECTED: u32 = 1337;

        struct MockCtx {
            internal_state: u32,
            test_res: tokio::sync::mpsc::Sender<u32>
        }
        impl Context for MockCtx {}

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

        async fn mock_handle(ctx: Rc<MockCtx>, _event: ()) -> Result<()> {
            ctx.test_res.send(ctx.internal_state).await?;
            Ok(())
        }

        // Act
        listen(stream, move || ctx, mock_handle);
        tx.send(()).await.unwrap();

        // Assert
        assert_eq!(test_res.next().await, Some(EXPECTED))
    }

    #[tokio::test]
    async fn should_be_able_to_use_non_thread_safe_ctx() {
        // Arrange
        const EXPECTED: &str = "foo";

        struct MockCtx {
            internal_state: std::rc::Rc<String>,
            test_res: tokio::sync::mpsc::Sender<String>
        }
        impl Context for MockCtx {}

        let (tx, rx) = tokio::sync::mpsc::channel::<()>(1);
        let stream = ReceiverStream::new(rx);

        let (test_res_tx, mut test_res) = {
            let (tx, rx) = tokio::sync::mpsc::channel::<String>(1);
            (tx, ReceiverStream::new(rx))
        };

        async fn mock_handle(ctx: Rc<MockCtx>, _event: ()) -> Result<()> {
            let str = (&*ctx.internal_state).clone();
            ctx.test_res.send(str).await?;
            Ok(())
        }

        // Act
        listen(stream, move || MockCtx {
            internal_state: std::rc::Rc::new(EXPECTED.to_string()),
            test_res: test_res_tx
        }, mock_handle);
        tx.send(()).await.unwrap();

        // Assert
        assert_eq!(test_res.next().await, Some(EXPECTED.to_string()))
    }

    #[tokio::test]
    async fn should_be_able_to_read_the_event() {
        // Arrange
        const EXPECTED: u32 = 1337;

        struct MockCtx {
            test_res: tokio::sync::mpsc::Sender<u32>
        }
        impl Context for MockCtx {}

        let (ctx, mut test_res) = {
            let (tx, rx) = tokio::sync::mpsc::channel::<u32>(1);
            let stream = ReceiverStream::new(rx);

            (MockCtx {
                test_res: tx
            }, stream)
        };

        let (tx, rx) = tokio::sync::mpsc::channel::<u32>(1);
        let stream = ReceiverStream::new(rx);

        async fn mock_handle(ctx: Rc<MockCtx>, event: u32) -> Result<()> {
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

        let (ctx, mut test_res) = {
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            let stream = ReceiverStream::new(rx);

            (MockCtx {
                test_res: tx
            }, stream)
        };

        let (tx, rx) = tokio::sync::mpsc::channel::<()>(1);
        let stream = ReceiverStream::new(rx);

        async fn mock_handle(_ctx: Rc<MockCtx>, _event: ()) -> Result<()> {
            bail!("rip")
        }

        async fn mock_handle_error(ctx: Rc<MockCtx>, error: Error) -> bool {
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

        let (ctx, mut test_res) = {
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            let stream = ReceiverStream::new(rx);

            (MockCtx {
                test_res: tx
            }, stream)
        };

        let (tx, rx) = tokio::sync::mpsc::channel::<Cow<'static, str>>(1);
        let stream = ReceiverStream::new(rx);

        async fn mock_handle(_ctx: Rc<MockCtx>, event: Cow<'static, str>) -> Result<()> {
            bail!(event)
        }

        async fn mock_handle_error(ctx: Rc<MockCtx>, error: Error) -> bool {
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

    mod deno {
        use super::*;
        use deno_core::{JsRuntime, RuntimeOptions};
        use std::cell::RefCell;

        #[tokio::test]
        async fn should_be_able_to_create_a_deno_runtime() {
            // Arrange
            struct MockCtx {
                test_res: tokio::sync::mpsc::Sender<Vec<u8>>
            }
            impl Context for MockCtx {}

            let (ctx, mut test_res) = {
                let (tx, rx) = tokio::sync::mpsc::channel(1);
                let stream = ReceiverStream::new(rx);

                (MockCtx {
                    test_res: tx
                }, stream)
            };

            let (tx, rx) = tokio::sync::mpsc::channel::<()>(1);
            let stream = ReceiverStream::new(rx);

            async fn mock_handle(ctx: Rc<MockCtx>, _event: ()) -> Result<()> {
                let mut failure_msg: Option<String> = None;

                /*
                 * Important but INSANE note: V8 will crash the whole process if we
                 * drop the snapshot creator before taking a snapshot, so uhh,
                 * no early exits plz
                 */
                let mut runtime = JsRuntime::new(RuntimeOptions {
                    module_loader: Some(Rc::new(deno_core::FsModuleLoader)),
                    will_snapshot: true,
                    ..RuntimeOptions::default()
                });

                if let Err(e) = runtime.execute("<test>", r#"const a = 1 + 1;"#) {
                    failure_msg = Some(match failure_msg {
                        Some(msg) => format!("{}\n{}", msg, e),
                        None => format!("{}", e)
                    });
                }

                if let Err(e) = runtime.run_event_loop().await {
                    failure_msg = Some(match failure_msg {
                        Some(msg) => format!("{}\n{}", msg, e),
                        None => format!("{}", e)
                    });
                }

                let snapshot = Vec::from(runtime.snapshot().as_ref());

                let res: Result<()> = match failure_msg {
                    Some(msg) => Err(anyhow!("{}", msg)),
                    None => Ok(())
                };

                ctx.test_res.send(snapshot).await?;
                res
            }

            // Act
            listen(stream, move || ctx, mock_handle);
            tx.send(()).await.unwrap();

            // Assert
            assert!(match test_res.next().await {
                None => false,
                Some(snapshot) => !snapshot.is_empty()
            })
        }

        #[tokio::test]
        async fn should_be_able_to_store_the_runtime_on_the_ctx() {
            // Arrange
            struct MockCtx {
                test_res: tokio::sync::mpsc::Sender<()>,
                runtime: RefCell<JsRuntime>
            }
            impl Context for MockCtx {}

            let (tx, rx) = tokio::sync::mpsc::channel::<()>(10);
            let stream = ReceiverStream::new(rx);

            let (test_res_tx, mut test_res) = {
                let (tx, rx) = tokio::sync::mpsc::channel::<()>(10);
                (tx, ReceiverStream::new(rx))
            };

            async fn mock_handle(ctx: Rc<MockCtx>, _event: ()) -> Result<()> {
                let mut runtime = ctx.runtime.borrow_mut();

                runtime.execute(
                    "<test>",
                    r#"Deno.core.print(`[should_be_able_to_store_the_runtime_on_the_ctx] Got event: ${++a}\n`);"#
                )?;
                runtime.run_event_loop().await?;

                ctx.test_res.send(()).await?;
                Ok(())
            }

            // Act
            listen(stream, move || {
                let runtime: JsRuntime = {
                    let tokio_rt = tokio::runtime::Handle::current();
                    tokio_rt.block_on(async {
                        let local = tokio::task::LocalSet::new();
                        local.run_until(async {
                            let mut runtime = JsRuntime::new(RuntimeOptions {
                                module_loader: Some(Rc::new(deno_core::FsModuleLoader)),
                                will_snapshot: false,
                                ..RuntimeOptions::default()
                            });

                            runtime.execute(
                                "<test>",
                                r#"
                                    Deno.core.print(`[should_be_able_to_store_the_runtime_on_the_ctx] Creating runtime\n`);
                                    let a = 0;
                                "#
                            ).unwrap();
                            runtime.run_event_loop().await.unwrap();

                            runtime
                        }).await
                    })
                };

                MockCtx {
                    test_res: test_res_tx,
                    runtime: RefCell::new(runtime)
                }
            }, mock_handle);

            const COUNT: u32 = 10;
            for _i in 0..COUNT {
                tx.send(()).await.unwrap();
            }

            // Assert
            for _i in 0..COUNT {
                assert_eq!(test_res.next().await, Some(()));
            }
        }
    }
}
