//! Framing for the daemon ↔ client socket: newline-delimited JSON.
//!
//! Chosen over a length-prefixed binary format for one reason that keeps
//! paying off: `nc -U $XDG_RUNTIME_DIR/tmper/socket` is a working debugger.
//! The cost is a few bytes per message on a unix socket, which is nothing.
//!
//! The invariant that makes it safe is that `serde_json` never emits a raw
//! `\n`: control characters inside strings are escaped, so one ordinary
//! message is one line. Protocol 2 splits larger messages into
//! base64 chunk frames; both the physical line and reassembled message are capped.
//!
//! Both ends of the socket are tokio tasks and both use the `_async` pair
//! below. There were once blocking twins of each — the client's handshake was
//! the last caller — and they are gone with it: one implementation cannot
//! disagree with itself about the cap, the CRLF rule or what a truncated line
//! means, which is what the parity tests that used to live here existed to
//! check.

pub mod proto;

use std::io;

use base64::Engine;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Physical NDJSON line cap. Large logical messages use bounded chunk frames.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;
/// Logical message cap, including a large queue or playlist store.
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;
const CHUNK_BYTES: usize = 512 * 1024;
const MAX_CHUNKS: usize = 1024;

#[derive(Serialize, Deserialize)]
struct Chunk {
    _tmper_chunk: String,
    more: bool,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Frame<T> {
    Chunk(Chunk),
    Message(T),
}

/// Write one message and flush it.
///
/// Flushing per message is not an optimization opportunity: a client in a
/// `select!` loop is waiting on the far end, and a message parked in a buffer
/// is indistinguishable from a hung daemon.
pub async fn write_message_async<W, T>(writer: &mut W, message: &T) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let bytes = serde_json::to_vec(message)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("unserializable: {e}")))?;
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(message_too_long());
    }
    if bytes.len() <= MAX_LINE_BYTES {
        writer.write_all(&bytes).await?;
        writer.write_all(b"\n").await?;
    } else {
        let mut chunks = bytes.chunks(CHUNK_BYTES).peekable();
        while let Some(bytes) = chunks.next() {
            let frame = Chunk {
                _tmper_chunk: base64::engine::general_purpose::STANDARD.encode(bytes),
                more: chunks.peek().is_some(),
            };
            let line = serde_json::to_vec(&frame).map_err(io::Error::other)?;
            writer.write_all(&line).await?;
            writer.write_all(b"\n").await?;
        }
    }
    writer.flush().await
}

/// Read one message. `Ok(None)` is a clean EOF at a line boundary.
///
/// EOF in the middle of a line is an error rather than a `None`, because the
/// two mean different things to a client: a daemon that closed between
/// messages said goodbye, while a daemon that died mid-message dropped
/// something the client was owed.
pub async fn read_message_async<R, T>(reader: &mut R) -> io::Result<Option<T>>
where
    R: AsyncBufRead + Unpin,
    T: DeserializeOwned,
{
    let mut assembled = Vec::new();
    let mut chunks = 0;
    loop {
        let Some(buf) = read_line_async(reader).await? else {
            if chunks == 0 {
                return Ok(None);
            }
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed mid-message",
            ));
        };
        let frame: Frame<T> = serde_json::from_slice(&buf)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("malformed: {e}")))?;
        match frame {
            Frame::Message(message) if chunks == 0 => return Ok(Some(message)),
            Frame::Message(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "message interrupted chunk sequence",
                ))
            }
            Frame::Chunk(chunk) => {
                chunks += 1;
                if chunks > MAX_CHUNKS {
                    return Err(message_too_long());
                }
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(chunk._tmper_chunk)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                if bytes.is_empty() {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "empty chunk"));
                }
                if assembled.len() + bytes.len() > MAX_MESSAGE_BYTES {
                    return Err(message_too_long());
                }
                assembled.extend_from_slice(&bytes);
                if !chunk.more {
                    return serde_json::from_slice(&assembled)
                        .map(Some)
                        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e));
                }
            }
        }
    }
}

