use criterion::{criterion_group, criterion_main, Criterion};
use tokio_stream::StreamExt;
use message_worker::EmptyCtx;
use anyhow::Result;

use message_worker::{blocking, non_blocking};

pub fn listener_creation(c: &mut Criterion) {
    c.bench_function("create listener", |b| {
        let executor = tokio::runtime::Runtime::new().unwrap();

        async fn handle_message(_ctx: &mut EmptyCtx, _event: ()) -> Result<()> { Ok(()) }

        b.to_async(executor).iter(move || async {
            let source = tokio_stream::iter(vec![()]);
            non_blocking::listen(source, || EmptyCtx, handle_message);
        })
    });
}

pub fn listener_process_one(c: &mut Criterion) {
    c.bench_function("listener process 1 message", |b| {
        let executor = tokio::runtime::Runtime::new().unwrap();

        async fn handle_message(_ctx: &mut EmptyCtx, _event: ()) -> Result<()> { Ok(()) }

        b.to_async(executor).iter(move || async {
            let source = tokio_stream::iter(vec![()]);
            non_blocking::listen(source, || EmptyCtx, handle_message).await.unwrap();
        })
    });
}

pub fn listener_process_many(c: &mut Criterion) {
    c.bench_function("listener process 1000 messages", |b| {
        let executor = tokio::runtime::Runtime::new().unwrap();
        async fn handle_message(_ctx: &mut EmptyCtx, _event: ()) -> Result<()> { Ok(()) }

        b.to_async(executor).iter(move || async {
            let source = futures::stream::repeat(()).take(1000);
            non_blocking::listen(source, || EmptyCtx, handle_message).await.unwrap();
        })
    });
}

pub fn blocking_listener_creation(c: &mut Criterion) {
    c.bench_function("(blocking) create listener", |b| {
        let executor = tokio::runtime::Runtime::new().unwrap();

        async fn handle_message(_ctx: &mut EmptyCtx, _event: ()) -> Result<()> { Ok(()) }

        b.to_async(executor).iter(move || async {
            let source = tokio_stream::iter(vec![()]);
            blocking::listen(source, || EmptyCtx, handle_message);
        })
    });
}

pub fn blocking_listener_process_one(c: &mut Criterion) {
    c.bench_function("(blocking) listener process 1 message", |b| {
        let executor = tokio::runtime::Runtime::new().unwrap();

        async fn handle_message(_ctx: &mut EmptyCtx, _event: ()) -> Result<()> { Ok(()) }

        b.to_async(executor).iter(move || async {
            let source = tokio_stream::iter(vec![()]);
            blocking::listen(source, || EmptyCtx, handle_message).await.unwrap();
        })
    });
}

pub fn blocking_listener_process_many(c: &mut Criterion) {
    c.bench_function("(blocking) listener process 1000 messages", |b| {
        let executor = tokio::runtime::Runtime::new().unwrap();
        async fn handle_message(_ctx: &mut EmptyCtx, _event: ()) -> Result<()> { Ok(()) }

        b.to_async(executor).iter(move || async {
            let source = futures::stream::repeat(()).take(1000);
            blocking::listen(source, || EmptyCtx, handle_message).await.unwrap();
        })
    });
}

criterion_group!(
    benches,
    listener_creation,
    listener_process_one,
    listener_process_many,
    blocking_listener_creation,
    blocking_listener_process_one,
    blocking_listener_process_many
);
criterion_main!(benches);
