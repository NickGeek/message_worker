# Message Worker (`message_worker`)

Message Worker is a library for Rust for the creation of event-listeners using futures and streams.
Notably, Message Worker supports non-sync and non-send (i.e. non-thread-safe) contexts within listeners.
