#![deny(missing_docs)]
#![warn(rustdoc::missing_doc_code_examples)]
//! Message Worker is a library for Rust for the creation of event-listeners using futures and
//! streams. Notably, Message Worker supports non-sync and non-send (i.e. non-thread-safe)
//! contexts within listeners.
//!
//! This is a fairly low-level library that can be used to build a wide-array of stream-processing
//! and event-driven systems. It can even be used to build actor systems!
//!
//! This library must be used in a [tokio](https://tokio.rs/) runtime.
//!
//! The tl;dr is that if you want a worker that accepts a stream of messages/events and does
//! something upon receiving each message asynchronously... this is the library for you!
//! The key function here is `message_worker::[non_]blocking::listen(stream, || ctx, handler)`.
//!
//! The first argument is a [Stream](https://docs.rs/futures-core/0.3.15/futures_core/stream/trait.Stream.html).
//! Streams are basically asynchronous iterators and can be made from many different things including
//! mpsc/broadcast channels.
//!
//! The second argument is a closure that creates the "context" for the worker. Essentially, this is
//! any state you want your worker to have access to. With a [non_blocking](non_blocking) worker it's generally best to
//! use immutable datastructures, like those from [im](https://docs.rs/im/15.0.0/im/) if you need to modify the
//! state. With a [blocking](blocking) worker, you can simply wrap your state in a [`RefCell`](https://doc.rust-lang.org/std/cell/struct.RefCell.html).
//!
//! The third argument is the handler, which is where the magic happens. The handler is the name of a function
//! you declare with the signature `fn(ctx: Arc/Rc<Context>, msg: MessageType) -> Result<Option<Context>, Err>`.
//! If an error is returned the error handler for the worker will run. If `Ok(None)` is returned the
//! worker will continue running as-is. If `Ok(context)` is returned the worker will continue running
//! but the next time it runs it will use the new context in that return value.
//!
//! # Examples
//! ## Printer
//! ```
//! use message_worker::non_blocking::listen;
//! use message_worker::{empty_ctx, EmptyCtx};
//! use std::sync::Arc;
//! use anyhow::Result;
//!
//! let mut rt = tokio::runtime::Runtime::new().unwrap();
//! rt.block_on(async {
//!     // Create our stream
//!     let source = tokio_stream::iter(vec![42, 0xff6900, 1337]);
//!
//!     // Create a listener that prints out each item in the stream
//!     async fn on_item(_ctx: Arc<EmptyCtx>, event: usize) -> Result<Option<EmptyCtx>> {
//!         eprintln!("{}", event);
//!         Ok(None)
//!     }
//!
//!     // Start listening
//!     listen(source, empty_ctx, on_item).await.unwrap();
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
//! use std::sync::Arc;
//! use anyhow::Result;
//! use tokio_stream::StreamExt;
//! use tokio_stream::wrappers::ReceiverStream;
//!
//! let mut rt = tokio::runtime::Runtime::new().unwrap();
//! rt.block_on(async {
//!     struct BiCtx { output: tokio::sync::mpsc::Sender<usize> }
//!
//!     // Create our stream
//!     let source = tokio_stream::iter(vec![42, 0xff6900, 1337]);
//!
//!     // Create a listener that outputs each item in the stream multiplied by two
//!     async fn on_item(ctx: Arc<BiCtx>, event: usize) -> Result<Option<BiCtx>> {
//!         ctx.output.send(event * 2).await?; // Send the output
//!         Ok(None)
//!     }
//!
//!     // Connect the number stream to `on_item`
//!     let (tx, rx) = tokio::sync::mpsc::channel::<usize>(3);
//!     listen(source, move || BiCtx {
//!         output: tx
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
//! ```no_run
//! use message_worker::non_blocking::listen;
//! use std::sync::Arc;
//! use anyhow::{Result, bail, anyhow};
//! use tokio_stream::wrappers::BroadcastStream;
//! use tokio_stream::StreamExt;
//!
//! #[tokio::main]
//! async fn main() {
//!     struct ActorCtx { output: tokio::sync::broadcast::Sender<Message> }
//!
//!     // Create our messages
//!     #[derive(Debug, Copy, Clone, Eq, PartialEq)]
//!     enum Message { Ping, Pong }
//!
//!
//!     // Create the ping actor
//!     async fn ping_actor(ctx: Arc<ActorCtx>, event: Message) -> Result<Option<ActorCtx>> {
//!         match event {
//!             Message::Ping => bail!("I'm meant to be the pinger!"),
//!             Message::Pong => ctx.output.send(Message::Ping).map_err(|err| anyhow!(err))?
//!         };
//!         Ok(None)
//!     }
//!
//!     // Create the pong actor
//!     async fn pong_actor(ctx: Arc<ActorCtx>, event: Message) -> Result<Option<ActorCtx>> {
//!         match event {
//!             Message::Ping => ctx.output.send(Message::Pong).map_err(|err| anyhow!(err))?,
//!             Message::Pong => bail!("I'm meant to be the ponger!")
//!         };
//!         Ok(None)
//!     }
//!
//!     // Create our initial stream
//!     let initial_ping = tokio_stream::iter(vec![Message::Ping]);
//!
//!     // Connect everything together
//!     let (tx_ping, rx_ping) = tokio::sync::broadcast::channel::<Message>(2);
//!     let (tx_pong, rx_pong) = tokio::sync::broadcast::channel::<Message>(2);
//!     let mut watch_pongs = BroadcastStream::new(tx_ping.clone().subscribe())
//!         .filter(|msg| msg.is_ok())
//!         .map(|msg| msg.unwrap());
//!     let mut watch_pings = BroadcastStream::new(tx_pong.clone().subscribe())
//!         .filter(|msg| msg.is_ok())
//!         .map(|msg| msg.unwrap());
//!
//!     // Start the ping actor
//!     listen(
//!         BroadcastStream::new(rx_ping)
//!             .filter(|msg| msg.is_ok())
//!             .map(|msg| msg.unwrap()),
//!         move || ActorCtx { output: tx_pong },
//!         ping_actor
//!     );
//!
//!     // Start the pong actor
//!     listen(
//!         initial_ping.chain(BroadcastStream::new(rx_pong)
//!             .filter(|msg| msg.is_ok())
//!             .map(|msg| msg.unwrap())),
//!         move || ActorCtx { output: tx_ping },
//!         pong_actor
//!     );
//!
//!     assert_eq!(watch_pings.next().await, Some(Message::Ping));
//!     assert_eq!(watch_pongs.next().await, Some(Message::Pong));
//!     assert_eq!(watch_pings.next().await, Some(Message::Ping));
//!     assert_eq!(watch_pongs.next().await, Some(Message::Pong));
//! }
//! ```
//!
//! ## The Wild Example (calling V8's C++ via Deno within an event listener to run JS)
//! ```
//! use message_worker::blocking::listen;
//! use deno_core::{JsRuntime, RuntimeOptions};
//! use std::rc::Rc;
//! use std::cell::RefCell;
//! use anyhow::Result;
//! use tokio_stream::StreamExt;
//! use tokio_stream::wrappers::ReceiverStream;
//!
//! let mut rt = tokio::runtime::Runtime::new().unwrap();
//! rt.block_on(async {
//!     struct Context {
//!         test_res: tokio::sync::mpsc::Sender<()>,
//!         runtime: JsRuntime
//!     }
//!
//!     let (mut tx, rx) = tokio::sync::mpsc::channel::<()>(1);
//!     let stream = ReceiverStream::new(rx);
//!
//!     let (test_res_tx, mut test_res) = {
//!         let (tx, rx) = tokio::sync::mpsc::channel::<()>(1);
//!         (tx, ReceiverStream::new(rx))
//!     };
//!
//!     async fn mock_handle(ctx: Rc<RefCell<Context>>, _event: ()) -> Result<Option<RefCell<Context>>> {
//!         let mut ctx = (&*ctx).borrow_mut();
//!         let runtime = &mut ctx.runtime;
//!
//!         runtime.execute_script_static(
//!             "<test>",
//!             r#"Deno.core.print(`Got a message!\n`);"#
//!         )?;
//!         runtime.run_event_loop(false).await?;
//!
//!         ctx.test_res.send(()).await?;
//!         Ok(None)
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
//!                         ..RuntimeOptions::default()
//!                     });
//!
//!                     runtime.execute_script_static(
//!                         "<test>",
//!                         r#"Deno.core.print(`Starting up the JS runtime via C++ FFI and Deno 🤯\n`);"#
//!                     ).unwrap();
//!                     runtime.run_event_loop(false).await.unwrap();
//!
//!                     runtime
//!                 }).await
//!             })
//!         };
//!
//!         RefCell::new(Context {
//!             test_res: test_res_tx,
//!             runtime
//!         })
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

