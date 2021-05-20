use anyhow::{Result, bail, Error, anyhow};
use message_worker::{Context, ThreadSafeContext, EmptyCtx};
use message_worker::non_blocking::{listen, listen_with_error_handler};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

struct ActorCtx { output: RwLock<tokio::sync::broadcast::Sender<Message>> }
impl Context for ActorCtx {} impl ThreadSafeContext for ActorCtx {}

// Create our messages
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum Message { Ping, Pong }

// Create the ping actor
async fn ping_actor(ctx: Arc<ActorCtx>, event: Message) -> Result<()> {
    match event {
        Message::Ping => bail!("I'm meant to be the pinger!"),
        Message::Pong =>
            ctx.output
                .write().await
                .send(Message::Ping)
                .map_err(|err| anyhow!(err))?
    };
    Ok(())
}

// Create the pong actor
async fn pong_actor(ctx: Arc<ActorCtx>, event: Message) -> Result<()> {
    match event {
        Message::Ping =>
            ctx.output
                .write().await
                .send(Message::Pong)
                .map_err(|err| anyhow!(err))?,
        Message::Pong => bail!("I'm meant to be the ponger!")
    };
    Ok(())
}

async fn error_handler(_ctx: Arc<ActorCtx>, error: Error) -> bool {
    eprintln!("There was an error sending an item: {:?}", error);
    true
}

async fn printer(_ctx: Arc<EmptyCtx>, msg: Message) -> Result<()> {
    println!("{:?}", msg);
    Ok(())
}

#[tokio::main]
async fn main() {
    // Create our initial stream
    let initial_ping = tokio_stream::iter(vec![Message::Ping]);

    // Connect everything together
    let (tx_ping, rx_ping) = tokio::sync::broadcast::channel::<Message>(100_000);
    let (tx_pong, rx_pong) = tokio::sync::broadcast::channel::<Message>(100_000);

    // Combined stream of all the pings and pongs
    let print_stream = Box::pin(
        BroadcastStream::new(tx_ping.clone().subscribe())
            .merge(BroadcastStream::new(tx_pong.clone().subscribe()))
            .filter(|msg| msg.is_ok())
            .map(|msg| msg.unwrap())
    );

    // Start the ping actor
    listen_with_error_handler(
        Box::pin(
            BroadcastStream::new(rx_ping)
                .filter(|msg| msg.is_ok())
                .map(|msg| msg.unwrap())
        ),
        move || ActorCtx { output: RwLock::new(tx_pong) },
        ping_actor,
        error_handler
    );

    // Start the pong actor
    listen_with_error_handler(
        initial_ping.chain(Box::pin(
            BroadcastStream::new(rx_pong)
                .filter(|msg| msg.is_ok())
                .map(|msg| msg.unwrap())
        )),
        move || ActorCtx { output: RwLock::new(tx_ping) },
        pong_actor,
        error_handler
    );

    // Start the printer so we can see the chatter
    listen(print_stream, || EmptyCtx, printer).await.unwrap();
}
