use std::path::Path;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use libreeb::RawFileReader;

pub fn evt3_decode_benchmark(c: &mut Criterion) {
    c.bench_function("evt3 decode", |b| {
        b.iter(|| {
            let path = Path::new("data/openeb/gen4_evt2_hand.raw");
            let mut reader =
                RawFileReader::new(Path::new(&path)).expect("Failed to open test file");
            let event_iterator = reader.read_events();
            event_iterator.count()
        })
    });
}

criterion_group!(benches, evt3_decode_benchmark);
criterion_main!(benches);
