#![deny(missing_docs)]
#![warn(missing_doc_code_examples)]

//! Message Worker is a library for Rust for the creation of event-listeners using futures and
//! streams. Notably, Message Worker supports non-sync and non-send (i.e. non-thread-safe)
//! contexts within listeners.
//!
//! This is a fairly low-level library that can be used to build a wide-array of stream-processing
//! and event-driven systems. It can even be used to build actor systems!
//!
//! This library must be used in a [tokio](https://tokio.rs/) runtime.
//!
//! # Examples
//! ## Printer
//! ```
//! use message_worker::non_blocking::listen;
//! use message_worker::{Context, ThreadSafeContext};
//! use std::sync::Arc;
//! use anyhow::Result;
//!
//! let mut rt = tokio::runtime::Runtime::new().unwrap();
//! rt.block_on(async {
//!     // We don't need any state for this example
//!     struct EmptyCtx;
//!     impl Context for EmptyCtx {} impl ThreadSafeContext for EmptyCtx {}
//!
//!     // Create our stream
//!     let source = tokio_stream::iter(vec![42, 0xff6900, 1337]);
//!
//!     // Create a listener that prints out each item in the stream
//!     async fn on_item(_ctx: Arc<EmptyCtx>, event: usize) -> Result<()> {
//!         eprintln!("{}", event);
//!         Ok(())
//!     }
//!
//!     // Start listening
//!     listen(source, move || EmptyCtx, on_item).await.unwrap();
//!
//!     /* Prints:
//!        42
//!        0xff6900
//!        1337
//!     */
//! })
//! ```
//!
//! ## Two-way communication
//! ```
//! use message_worker::non_blocking::listen;
//! use message_worker::{Context, ThreadSafeContext};
//! use std::sync::Arc;
//! use anyhow::Result;
//! use tokio::sync::RwLock;
//! use tokio_stream::StreamExt;
//! use tokio_stream::wrappers::ReceiverStream;
//!
//! let mut rt = tokio::runtime::Runtime::new().unwrap();
//! rt.block_on(async {
//!     struct BiCtx { output: RwLock<tokio::sync::mpsc::Sender<usize>> }
//!     impl Context for BiCtx {} impl ThreadSafeContext for BiCtx {}
//!
//!     // Create our stream
//!     let source = tokio_stream::iter(vec![42, 0xff6900, 1337]);
//!
//!     // Create a listener that outputs each item in the stream multiplied by two
//!     async fn on_item(ctx: Arc<BiCtx>, event: usize) -> Result<()> {
//!         let mut output = ctx.output.write().await;
//!         output.send(event * 2).await?; // Send the output
//!         Ok(())
//!     }
//!
//!     // Connect the number stream to `on_item`
//!     let (tx, rx) = tokio::sync::mpsc::channel::<usize>(3);
//!     listen(source, move || BiCtx {
//!         output: RwLock::new(tx)
//!     }, on_item);
//!
//!     let mut  rx = ReceiverStream::new(rx);
//!     assert_eq!(rx.next().await, Some(84));
//!     assert_eq!(rx.next().await, Some(0x1fed200));
//!     assert_eq!(rx.next().await, Some(2674));
//! })
//! ```
//!
//! ## Ping-pong (Actors)
//! ```
//! use message_worker::non_blocking::listen;
//! use message_worker::{Context, ThreadSafeContext};
//! use std::sync::Arc;
//! use anyhow::{Result, bail, anyhow};
//! use tokio::sync::RwLock;
//! use tokio_stream::wrappers::BroadcastStream;
//! use tokio_stream::StreamExt;
//!
//! let mut rt = tokio::runtime::Runtime::new().unwrap();
//! rt.block_on(async {
//!     struct ActorCtx { output: RwLock<tokio::sync::broadcast::Sender<Message>> }
//!     impl Context for ActorCtx {} impl ThreadSafeContext for ActorCtx {}
//!
//!     // Create our messages
//!     #[derive(Debug, Copy, Clone, Eq, PartialEq)]
//!     enum Message { Ping, Pong }
//!
//!
//!     // Create the ping actor
//!     async fn ping_actor(ctx: Arc<ActorCtx>, event: Message) -> Result<()> {
//!         match event {
//!             Message::Ping => bail!("I'm meant to be the pinger!"),
//!             Message::Pong =>
//!                 ctx.output
//!                     .write().await
//!                     .send(Message::Ping)
//!                     .map_err(|err| anyhow!(err))?
//!         };
//!         Ok(())
//!     }
//!
//!     // Create the pong actor
//!     async fn pong_actor(ctx: Arc<ActorCtx>, event: Message) -> Result<()> {
//!         match event {
//!             Message::Ping =>
//!                 ctx.output
//!                     .write().await
//!                     .send(Message::Pong)
//!                     .map_err(|err| anyhow!(err))?,
//!             Message::Pong => bail!("I'm meant to be the ponger!")
//!         };
//!         Ok(())
//!     }
//!
//!     // Create our initial stream
//!     let initial_ping = tokio_stream::iter(vec![Message::Ping]);
//!
//!     // Connect everything together
//!     let (tx_ping, mut rx_ping) = tokio::sync::broadcast::channel::<Message>(128);
//!     let (tx_pong, mut rx_pong) = tokio::sync::broadcast::channel::<Message>(128);
//!     let mut watch_pongs = BroadcastStream::new(tx_ping.clone().subscribe()).map(|msg| msg.unwrap());
//!     let mut watch_pings = BroadcastStream::new(tx_pong.clone().subscribe()).map(|msg| msg.unwrap());
//!
//!     // Start the ping actor
//!     listen(
//!         BroadcastStream::new(rx_ping).map(|msg| msg.unwrap()),
//!         move || ActorCtx { output: RwLock::new(tx_pong) },
//!         ping_actor
//!     );
//!
//!     // Start the pong actor
//!     listen(
//!         initial_ping.chain(BroadcastStream::new(rx_pong).map(|msg| msg.unwrap())),
//!         move || ActorCtx { output: RwLock::new(tx_ping) },
//!         pong_actor
//!     );
//!
//!     assert_eq!(watch_pings.next().await, Some(Message::Ping));
//!     assert_eq!(watch_pongs.next().await, Some(Message::Pong));
//!     assert_eq!(watch_pings.next().await, Some(Message::Ping));
//!     assert_eq!(watch_pongs.next().await, Some(Message::Pong));
//! })
//! ```
//!
//! ## The Wild Example (calling V8's C++ via Deno within an event listener to run JS)
//! ```
//! use message_worker::blocking::listen;
//! use message_worker::Context;
//! use std::cell::RefCell;
//! use deno_core::{JsRuntime, RuntimeOptions};
//! use std::rc::Rc;
//! use anyhow::Result;
//! use tokio_stream::StreamExt;
//! use tokio_stream::wrappers::ReceiverStream;
//!
//! let mut rt = tokio::runtime::Runtime::new().unwrap();
//! rt.block_on(async {
//!     struct MockCtx {
//!         test_res: RefCell<tokio::sync::mpsc::Sender<()>>,
//!         runtime: RefCell<JsRuntime>
//!     }
//!     impl Context for MockCtx {}
//!
//!     let (mut tx, rx) = tokio::sync::mpsc::channel::<()>(1);
//!     let stream = ReceiverStream::new(rx);
//!
//!     let (test_res_tx, mut test_res) = {
//!         let (tx, rx) = tokio::sync::mpsc::channel::<()>(1);
//!         (tx, ReceiverStream::new(rx))
//!     };
//!
//!     async fn mock_handle(ctx: Rc<MockCtx>, _event: ()) -> Result<()> {
//!         let mut runtime = ctx.runtime.borrow_mut();
//!
//!         runtime.execute(
//!             "<test>",
//!             r#"Deno.core.print(`Got a message!\n`);"#
//!         )?;
//!         runtime.run_event_loop().await?;
//!
//!         ctx.test_res.borrow_mut().send(()).await?;
//!         Ok(())
//!     }
//!
//!     listen(stream, move || {
//!         let runtime: JsRuntime = {
//!             let tokio_rt = tokio::runtime::Handle::current();
//!             tokio_rt.block_on(async {
//!                 let local = tokio::task::LocalSet::new();
//!                 local.run_until(async {
//!                     let mut runtime = JsRuntime::new(RuntimeOptions {
//!                         module_loader: Some(Rc::new(deno_core::FsModuleLoader)),
//!                         will_snapshot: false,
//!                         ..RuntimeOptions::default()
//!                     });
//!
//!                     runtime.execute(
//!                         "<test>",
//!                         r#"Deno.core.print(`Starting up the JS runtime via C++ FFI and Deno 🤯\n`);"#
//!                     ).unwrap();
//!                     runtime.run_event_loop().await.unwrap();
//!
//!                     runtime
//!                 }).await
//!             })
//!         };
//!
//!         MockCtx {
//!             test_res: RefCell::new(test_res_tx),
//!             runtime: RefCell::new(runtime)
//!         }
//!     }, mock_handle);
//!     tx.send(()).await.unwrap();
//!
//!     /* Prints:
//!        Starting up the JS runtime via C++ FFI and Deno 🤯
//!        Got a message!
//!     */
//!     assert_eq!(test_res.next().await, Some(()));
//! })
//! ```
//!

