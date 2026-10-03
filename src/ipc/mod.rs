//! Framing for the daemon ↔ client socket: newline-delimited JSON.
//!
//! Chosen over a length-prefixed binary format for one reason that keeps
//! paying off: `nc -U $XDG_RUNTIME_DIR/tmper/socket` is a working debugger.
//! The cost is a few bytes per message on a unix socket, which is nothing.
//!
//! The invariant that makes it safe is that `serde_json` never emits a raw
//! `\n`: control characters inside strings are escaped, so one message is
//! always exactly one line.

// The contract lands before its two ends do: `daemon.rs` and
// `client/handle.rs` are the next commit of the same phase. Remove this when
// they arrive — `cargo clippy -- -D warnings` is what will say so.
#![allow(dead_code)]

pub mod proto;

use std::io::{self, BufRead, Write};

use serde::de::DeserializeOwned;
use serde::Serialize;

/// A message longer than this is a protocol error, not a big message.
///
/// Nothing legitimate comes close: a full snapshot is a few hundred bytes and
/// the spectrum is ~450. A peer that sends more is either confused or hostile,
/// and either way the answer is the same — refuse it and close, rather than
/// grow the buffer until the machine swaps.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

/// Write one message and flush it.
///
/// Flushing per message is not an optimization opportunity: a client in a
/// `select!` loop is waiting on the far end, and a message parked in a buffer
/// is indistinguishable from a hung daemon.
pub fn write_message<W, T>(writer: &mut W, message: &T) -> io::Result<()>
where
    W: Write,
    T: Serialize,
{
    let line = serde_json::to_string(message)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("unserializable: {e}")))?;
    if line.len() > MAX_LINE_BYTES {
        return Err(too_long());
    }
    writer.write_all(line.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()
}

/// Read one message. `Ok(None)` is a clean EOF at a line boundary.
///
/// EOF in the middle of a line is an error rather than a `None`, because the
/// two mean different things to a client: a daemon that closed between
/// messages said goodbye, while a daemon that died mid-message dropped
/// something the client was owed.
pub fn read_message<R, T>(reader: &mut R) -> io::Result<Option<T>>
where
    R: BufRead,
    T: DeserializeOwned,
{
    let Some(line) = read_line(reader)? else {
        return Ok(None);
    };
    serde_json::from_str(&line)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("malformed: {e}")))
}

/// Read up to and including the next `\n`, without a `BufRead::read_line`'s
/// unbounded growth.
///
/// `read_line` is the obvious call and the wrong one: it will happily allocate
/// until the peer stops sending.
fn read_line<R: BufRead>(reader: &mut R) -> io::Result<Option<String>> {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if buf.is_empty() {
                Ok(None)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "connection closed mid-line",
                ))
            };
        }
        match available.iter().position(|&byte| byte == b'\n') {
            Some(index) => {
                buf.extend_from_slice(&available[..index]);
                reader.consume(index + 1);
                if buf.len() > MAX_LINE_BYTES {
                    return Err(too_long());
                }
                // Tolerate CRLF: `nc` on a terminal sends it, and this format
                // exists to be hand-driven.
                if buf.last() == Some(&b'\r') {
                    buf.pop();
                }
                return String::from_utf8(buf).map(Some).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "line is not valid UTF-8")
                });
            }
            None => {
                let len = available.len();
                buf.extend_from_slice(available);
                reader.consume(len);
                if buf.len() > MAX_LINE_BYTES {
                    return Err(too_long());
                }
            }
        }
    }
}

fn too_long() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("message exceeds {MAX_LINE_BYTES} bytes"),
    )
}

#[cfg(test)]
mod tests {
    use super::proto::*;
    use super::*;
    use std::io::{BufReader, Cursor};

    fn reader(bytes: &[u8]) -> BufReader<Cursor<Vec<u8>>> {
        BufReader::new(Cursor::new(bytes.to_vec()))
    }

