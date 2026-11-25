use std::io::Cursor;
use std::sync::Arc;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::io::BufReader;

use libreeb::{io::header::parse_header, make_decoder_generic, EventBuf, RawFileReader};

// ── Helper: load a file into a shared, cheaply-cloneable buffer ──────────────

fn load_file(path: &str) -> Arc<[u8]> {
    std::fs::read(path)
        .unwrap_or_else(|_| panic!("Failed to read {path}"))
        .into()
}

// ── decode_only: pre-loaded into memory, measures pure decode throughput ─────

fn bench_decode_only(c: &mut Criterion) {
    let files = [
        ("gen4_evt3_hand", "data/evt3/gen4_evt3_hand.raw"),
        ("driving_sample", "data/evt3/driving_sample.raw"),
        ("claque_doigt_evt21", "data/evt21/claque_doigt_evt21.raw"),
    ];

    let mut group = c.benchmark_group("decode_only");

    for (name, path) in &files {
        let bytes: Arc<[u8]> = load_file(path);
        group.throughput(Throughput::Bytes(bytes.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), &bytes, |b, data| {
            b.iter(|| {
                let cursor = Cursor::new(Arc::clone(data));
                let mut reader =
                    RawFileReader::from_reader(cursor).expect("Failed to parse header");
                std::iter::from_fn(|| reader.decoder().next()).count()
            })
        });
    }

    group.finish();
}

// ── decode_event_buf: same as decode_only, decoding into the packed EventBuf ──

fn bench_decode_event_buf(c: &mut Criterion) {
    let files = [
        ("gen4_evt3_hand", "data/evt3/gen4_evt3_hand.raw"),
        ("driving_sample", "data/evt3/driving_sample.raw"),
        ("claque_doigt_evt21", "data/evt21/claque_doigt_evt21.raw"),
    ];

    let mut group = c.benchmark_group("decode_event_buf");

    for (name, path) in &files {
        let bytes: Arc<[u8]> = load_file(path);
        group.throughput(Throughput::Bytes(bytes.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), &bytes, |b, data| {
            b.iter(|| {
                let mut reader = BufReader::with_capacity(64 * 1024, Cursor::new(Arc::clone(data)));
                let header = parse_header(&mut reader).expect("Failed to parse header");
                make_decoder_generic::<_, EventBuf>(reader, header.event_type)
                    .expect("Unsupported format")
                    .count()
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
                std::iter::from_fn(|| reader.decoder().next()).count()
            })
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_decode_only,
    bench_decode_event_buf,
    bench_full_pipeline
);
criterion_main!(benches);
