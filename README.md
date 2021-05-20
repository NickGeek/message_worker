# Message Worker (`message_worker`)
[![crates.io link](https://img.shields.io/crates/v/message_worker.svg)](https://crates.io/crates/message_worker)
[![crates.io link](https://docs.rs/message_worker/badge.svg)](https://docs.rs/message_worker)

Message Worker is a library for Rust for the creation of event-listeners using futures and streams.
Notably, Message Worker supports non-sync and non-send (i.e. non-thread-safe) contexts within listeners.

See the [documentation](https://docs.rs/message_worker) for more information.
