#![feature(test)]
use std::{env, path::Path};

use openevt::RawFileReader;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <filename>", args[0]);
        std::process::exit(1);
    }

    let mut file_reader = RawFileReader::new(Path::new(&args[1]));
    let it = file_reader.read_events();

    for evt in it {
        println!("{:?}", evt);
    }
}

extern crate test;
#[cfg(test)]
mod tests {
    use super::*;
    use openevt::EventCD;
    use test::Bencher;

    #[test]
    fn last_event() {
        let mut reader = RawFileReader::new(Path::new("data/laser_small.raw"));
        let last = reader.read_events().last().unwrap();
        let expected_last = EventCD {
            x: 390,
            y: 486,
            p: 0,
            t: 6438891,
        };
        assert_eq!(last, expected_last);
    }

    #[test]
    fn sum_of_values() {
        let mut reader = RawFileReader::new(Path::new("data/laser_small.raw"));
        let result = reader.read_events();
        let (sum_of_polarities, sum_of_x, sum_of_y) = result
            .fold((0, 0, 0), |(p_sum, x_sum, y_sum), e| {
                (p_sum + e.p as i64, x_sum + e.x as i64, y_sum + e.y as i64)
            });
        assert_eq!(sum_of_x, 41884496);
        assert_eq!(sum_of_y, 24328383);
        assert_eq!(sum_of_polarities, 7636);
    }

    #[bench]
    fn read_events(b: &mut Bencher) {
        b.iter(|| {
            let mut reader = RawFileReader::new(Path::new("data/laser_small.raw"));
            let result = reader.read_events().collect_vec();
            result
        })
    }
}
