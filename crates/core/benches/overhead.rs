use std::time::Duration;

use chatjimmy::{ChatRequest, Client, Message};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use tokio::net::TcpListener;

#[path = "../examples/mock_server.rs"]
mod mock;

fn bench(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let client = rt.block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(mock::serve(listener, Duration::from_millis(5)));
        Client::builder().base_url(&url).build().unwrap()
    });

    let mut group = c.benchmark_group("chat_many");
    group.sample_size(10).measurement_time(Duration::from_secs(3));
    for n in [1usize, 16, 128] {
        for conc in [1usize, 8, 32] {
            group.bench_with_input(BenchmarkId::new(format!("n{n}"), conc), &(n, conc), |b, &(n, conc)| {
                b.iter(|| {
                    let reqs: Vec<_> = (0..n).map(|_| ChatRequest::new([Message::user("hi")])).collect();
                    let out = rt.block_on(client.chat_many(reqs, conc));
                    assert!(out.iter().all(|r| r.is_ok()));
                })
            });
        }
    }
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
