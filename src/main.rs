use color_eyre::{owo_colors::OwoColorize, Result};
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, KeyEventKind},
    ExecutableCommand,
};
use itertools::Itertools;
use libreeb::{slice_events, Event, RawFileReader, SliceBy};
use ratatui::{
    crossterm::event::{self, KeyCode, MouseEventKind},
    layout::{Alignment, Constraint, Layout, Position, Rect},
    style::{Color, Stylize},
    symbols::Marker,
    widgets::{
        canvas::{Canvas, Circle, Map, MapResolution, Points, Rectangle},
        Block, Paragraph, Widget, Wrap,
    },
    DefaultTerminal, Frame,
};
use std::{io::stdout, path::Path};
use time::{Duration, OffsetDateTime};

fn main() -> Result<()> {
    color_eyre::install()?;
    let mut pargs = pico_args::Arguments::from_env();

    if pargs.contains(["-h", "--help"]) {
        println!("You asked for help, good luck");
        std::process::exit(0);
    }

    let path: String = pargs.value_from_str("--input")?;

    stdout().execute(EnableMouseCapture)?;
    let terminal = ratatui::init();
    let mut reader = RawFileReader::new(Path::new(&path))?;
    println!("HEADER: {:?}", &reader.header);
    // let mut it = reader.read_events();

    // let mut events = it.collect_vec();
    // let first = events.first();
    // let last = events.last();
    // println!("first: {:?}, last {:?}", first, last);
    // reader.reversed()

    // let mut sum = 0;
    // for evt in it {
    //     println!("{:?}", evt);
    //     sum += 1
    // }

    // println!("{:?}", sum);
    let app_result = App::new(reader).run(terminal);
    ratatui::restore();
    stdout().execute(DisableMouseCapture)?;
    app_result
    // Ok(())
}

struct App {
    exit: bool,
    x: f64,
    y: f64,
    tick_count: u64,
    marker: Marker,
    positive_points: Vec<Position>,
    negative_points: Vec<Position>,
    is_drawing: bool,
    file_reader: RawFileReader,
    current_timetamp: u64,
    slice_duration: u64,
    fps: f64,
    pause: bool,
    step: bool,
}

impl App {
    fn new(file_reader: RawFileReader) -> Self {
        Self {
            exit: false,
            x: 0.0,
            y: 0.0,
            tick_count: 0,
            marker: Marker::Braille,
            positive_points: vec![],
            negative_points: vec![],
            is_drawing: false,
            file_reader,
            current_timetamp: 0,
            slice_duration: 1_000,
            fps: 0.0,
            pause: false,
            step: false,
        }
    }

