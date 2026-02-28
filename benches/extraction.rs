// Benchmarks for readability-rs extraction performance.
//
// Coverage:
//   - Individual pages across the small/medium/large size range
//   - Full fixture suite throughput (all 133 test pages, one parse each)

use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use libreadability::Parser;
use url::Url;

// ── per-page helpers ──────────────────────────────────────────────────────────

fn load(name: &str) -> (String, Url) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test-pages")
        .join(name)
        .join("source.html");
    let html = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("failed to read {}", path.display()));
    let url = Url::parse("http://fakehost/test/page.html").unwrap();
    (html, url)
}

fn bench_single(c: &mut Criterion, name: &str) {
    let (html, url) = load(name);
    let bytes = html.len() as u64;

    let mut group = c.benchmark_group(name);
    group.throughput(Throughput::Bytes(bytes));
    group.bench_function("parse", |b| {
        b.iter_batched(
            || (html.clone(), Parser::new()),
            |(h, mut p)| p.parse(&h, Some(&url)).unwrap(),
            BatchSize::LargeInput,
        )
    });
    group.finish();
}

// ── individual page benchmarks ────────────────────────────────────────────────

fn bench_ars_1(c: &mut Criterion) {
    bench_single(c, "ars-1"); // ~56 KB — small
}

fn bench_wapo_1(c: &mut Criterion) {
    bench_single(c, "wapo-1"); // ~180 KB — medium
}

fn bench_wikipedia(c: &mut Criterion) {
    bench_single(c, "wikipedia"); // ~244 KB — medium
}

fn bench_nytimes_3(c: &mut Criterion) {
    bench_single(c, "nytimes-3"); // ~489 KB — large
}

fn bench_yahoo_2(c: &mut Criterion) {
    bench_single(c, "yahoo-2"); // ~1.6 MB — very large
}

// ── full fixture suite throughput ─────────────────────────────────────────────

fn bench_all_fixtures(c: &mut Criterion) {
    let fixtures_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("test-pages");
    let url = Url::parse("http://fakehost/test/page.html").unwrap();

    // Collect all fixture pages that have a source.html.
    let pages: Vec<(String, String)> = std::fs::read_dir(&fixtures_dir)
        .unwrap()
        .filter_map(|e| {
            let path = e.ok()?.path();
            let src = path.join("source.html");
            if src.is_file() {
                let name = path.file_name()?.to_str()?.to_owned();
                let html = std::fs::read_to_string(src).ok()?;
                Some((name, html))
            } else {
                None
            }
        })
        .collect();

    let total_bytes: u64 = pages.iter().map(|(_, h)| h.len() as u64).sum();
    let n = pages.len() as u64;

    let mut group = c.benchmark_group("all_fixtures");
    // Report throughput as total input bytes / iteration (one full pass over all pages).
    group.throughput(Throughput::Bytes(total_bytes));
    group.bench_function(format!("{n}_pages"), |b| {
        b.iter(|| {
            for (_, html) in &pages {
                let mut parser = Parser::new();
                let _ = parser.parse(html, Some(&url));
            }
        })
    });
    group.finish();
}

// ── registration ──────────────────────────────────────────────────────────────

criterion_group!(
    benches,
    bench_ars_1,
    bench_wapo_1,
    bench_wikipedia,
    bench_nytimes_3,
    bench_yahoo_2,
    bench_all_fixtures,
);
criterion_main!(benches);