/// Listeners that perform CPU intensive/blocking tasks or work with non-threadsafe data
pub mod blocking;
/// Listeners that don't block and work with threadsafe (`Sync` + `Send`) data.
pub mod non_blocking;

/// This trait needs to be implemented by the item you're using as the state for the listener.
///
/// For example, if the listener has no state you can simply go:
/// ```
/// use message_worker::Context;
/// struct EmptyCtx;
/// impl Context for EmptyCtx {}
/// ```
///
/// `EmptyCtx` can now be used as the context for your listeners. For mutability inside a `Context`
/// you can use [RefCell](https://doc.rust-lang.org/std/cell/struct.RefCell.html)s. The Message Worker
/// runtime guarantees that it will never attempt to access your context in parallel from one listener.
pub trait Context: 'static {}

/// If you are using a [`non_blocking`](non_blocking) listener, this implementation is required
/// alongside `Context`.
///
/// For mutability inside a `Context`
/// you can use [Mutex](https://doc.rust-lang.org/std/sync/struct.Mutex.html)es
/// or [RwLock](https://docs.rs/tokio/0.2/tokio/sync/struct.RwLock.html)s.
/// The Message Worker runtime guarantees that it will never attempt to access your context
/// in parallel from one listener.
pub trait ThreadSafeContext: Context + Send + Sync {}

/// A predefined context for listeners that don't need any state.
pub struct EmptyCtx;
impl Context for EmptyCtx {}
impl ThreadSafeContext for EmptyCtx {}

#[cfg(test)]
mod tests {}
