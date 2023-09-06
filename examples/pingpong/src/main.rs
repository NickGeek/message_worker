use anyhow::{anyhow, bail, Result};
use message_worker::non_blocking::{listen, listen_with_error_handler};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

#[derive(Clone)]
#[repr(transparent)]
struct ActorCtx { output: tokio::sync::broadcast::Sender<Message> }

// Create our messages
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum Message { Ping, Pong }

// Create the ping actor
async fn ping_actor(ctx: ActorCtx, event: Message) -> Result<Option<ActorCtx>> {
    match event {
        Message::Ping => bail!("I'm meant to be the pinger!"),
        Message::Pong => ctx.output.send(Message::Ping).map_err(|err| anyhow!(err))?
    };
    Ok(None)
}

// Create the pong actor
async fn pong_actor(ctx: ActorCtx, event: Message) -> Result<Option<ActorCtx>> {
    match event {
        Message::Ping => ctx.output.send(Message::Pong).map_err(|err| anyhow!(err))?,
        Message::Pong => bail!("I'm meant to be the ponger!")
    };
    Ok(None)
}

async fn error_handler(_ctx: ActorCtx, error: Box<dyn std::error::Error + Send + Sync>) -> bool {
    eprintln!("There was an error sending an item: {:?}", error);
    true
}

async fn printer(ctx: im::Vector<u8>, msg: Message) -> Result<Option<im::Vector<u8>>> {
    let mut buf = ctx;
    let msg = match msg {
        Message::Ping => "ping!\n",
        Message::Pong => "pong!\n"
    };

    for char in msg.as_bytes() {
        buf.push_back(*char);
    }

    const CAPACITY: usize = 6 * 10666; // Close-ish to 64KB
    if buf.len() == CAPACITY {
        let mut str = Vec::<u8>::with_capacity(CAPACITY);
        for byte in buf.iter() {
            str.push(*byte)
        }

        // Safety: We know these are all valid utf-8 chars because we converted them from utf-8 chars
        // earlier.
        unsafe { print!("{}", std::str::from_utf8_unchecked(str.as_slice())); }
        buf.clear();
    }
    Ok(Some(buf))
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
        move || ActorCtx { output: tx_pong },
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
        move || ActorCtx { output: tx_ping },
        pong_actor,
        error_handler
    );

    // Start the printer so we can see the chatter
    listen(print_stream, || im::Vector::<u8>::new(), printer).await.unwrap();
}