    #[test]
    fn messages_round_trip_through_the_framing() {
        let mut buffer: Vec<u8> = Vec::new();
        let requests = [
            Request::Toggle,
            Request::Play {
                path: std::path::PathBuf::from("/music/一首歌.flac"),
            },
            Request::VolumeStep { delta: -0.05 },
        ];
        for request in &requests {
            write_message(&mut buffer, request).expect("write");
        }
        assert_eq!(buffer.iter().filter(|&&b| b == b'\n').count(), 3);

        let mut source = reader(&buffer);
        for request in &requests {
            let read: Option<Request> = read_message(&mut source).expect("read");
            assert_eq!(read.as_ref(), Some(request));
        }
        // The third message is the last one: the next read is a clean EOF.
        let end: Option<Request> = read_message(&mut source).expect("read");
        assert_eq!(end, None);
    }

    /// A message with an embedded newline in a string must stay one line, or
    /// the framing desynchronizes for everything after it.
    #[test]
    fn newlines_inside_a_payload_do_not_break_framing() {
        let mut buffer: Vec<u8> = Vec::new();
        write_message(
            &mut buffer,
            &Event::Notice {
                level: NoticeLevel::Error,
                message: "first\nsecond\r\nthird".into(),
            },
        )
        .expect("write");
        assert_eq!(buffer.iter().filter(|&&b| b == b'\n').count(), 1);

        let mut source = reader(&buffer);
        let read: Option<Event> = read_message(&mut source).expect("read");
        assert_eq!(
            read,
            Some(Event::Notice {
                level: NoticeLevel::Error,
                message: "first\nsecond\r\nthird".into(),
            })
        );
    }

    /// The spectrum arrives 30 times a second and is by far the biggest thing
    /// on the wire; it must survive being read across many buffer refills.
    #[test]
    fn a_line_larger_than_the_read_buffer_is_reassembled() {
        let bars: Vec<f32> = (0..64).map(|i| i as f32 / 64.0).collect();
        let mut buffer: Vec<u8> = Vec::new();
        write_message(&mut buffer, &Event::Visualizer { bars: bars.clone() }).expect("write");
        assert!(
            buffer.len() > 512,
            "should exceed a default BufReader refill"
        );

        let mut source = reader(&buffer);
        let read: Option<Event> = read_message(&mut source).expect("read");
        assert_eq!(read, Some(Event::Visualizer { bars }));
    }

    #[test]
    fn eof_between_messages_is_a_clean_end() {
        let mut source = reader(b"");
        let read: Option<Request> = read_message(&mut source).expect("read");
        assert_eq!(read, None);
    }

    #[test]
    fn eof_in_the_middle_of_a_line_is_an_error() {
        let mut source = reader(br#"{"t":"tog"#);
        let error = read_message::<_, Request>(&mut source).expect_err("should fail");
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn malformed_json_is_an_error() {
        let mut source = reader(b"{\"t\":\"teleport\"}\n");
        let error = read_message::<_, Request>(&mut source).expect_err("should fail");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn an_oversized_line_is_refused_without_being_buffered() {
        let mut bytes = vec![b'x'; MAX_LINE_BYTES + 1];
        bytes.push(b'\n');
        let mut source = reader(&bytes);
        let error = read_message::<_, Request>(&mut source).expect_err("should fail");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_carriage_return_before_the_newline_is_tolerated() {
        let mut source = reader(b"{\"t\":\"toggle\"}\r\n");
        let read: Option<Request> = read_message(&mut source).expect("read");
        assert_eq!(read, Some(Request::Toggle));
    }

    /// A blank line is not an absent message — it means the peer sent
    /// something it should not have, and guessing is worse than saying so.
    #[test]
    fn a_blank_line_is_an_error() {
        let mut source = reader(b"\n");
        let error = read_message::<_, Request>(&mut source).expect_err("should fail");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_line_that_is_not_utf8_is_an_error() {
        let mut source = reader(&[0xff, 0xfe, b'\n']);
        let error = read_message::<_, Request>(&mut source).expect_err("should fail");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
