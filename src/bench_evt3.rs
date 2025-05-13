use libreeb::RawFileReader;
use std::path::Path;

fn main() {
    let mut reader = RawFileReader::new(Path::new(
        "/home/tvercueil/ws/libreeb/data/openeb/gen4_evt3_hand.raw",
    ))
    .unwrap();
    let it = reader.read_events();
    let count = it.count();
    print!("{}\n", count);
}
