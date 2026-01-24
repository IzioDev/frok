use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use frok_bench::{Ceremony, CeremonyConfig, LoadSpec};

const DEFAULT_CONCURRENCY: usize = 32;
const REQUESTS_PER_ITER: u64 = 200;

fn build_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

fn bench_single_route(
    c: &mut Criterion,
    name: &str,
    config: CeremonyConfig,
    concurrency: usize,
    requests_per_iter: u64,
) {
    let rt = build_runtime();
    let ceremony = rt
        .block_on(Ceremony::start(config))
        .expect("start ceremony");
    let route = ceremony.routes()[0].name.clone();
    let spec = LoadSpec {
        total_requests: requests_per_iter,
        concurrency,
        ..Default::default()
    };

    let mut group = c.benchmark_group(name);
    group.throughput(Throughput::Elements(requests_per_iter));
    group.bench_function("load", |b| {
        let spec = spec.clone();
        b.to_async(&rt).iter(|| {
            let ceremony = ceremony.clone();
            let route = route.clone();
            let spec = spec.clone();
            async move {
                ceremony.load_http(&route, spec).await.expect("load http");
            }
        })
    });
    group.finish();

    rt.block_on(ceremony.shutdown()).expect("shutdown ceremony");
}

fn bench_http1(c: &mut Criterion) {
    bench_single_route(
        c,
        "http1_single",
        CeremonyConfig::single_http1(),
        DEFAULT_CONCURRENCY,
        REQUESTS_PER_ITER,
    );
}

fn bench_http2(c: &mut Criterion) {
    bench_single_route(
        c,
        "http2_single",
        CeremonyConfig::single_http2(),
        DEFAULT_CONCURRENCY,
        REQUESTS_PER_ITER,
    );
}

criterion_group!(benches, bench_http1, bench_http2);
criterion_main!(benches);
