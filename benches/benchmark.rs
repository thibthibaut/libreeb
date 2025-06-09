use std::path::Path;

use criterion::{criterion_group, criterion_main, Criterion};
use libreeb::RawFileReader;

pub fn evt3_decode_benchmark(c: &mut Criterion) {
    c.bench_function("evt3_decode", |b| {
        b.iter(|| {
            let path = Path::new("data/openeb/gen4_evt3_hand.raw");
            let mut reader =
                RawFileReader::new(Path::new(&path)).expect("Failed to open test file");
            let event_iterator = reader.read_events();
            event_iterator.count()
        })
    });
}

pub fn evt21_decode_benchmark(c: &mut Criterion) {
    c.bench_function("evt21_decode", |b| {
        b.iter(|| {
            let path = Path::new("data/openeb/claque_doigt_evt21.raw");
            let mut reader =
                RawFileReader::new(Path::new(&path)).expect("Failed to open test file");
            let event_iterator = reader.read_events();
            event_iterator.count()
        })
    });
}

criterion_group!(benches, evt3_decode_benchmark, evt21_decode_benchmark);
criterion_main!(benches);
