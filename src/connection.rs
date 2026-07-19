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

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::flap::{FlapChannel, FlapFrame, FLAP_HEADER_SIZE};
use crate::server_address::ServerAddress;
use crate::snac::Snac;

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
        self.send_sequence = self.send_sequence.wrapping_add(1);
        let frame = FlapFrame { channel, sequence: self.send_sequence, payload };
        self.stream.write_all(&frame.encode()).await
    }

    /// Convenience for sending a SNAC — wraps it as a channel-2 data frame.
    pub async fn send_snac(&mut self, snac: &Snac) -> std::io::Result<()> {
        self.send(FlapChannel::Data, snac.encode()).await
    }

    /// Reads exactly one complete FLAP frame off the wire, blocking (in the
    /// async sense) until it arrives. Returns `Ok(None)` on clean connection
    /// close, matching Tokio's usual EOF convention.
    pub async fn read_frame(&mut self) -> std::io::Result<Option<FlapFrame>> {
        let mut header = [0u8; FLAP_HEADER_SIZE];
        match self.stream.read_exact(&mut header).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }

        let (channel, sequence, length) = FlapFrame::parse_header(&header).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "malformed FLAP header (bad magic byte or unknown channel)")
        })?;

        let mut payload = vec![0u8; length as usize];
        if length > 0 {
            self.stream.read_exact(&mut payload).await?;
        }

        Ok(Some(FlapFrame { channel, sequence, payload }))
    }
}
