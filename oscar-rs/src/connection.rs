//! Wraps a raw TCP socket and speaks FLAP framing on top of it, using Tokio.
//! One instance = one TCP connection to either the auth server or the BOS
//! server — a full OSCAR login involves connecting to both in sequence (see
//! `client.rs`).
//!
//! Worth calling out vs. the original Swift version: Tokio's `read_exact`
//! lets us read "the next N bytes" directly, so there's no need for the
//! manual buffer-and-drain loop `FLAPConnection.swift` needed to handle
//! `NWConnection`'s arbitrary-sized read callbacks. Same protocol, simpler
//! implementation, courtesy of a blocking-style async API instead of a
//! callback-based one.
//!
//! `FlapConnection` can be split via `into_split()` into a `FlapReader` +
//! `FlapWriter` pair. This exists for the Tauri layer: a background task
//! owns the reader and forwards parsed frames over a channel, while the
//! writer stays with `OscarSession` for outgoing commands — critically,
//! nothing ever races the raw multi-step socket read inside a `select!`,
//! since `read_exact` is not cancellation-safe (dropping a read mid-flight
//! silently loses whatever bytes were already pulled off the socket for
//! that frame, permanently desyncing the FLAP stream).

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;

use crate::flap::{FlapChannel, FlapFrame, FLAP_HEADER_SIZE};
use crate::server_address::ServerAddress;
use crate::snac::Snac;

async fn read_frame(stream: &mut (impl tokio::io::AsyncRead + Unpin)) -> std::io::Result<Option<FlapFrame>> {
    let mut header = [0u8; FLAP_HEADER_SIZE];
    match stream.read_exact(&mut header).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }

    let (channel, sequence, length) = FlapFrame::parse_header(&header).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "malformed FLAP header (bad magic byte or unknown channel)")
    })?;

    let mut payload = vec![0u8; length as usize];
    if length > 0 {
        stream.read_exact(&mut payload).await?;
    }

    Ok(Some(FlapFrame { channel, sequence, payload }))
}

async fn send(
    stream: &mut (impl tokio::io::AsyncWrite + Unpin),
    send_sequence: &mut u16,
    channel: FlapChannel,
    payload: Vec<u8>,
) -> std::io::Result<()> {
    *send_sequence = send_sequence.wrapping_add(1);
    let frame = FlapFrame { channel, sequence: *send_sequence, payload };
    stream.write_all(&frame.encode()).await
}

pub struct FlapConnection {
    stream: TcpStream,
    send_sequence: u16,
}

impl FlapConnection {
    pub async fn connect(address: &ServerAddress) -> std::io::Result<Self> {
        let stream = TcpStream::connect((address.host.as_str(), address.port)).await?;
        Ok(FlapConnection { stream, send_sequence: 0 })
    }

    /// Wraps an already-open stream. `connect()` above is what OscarClient
    /// uses for real outbound connections; this exists so tests can wrap an
    /// *accepted* stream when standing in a fake server.
    pub fn from_stream(stream: TcpStream) -> Self {
        FlapConnection { stream, send_sequence: 0 }
    }

    /// Sends a payload on the given channel, handling sequence numbering automatically.
    pub async fn send(&mut self, channel: FlapChannel, payload: Vec<u8>) -> std::io::Result<()> {
        send(&mut self.stream, &mut self.send_sequence, channel, payload).await
    }

    /// Convenience for sending a SNAC — wraps it as a channel-2 data frame.
    pub async fn send_snac(&mut self, snac: &Snac) -> std::io::Result<()> {
        self.send(FlapChannel::Data, snac.encode()).await
    }

    /// Reads exactly one complete FLAP frame off the wire, blocking (in the
    /// async sense) until it arrives. Returns `Ok(None)` on clean connection
    /// close, matching Tokio's usual EOF convention.
    pub async fn read_frame(&mut self) -> std::io::Result<Option<FlapFrame>> {
        read_frame(&mut self.stream).await
    }

    /// Splits into an owned reader/writer pair so each half can live on a
    /// different task — see the module doc comment for why this matters.
    pub fn into_split(self) -> (FlapReader, FlapWriter) {
        let (read_half, write_half) = self.stream.into_split();
        (FlapReader { read_half }, FlapWriter { write_half, send_sequence: self.send_sequence })
    }
}

pub struct FlapReader {
    read_half: OwnedReadHalf,
}

impl FlapReader {
    pub async fn read_frame(&mut self) -> std::io::Result<Option<FlapFrame>> {
        read_frame(&mut self.read_half).await
    }
}

pub struct FlapWriter {
    write_half: OwnedWriteHalf,
    send_sequence: u16,
}

impl FlapWriter {
    pub async fn send(&mut self, channel: FlapChannel, payload: Vec<u8>) -> std::io::Result<()> {
        send(&mut self.write_half, &mut self.send_sequence, channel, payload).await
    }

    pub async fn send_snac(&mut self, snac: &Snac) -> std::io::Result<()> {
        self.send(FlapChannel::Data, snac.encode()).await
    }
}