/// One line, capped, with a `\r` before the `\n` tolerated.
///
/// `Take` is what bounds the read. `read_until` on its own runs until it finds
/// a newline or the peer stops sending, which is how a peer turns a message
/// into an allocation the size of what it cares to send; capping the reader two
/// bytes past the limit makes the allocation unable to exceed it, whatever the
/// peer does.
async fn read_line_async<R: AsyncBufRead + Unpin>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut buf: Vec<u8> = Vec::new();
    let mut capped = reader.take(MAX_LINE_BYTES as u64 + 2);
    capped.read_until(b'\n', &mut buf).await?;
    if buf.is_empty() {
        return Ok(None); // clean EOF at a line boundary
    }

    // Length first, then the newline: an over-long line is a protocol error
    // whether or not it ever ended, and checking in this order is what keeps
    // the two implementations agreeing on which error a given byte string is.
    let complete = buf.ends_with(b"\n");
    let content_len = buf.len() - usize::from(complete);
    if content_len > MAX_LINE_BYTES {
        return Err(too_long());
    }
    if !complete {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "connection closed mid-line",
        ));
    }
    buf.truncate(content_len);
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    Ok(Some(buf))
}

fn too_long() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("message exceeds {MAX_LINE_BYTES} bytes"),
    )
}

fn message_too_long() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("message exceeds {MAX_MESSAGE_BYTES} bytes or {MAX_CHUNKS} chunks"),
    )
}

#[cfg(test)]
mod tests {
    use super::proto::*;
    use super::*;
    use std::io::Cursor;

    fn reader(bytes: &[u8]) -> tokio::io::BufReader<Cursor<Vec<u8>>> {
        tokio::io::BufReader::new(Cursor::new(bytes.to_vec()))
    }