    pub fn run(mut self, mut terminal: DefaultTerminal) -> Result<()> {
        let tick_rate = std::time::Duration::from_millis(1000 / 30);
        // let tick_rate = Duration::from_secs(16);
        let mut last_tick = std::time::Instant::now();
        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;
            let timeout = tick_rate.saturating_sub(last_tick.elapsed());
            if event::poll(timeout)? {
                match event::read()? {
                    crossterm::event::Event::Key(key) => self.handle_key_press(key),
                    crossterm::event::Event::Mouse(event) => self.handle_mouse_event(event),
                    _ => (),
                }
            }

            if last_tick.elapsed() >= tick_rate {
                self.fps = 1000.0 / last_tick.elapsed().as_millis() as f64;
                self.on_tick();
                last_tick = std::time::Instant::now();
            }
        }
        Ok(())
    }

    fn handle_key_press(&mut self, key: event::KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.exit = true,
            KeyCode::Char('p') => self.pause = !self.pause,
            KeyCode::Char('s') => self.step = true,
            KeyCode::Down | KeyCode::Char('j') => self.y += 1.0,
            KeyCode::Up | KeyCode::Char('k') => self.y -= 1.0,
            KeyCode::Right | KeyCode::Char('l') => self.x += 1.0,
            KeyCode::Left | KeyCode::Char('h') => self.x -= 1.0,
            _ => {}
        }
    }

    fn handle_mouse_event(&mut self, event: event::MouseEvent) {
        match event.kind {
            MouseEventKind::Down(_) => self.is_drawing = true,
            MouseEventKind::Up(_) => self.is_drawing = false,
            // MouseEventKind::Drag(_) => {
            //     self.points.push(Position::new(event.column, event.row));
            // }
            _ => {}
        }
    }

    fn on_tick(&mut self) {
        // Do do anything if we are in pause
        if self.pause && !self.step {
            return;
        }
        // let data = self.file_reader.read_events().take(4048 * 2).collect_vec();
        let data = slice_events(self.file_reader.read_events(), SliceBy::Time(2000)).next();

        if let Some(mut data) = data {
            // Keep only cd events (for now) TODO: Maybe handle external triggers
            data.retain(|e| matches!(e, Event::CD { .. }));

            self.current_timetamp = data.first().unwrap().timestamp().unwrap();

            self.positive_points = data
                .iter()
                .filter(|evt| evt.polarity().unwrap() == 1)
                .map(|evt| Position {
                    x: evt.x().unwrap(),
                    y: evt.y().unwrap(),
                })
                .collect_vec();

            self.negative_points = data
                .iter()
                .filter(|evt| evt.polarity().unwrap() == 0)
                .map(|evt| Position {
                    x: evt.x().unwrap(),
                    y: evt.y().unwrap(),
                })
                .collect_vec();
        } else {
            self.file_reader.reset();
        }
        self.tick_count += 1;
        self.step = false;
    }

    fn draw(&self, frame: &mut Frame) {
        let horizontal = Layout::horizontal([Constraint::Length(320), Constraint::Length(320)]);
        let vertical = Layout::vertical([Constraint::Length(320), Constraint::Length(320)]);
        let [left, right] = horizontal.areas(frame.area());
        let [draw, map] = vertical.areas(left);
        let [pong, boxes] = vertical.areas(right);

        frame.render_widget(self.draw_canvas(draw), draw);
        frame.render_widget(self.pong_canvas(), pong);
    }

    fn map_canvas(&self) -> impl Widget + '_ {
        Canvas::default()
            .block(Block::bordered().title("Event Data"))
            .paint(|ctx| {
                ctx.draw(&Map {
                    color: Color::Green,
                    resolution: MapResolution::High,
                });
                // ctx.print(self.x, -self.y, "You are here".yellow());
            })
            .x_bounds([0.0, 1.0])
            .y_bounds([0.0, 1.0])
    }

    fn draw_canvas(&self, area: Rect) -> impl Widget + '_ {
        Canvas::default()
            .block(Block::bordered().title("Event Slices"))
            .marker(self.marker)
            .x_bounds([0.0, 1.0 /*f64::from(area.width)*/])
            .y_bounds([0.0, 1.0 /*f64::from(area.height)*/])
            .paint(move |ctx| {
                let ppoints = self
                    .positive_points
                    .iter()
                    .map(|p| (p.x as f64 / 320.0, 1.0 - (p.y as f64 / 320.0)))
                    .collect_vec();

                let npoints = self
                    .negative_points
                    .iter()
                    .map(|p| (p.x as f64 / 320.0, 1.0 - (p.y as f64 / 320.0)))
                    .collect_vec();

                ctx.draw(&Points {
                    coords: &ppoints,
                    color: Color::Blue,
                });
                ctx.draw(&Points {
                    coords: &npoints,
                    color: Color::Magenta,
                });
            })
    }

    fn pong_canvas(&self) -> impl Widget + '_ {
        let seconds = (self.current_timetamp / 1_000_000) as i64;
        let micros = (self.current_timetamp % 1_000_000) as i32;
        // Create timestamp from UNIX epoch
        let time = OffsetDateTime::from_unix_timestamp(seconds).unwrap()
            + Duration::microseconds(micros as i64);

        let timestamp = format!(
            "{:02}:{:02}:{:02}.{:03}",
            time.hour(),
            time.minute(),
            time.second(),
            time.microsecond() / 1000
        );

        Paragraph::new(format!("Timestamp: {}\n FPS {:.1}", timestamp, self.fps))
            .block(Block::bordered().title("Info"))
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: true })
    }

    fn boxes_canvas(&self, area: Rect) -> impl Widget {
        let left = 0.0;
        let right = f64::from(area.width);
        let bottom = 0.0;
        let top = f64::from(area.height).mul_add(2.0, -4.0);
        Canvas::default()
            .block(Block::bordered().title("Rects"))
            .marker(self.marker)
            .x_bounds([left, right])
            .y_bounds([bottom, top])
            .paint(|ctx| {
                for i in 0..=11 {
                    ctx.draw(&Rectangle {
                        x: f64::from(i * i + 3 * i) / 2.0 + 2.0,
                        y: 2.0,
                        width: f64::from(i),
                        height: f64::from(i),
                        color: Color::Red,
                    });
                    ctx.draw(&Rectangle {
                        x: f64::from(i * i + 3 * i) / 2.0 + 2.0,
                        y: 21.0,
                        width: f64::from(i),
                        height: f64::from(i),
                        color: Color::Blue,
                    });
                }
                for i in 0..100 {
                    if i % 10 != 0 {
                        ctx.print(f64::from(i) + 1.0, 0.0, format!("{i}", i = i % 10));
                    }
                    if i % 2 == 0 && i % 10 != 0 {
                        ctx.print(0.0, f64::from(i), format!("{i}", i = i % 10));
                    }
                }
            })
    }
}

