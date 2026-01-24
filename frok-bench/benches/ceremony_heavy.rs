use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use frok_bench::{Ceremony, CeremonyConfig, LoadSpec};
use futures_util::future::join_all;

const HEAVY_CONCURRENCY: usize = 128;
const HEAVY_REQUESTS_PER_ITER: u64 = 1_000;
const HEAVY_AGENT_COUNT: usize = 6;

fn build_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

async fn run_multi_route_load(ceremony: Ceremony, routes: Vec<String>, spec: LoadSpec) {
    let futures = routes.into_iter().map(|route| {
        let ceremony = ceremony.clone();
        let spec = spec.clone();
        async move {
            ceremony.load_http(&route, spec).await.expect("load http");
        }
    });
    let _ = join_all(futures).await;
}

fn bench_scale_heavy(c: &mut Criterion) {
    let rt = build_runtime();
    let config = CeremonyConfig::heavy_mixed(HEAVY_AGENT_COUNT);
    let ceremony = rt
        .block_on(Ceremony::start(config))
        .expect("start ceremony");
    let routes = ceremony
        .routes()
        .iter()
        .map(|route| route.name.clone())
        .collect::<Vec<_>>();
    let throughput = HEAVY_REQUESTS_PER_ITER * routes.len() as u64;
    let spec = LoadSpec {
        total_requests: HEAVY_REQUESTS_PER_ITER,
        concurrency: HEAVY_CONCURRENCY,
        ..Default::default()
    };

    let mut group = c.benchmark_group("scale_heavy_mixed");
    group.throughput(Throughput::Elements(throughput));
    group.bench_function("load", |b| {
        let spec = spec.clone();
        b.to_async(&rt).iter(|| {
            let ceremony = ceremony.clone();
            let routes = routes.clone();
            let spec = spec.clone();
            async move {
                run_multi_route_load(ceremony, routes, spec).await;
            }
        })
    });
    group.finish();

    rt.block_on(ceremony.shutdown()).expect("shutdown ceremony");
}

criterion_group!(benches, bench_scale_heavy);
criterion_main!(benches);
