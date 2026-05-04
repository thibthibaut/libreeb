use std::io::Cursor;
use std::sync::Arc;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use libreeb::RawFileReader;

// ── Helper: load a file into a shared, cheaply-cloneable buffer ──────────────

fn load_file(path: &str) -> Arc<[u8]> {
    std::fs::read(path)
        .unwrap_or_else(|_| panic!("Failed to read {path}"))
        .into()
}

// ── decode_only: pre-loaded into memory, measures pure decode throughput ─────

fn bench_decode_only(c: &mut Criterion) {
    let files = [
        ("gen4_evt3_hand", "data/gen4_evt3_hand.raw"),
        ("driving_sample", "data/driving_sample.raw"),
        ("claque_doigt_evt21", "data/claque_doigt_evt21.raw"),
    ];

    let mut group = c.benchmark_group("decode_only");

    for (name, path) in &files {
        let bytes: Arc<[u8]> = load_file(path);
        group.throughput(Throughput::Bytes(bytes.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), &bytes, |b, data| {
            b.iter(|| {
                // Arc::clone is a refcount bump — no data copy.
                let cursor = Cursor::new(Arc::clone(data));
                let mut reader =
                    RawFileReader::from_reader(cursor).expect("Failed to parse header");
                reader.read_events().count()
            })
        });
    }

    group.finish();
}

// ── full_pipeline: open + read + decode from disk (I/O included) ─────────────

fn bench_full_pipeline(c: &mut Criterion) {
    let files = [
        ("gen4_evt3_hand", "data/gen4_evt3_hand.raw"),
        ("claque_doigt_evt21", "data/claque_doigt_evt21.raw"),
    ];

    let mut group = c.benchmark_group("full_pipeline");

    for (name, path) in &files {
        let bytes_len = std::fs::metadata(path)
            .unwrap_or_else(|_| panic!("Cannot stat {path}"))
            .len();
        group.throughput(Throughput::Bytes(bytes_len));
        group.bench_function(BenchmarkId::from_parameter(name), |b| {
            b.iter(|| {
                let mut reader =
                    RawFileReader::new(std::path::Path::new(path)).expect("Failed to open file");
                reader.read_events().count()
            })
        });
    }

    group.finish();
}

criterion_group!(benches, bench_decode_only, bench_full_pipeline);
criterion_main!(benches);
