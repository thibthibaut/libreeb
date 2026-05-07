use std::env;
use std::path::Path;

use libreeb::RawFileReader;

fn benchmark(filepath: &str) -> usize {
    let path = Path::new(filepath);
    let mut reader = RawFileReader::new(path).expect("Failed to open file");

    let mut total = 0;
    while let Some(batch) = reader.decoder().next() {
        total += batch.len();
    }

    total
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: {} <filepath>", args[0]);
        std::process::exit(1);
    }

    let filename = &args[1];
    let result = benchmark(filename);

    println!("{}", result);
}
