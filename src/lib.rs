#![feature(portable_simd)]
#![feature(avx512_target_feature)]
use std::{
    fs::File,
    io::{BufRead, BufReader, Read},
    path::Path,
};

pub use evt3::*; // Re-export evt3 public

pub mod evt3;
mod macros;

#[derive(Debug, PartialEq, Eq)]
pub struct EventCD {
    pub x: u16,
    pub y: u16,
    pub p: u8,
    pub t: u64,
}

pub trait EventDecoder {
    fn decode(&mut self, reader: &mut impl Read) -> impl Iterator<Item = EventCD>;
}

pub struct RawFileReader {
    reader: BufReader<File>,
    decoder: Evt3Decoder,
}

impl RawFileReader {
    pub fn new(path: &Path) -> Self {
        let file = File::open(path).expect("Cannot read file");
        let reader = BufReader::with_capacity(32 * 1024, file);
        let decoder = Evt3Decoder::default();
        RawFileReader { reader, decoder }
    }

    pub fn read_events(&mut self) -> impl Iterator<Item = EventCD> + use<'_> {
        loop {
            // Look at the next char without consuming it
            let buffer = self.reader.fill_buf().unwrap();
            let next_char = buffer[0];
            if next_char != b'%' {
                // Read end of header section
                break;
            }
            // Read line
            let mut header_line = String::new();
            self.reader.read_line(&mut header_line).unwrap();
            println!("Header Entry {}", header_line[1..].trim());
        }
        self.decoder.decode(&mut self.reader)
    }
}