/*
#![feature(test)]

use color_eyre::Result;
use itertools::Itertools;
use libreeb::RawFileReader;
use std::{env, io, path::Path};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Color,
    style::Stylize,
    symbols::border,
    text::Text,
    widgets::{
        canvas::{Canvas, Line, Map, MapResolution, Rectangle},
        Block, Paragraph, Widget,
    },
    DefaultTerminal, Frame,
};

#[derive(Debug, Default)]
pub struct App {
    counter: u8,
    exit: bool,
}

fn main() -> io::Result<()> {
    let mut terminal = ratatui::init();
    let app_result = App::default().run(&mut terminal);
    app_result
}

impl App {
    /// runs the application's main loop until the user quits
    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;
            self.handle_events()?;
        }
        Ok(())
    }

    fn draw(&self, frame: &mut Frame) {
        frame.render_widget(self, frame.area());
    }

    fn handle_events(&mut self) -> io::Result<()> {
        match event::read()? {
            // it's important to check that the event is a key press event as
            // crossterm also emits key release and repeat events on Windows.
            Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                self.handle_key_event(key_event)
            }
            _ => {}
        };
        Ok(())
    }

    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Char('q') => self.exit(),
            KeyCode::Left => self.decrement_counter(),
            KeyCode::Right => self.increment_counter(),
            _ => {}
        }
    }

    fn exit(&mut self) {
        self.exit = true;
    }

    fn increment_counter(&mut self) {
        self.counter += 1;
    }

    fn decrement_counter(&mut self) {
        self.counter -= 1;
    }
}

impl Widget for &App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Canvas::default()
            .block(Block::bordered().title("Canvas"))
            .x_bounds([-180.0, 180.0])
            .y_bounds([-90.0, 90.0])
            .paint(|ctx| {
                ctx.draw(&Map {
                    resolution: MapResolution::High,
                    color: Color::White,
                });
                ctx.layer();
                ctx.draw(&Line {
                    x1: 0.0,
                    y1: 10.0,
                    x2: 10.0,
                    y2: 10.0,
                    color: Color::White,
                });
                ctx.draw(&Rectangle {
                    x: 10.0,
                    y: 20.0,
                    width: 10.0,
                    height: 10.0,
                    color: Color::Red,
                });
            });
    }
}

/*
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
*/
extern crate test;
#[cfg(test)]
mod tests {
    use super::*;
    use libreeb::EventCD;
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
*/