/// The type of the empty context (unit)
pub type EmptyCtx = ();

/// A predefined context for listeners that don't need any state.
#[inline]
pub const fn empty_ctx() {}

#[cfg(test)]
mod tests {

    #[tokio::test]
    async fn ping_pong() {
        use crate::non_blocking::listen;
        use anyhow::{Result, bail, anyhow};
        use tokio_stream::wrappers::BroadcastStream;
        use tokio_stream::StreamExt;
        use std::sync::Arc;

        struct ActorCtx { output: tokio::sync::broadcast::Sender<Message> }

        // Create our messages
        #[derive(Debug, Copy, Clone, Eq, PartialEq)]
        enum Message { Ping, Pong }


        // Create the ping actor
        async fn ping_actor(ctx: Arc<ActorCtx>, event: Message) -> Result<Option<ActorCtx>> {
            match event {
                Message::Ping => bail!("I'm meant to be the pinger!"),
                Message::Pong => ctx.output.send(Message::Ping).map_err(|err| anyhow!(err))?
            };
            Ok(None)
        }

        // Create the pong actor
        async fn pong_actor(ctx: Arc<ActorCtx>, event: Message) -> Result<Option<ActorCtx>> {
            match event {
                Message::Ping => ctx.output.send(Message::Pong).map_err(|err| anyhow!(err))?,
                Message::Pong => bail!("I'm meant to be the ponger!")
            };
            Ok(None)
        }

        // Create our initial stream
        let initial_ping = tokio_stream::iter(vec![Message::Ping]);

        // Connect everything together
        let (tx_ping, rx_ping) = tokio::sync::broadcast::channel::<Message>(2);
        let (tx_pong, rx_pong) = tokio::sync::broadcast::channel::<Message>(2);
        let mut watch_pongs = BroadcastStream::new(tx_ping.clone().subscribe())
            .filter(|msg| msg.is_ok())
            .map(|msg| msg.unwrap());
        let mut watch_pings = BroadcastStream::new(tx_pong.clone().subscribe())
            .filter(|msg| msg.is_ok())
            .map(|msg| msg.unwrap());

        // Start the ping actor
        listen(
            BroadcastStream::new(rx_ping)
                .filter(|msg| msg.is_ok())
                .map(|msg| msg.unwrap()),
            move || ActorCtx { output: tx_pong },
            ping_actor
        );

        // Start the pong actor
        listen(
            initial_ping.chain(BroadcastStream::new(rx_pong)
                .filter(|msg| msg.is_ok())
                .map(|msg| msg.unwrap())),
            move || ActorCtx { output: tx_ping },
            pong_actor
        );

        assert_eq!(watch_pings.next().await, Some(Message::Ping));
        assert_eq!(watch_pongs.next().await, Some(Message::Pong));
        assert_eq!(watch_pings.next().await, Some(Message::Ping));
        assert_eq!(watch_pongs.next().await, Some(Message::Pong));
    }
}
