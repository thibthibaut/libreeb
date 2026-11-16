//! TeeReader - A Read wrapper that copies bytes to a recording buffer.

use std::io::{BufRead, Read, Result};
use std::sync::{Arc, Mutex};

/// A reader that "tees" bytes to a recording buffer while passing them through.
pub struct TeeReader<R: Read> {
    inner: R,
    recorder: Option<Arc<Mutex<Vec<u8>>>>,
}

impl<R: Read> TeeReader<R> {
    pub fn new(inner: R) -> Self {
        TeeReader {
            inner,
            recorder: None,
        }
    }

    pub fn with_recorder(inner: R, recorder: Arc<Mutex<Vec<u8>>>) -> Self {
        TeeReader {
            inner,
            recorder: Some(recorder),
        }
    }

    pub fn set_recorder(&mut self, recorder: Option<Arc<Mutex<Vec<u8>>>>) {
        self.recorder = recorder;
    }
}

impl<R: Read> Read for TeeReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let n = self.inner.read(buf)?;

        // Copy read bytes to recorder if active
        if n > 0 {
            if let Some(ref rec) = self.recorder {
                if let Ok(mut buffer) = rec.lock() {
                    buffer.extend_from_slice(&buf[..n]);
                }
            }
        }

        Ok(n)
    }
}

impl<R: Read + BufRead> BufRead for TeeReader<R> {
    fn fill_buf(&mut self) -> Result<&[u8]> {
        self.inner.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        self.inner.consume(amt)
    }
}