    #[tokio::test]
    async fn a_large_unicode_collection_is_chunked_and_followed_by_another_message() {
        let event = Event::Playlists {
            playlists: vec![crate::playlist::PlaylistData {
                id: 1,
                name: "大歌单".into(),
                songs: (0..20000)
                    .map(|i| {
                        std::path::PathBuf::from(format!(
                            "/music/{i}/{}\".flac",
                            "中文\\".repeat(10)
                        ))
                    })
                    .collect(),
            }],
        };
        assert!(serde_json::to_vec(&event).unwrap().len() > MAX_LINE_BYTES);
        let mut bytes = Vec::new();
        write_message_async(&mut bytes, &event).await.unwrap();
        write_message_async(&mut bytes, &Event::Bye).await.unwrap();
        assert!(bytes
            .split(|b| *b == b'\n')
            .all(|line| line.len() <= MAX_LINE_BYTES));
        let mut source = reader(&bytes);
        assert_eq!(
            read_message_async::<_, Event>(&mut source).await.unwrap(),
            Some(event)
        );
        assert_eq!(
            read_message_async::<_, Event>(&mut source).await.unwrap(),
            Some(Event::Bye)
        );
        assert_eq!(
            read_message_async::<_, Event>(&mut source).await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn interrupted_and_invalid_chunks_are_rejected() {
        let chunk = serde_json::to_string(&Chunk {
            _tmper_chunk: "e30=".into(),
            more: true,
        })
        .unwrap();
        let mut source = reader(format!("{chunk}\n").as_bytes());
        assert_eq!(
            read_message_async::<_, Event>(&mut source)
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::UnexpectedEof
        );
        for bytes in [
            format!("{chunk}\n{{\"t\":\"bye\"}}\n"),
            "{\"_tmper_chunk\":\"!invalid!\",\"more\":false}\n".into(),
            "{\"_tmper_chunk\":\"\",\"more\":true}\n".into(),
            format!("{chunk}\n").repeat(MAX_CHUNKS + 1),
        ] {
            let mut source = reader(bytes.as_bytes());
            assert_eq!(
                read_message_async::<_, Event>(&mut source)
                    .await
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidData
            );
        }
    }

    #[tokio::test]
    async fn messages_round_trip_through_the_framing() {
        let mut buffer: Vec<u8> = Vec::new();
        let requests = [
            Request::Toggle,
            Request::Play {
                path: std::path::PathBuf::from("/music/一首歌.flac"),
            },
            Request::VolumeStep { delta: -0.05 },
        ];
        for request in &requests {
            write_message_async(&mut buffer, request)
                .await
                .expect("write");
        }
        assert_eq!(buffer.iter().filter(|&&b| b == b'\n').count(), 3);

        let mut source = reader(&buffer);
        for request in &requests {
            let read: Option<Request> = read_message_async(&mut source).await.expect("read");
            assert_eq!(read.as_ref(), Some(request));
        }
        // The third message is the last one: the next read is a clean EOF.
        let end: Option<Request> = read_message_async(&mut source).await.expect("read");
        assert_eq!(end, None);
    }

    /// A message with an embedded newline in a string must stay one line, or
    /// the framing desynchronizes for everything after it.
    #[tokio::test]
    async fn newlines_inside_a_payload_do_not_break_framing() {
        let mut buffer: Vec<u8> = Vec::new();
        write_message_async(
            &mut buffer,
            &Event::Notice {
                level: NoticeLevel::Error,
                message: "first\nsecond\r\nthird".into(),
            },
        )
        .await
        .expect("write");
        assert_eq!(buffer.iter().filter(|&&b| b == b'\n').count(), 1);

        let mut source = reader(&buffer);
        let read: Option<Event> = read_message_async(&mut source).await.expect("read");
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
    #[tokio::test]
    async fn a_line_larger_than_the_read_buffer_is_reassembled() {
        let bars: Vec<f32> = (0..64).map(|i| i as f32 / 64.0).collect();
        let mut buffer: Vec<u8> = Vec::new();
        write_message_async(&mut buffer, &Event::Visualizer { bars: bars.clone() })
            .await
            .expect("write");
        assert!(
            buffer.len() > 512,
            "should exceed a default BufReader refill"
        );

        let mut source = reader(&buffer);
        let read: Option<Event> = read_message_async(&mut source).await.expect("read");
        assert_eq!(read, Some(Event::Visualizer { bars }));
    }

    /// Back to back on one buffer: the reader must stop at each newline rather
    /// than take the two messages for one.
    #[tokio::test]
    async fn a_second_message_waits_for_its_own_read() {
        let mut buffer = Vec::new();
        write_message_async(&mut buffer, &Event::Bye)
            .await
            .expect("write");
        write_message_async(&mut buffer, &Event::Bye)
            .await
            .expect("write");

        let mut source = reader(&buffer);
        for _ in 0..2 {
            let read: Option<Event> = read_message_async(&mut source).await.expect("read");
            assert_eq!(read, Some(Event::Bye));
        }
        let end: Option<Event> = read_message_async(&mut source).await.expect("read");
        assert_eq!(end, None);
    }

    #[tokio::test]
    async fn eof_between_messages_is_a_clean_end() {
        let mut source = reader(b"");
        let read: Option<Request> = read_message_async(&mut source).await.expect("read");
        assert_eq!(read, None);
    }

    #[tokio::test]
    async fn eof_in_the_middle_of_a_line_is_an_error() {
        let mut source = reader(br#"{"t":"tog"#);
        let error = read_message_async::<_, Request>(&mut source)
            .await
            .expect_err("should fail");
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[tokio::test]
    async fn malformed_json_is_an_error() {
        let mut source = reader(b"{\"t\":\"teleport\"}\n");
        let error = read_message_async::<_, Request>(&mut source)
            .await
            .expect_err("should fail");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    /// The cap holds whether or not the line ever ended: a peer that sends a
    /// megabyte with no newline is refused for being too long, not buffered
    /// until it runs out of socket.
    #[tokio::test]
    async fn an_oversized_line_is_refused_without_being_buffered() {
        let mut terminated = vec![b'x'; MAX_LINE_BYTES + 1];
        terminated.push(b'\n');
        let mut source = reader(&terminated);
        let error = read_message_async::<_, Request>(&mut source)
            .await
            .expect_err("should fail");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);

        let unterminated = vec![b'x'; MAX_LINE_BYTES + 4];
        let mut source = reader(&unterminated);
        let error = read_message_async::<_, Request>(&mut source)
            .await
            .expect_err("should fail");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn a_carriage_return_before_the_newline_is_tolerated() {
        let mut source = reader(b"{\"t\":\"toggle\"}\r\n");
        let read: Option<Request> = read_message_async(&mut source).await.expect("read");
        assert_eq!(read, Some(Request::Toggle));
    }

    /// A blank line is not an absent message — it means the peer sent
    /// something it should not have, and guessing is worse than saying so.
    #[tokio::test]
    async fn a_blank_line_is_an_error() {
        let mut source = reader(b"\n");
        let error = read_message_async::<_, Request>(&mut source)
            .await
            .expect_err("should fail");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn a_line_that_is_not_utf8_is_an_error() {
        let mut source = reader(&[0xff, 0xfe, b'\n']);
        let error = read_message_async::<_, Request>(&mut source)
            .await
            .expect_err("should fail");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
