//! QF19w2: RFC 6455 framing for the WebSocket client, by hand (the
//! `tungstenite` crate is not in the offline registry, and a crate that wants
//! its own network access is banned by QNET1). Pure data in, data out: no I/O,
//! no threads. The bytes travel over wire's unix socket (see `js_ws.rs`).
//!
//! Client role only: frames we SEND are masked (a fresh random key each),
//! frames we RECEIVE must not be. The decoder rejects, with the close code the
//! RFC prescribes: reserved bits or unknown opcodes (1002), masked server
//! frames (1002), fragmented or over-long control frames (1002), a continuation
//! without a start or a new data frame inside a fragmented message (1002),
//! invalid UTF-8 in a text message or a close reason (1007), and a frame or
//! message over `MAX_MESSAGE` (1009).

/// Largest frame payload and largest reassembled message we accept.
pub(crate) const MAX_MESSAGE: usize = 16 * 1024 * 1024;

pub(crate) const OP_CONT: u8 = 0x0;
pub(crate) const OP_TEXT: u8 = 0x1;
pub(crate) const OP_BINARY: u8 = 0x2;
pub(crate) const OP_CLOSE: u8 = 0x8;
pub(crate) const OP_PING: u8 = 0x9;
pub(crate) const OP_PONG: u8 = 0xA;

// ------------------------------------------------------------ SHA-1 (RFC 3174)

/// SHA-1, only for `Sec-WebSocket-Accept` (not used for anything secret).
pub(crate) fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (i, c) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6u32),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 20];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

/// The `Sec-WebSocket-Accept` value for a `Sec-WebSocket-Key`.
pub(crate) fn accept_key(key: &str) -> String {
    use base64::Engine;
    let mut v = key.as_bytes().to_vec();
    v.extend_from_slice(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    base64::engine::general_purpose::STANDARD.encode(sha1(&v))
}

/// `n` bytes from the kernel CSPRNG (`getrandom(2)`); a failure leaves zeros
/// (a predictable mask is not a security issue for a client behind wire, and
/// the handshake key is then still accepted by any server).
pub(crate) fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    let mut off = 0;
    while off < N {
        // SAFETY: the pointer/length describe the unfilled tail of `buf`.
        let r = unsafe { libc::getrandom(buf[off..].as_mut_ptr().cast(), N - off, 0) };
        if r <= 0 {
            break;
        }
        off += r as usize;
    }
    buf
}

/// A fresh `Sec-WebSocket-Key` (16 random bytes, base64).
pub(crate) fn new_key() -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(random_bytes::<16>())
}

// ------------------------------------------------------------ encoding

/// One masked client frame.
pub(crate) fn encode_frame(fin: bool, opcode: u8, payload: &[u8], mask: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 14);
    out.push(if fin { 0x80 } else { 0 } | (opcode & 0x0F));
    let n = payload.len();
    if n <= 125 {
        out.push(0x80 | n as u8);
    } else if n <= 0xFFFF {
        out.push(0x80 | 126);
        out.extend_from_slice(&(n as u16).to_be_bytes());
    } else {
        out.push(0x80 | 127);
        out.extend_from_slice(&(n as u64).to_be_bytes());
    }
    out.extend_from_slice(&mask);
    out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i & 3]));
    out
}

/// The payload of a close frame: the code (big endian) and the reason, or
/// nothing at all without a code.
pub(crate) fn close_payload(code: Option<u16>, reason: &str) -> Vec<u8> {
    match code {
        None => Vec::new(),
        Some(c) => {
            let mut v = c.to_be_bytes().to_vec();
            v.extend_from_slice(reason.as_bytes());
            v
        }
    }
}

// ------------------------------------------------------------ decoding

/// A fatal violation: the connection must be failed with this close code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Violation {
    pub code: u16,
    pub msg: &'static str,
}

fn violation(code: u16, msg: &'static str) -> Violation {
    Violation { code, msg }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Frame {
    pub fin: bool,
    pub opcode: u8,
    pub payload: Vec<u8>,
}

/// Incremental frame parser: `push` what arrived, `next` until `None`.
#[derive(Default)]
pub(crate) struct FrameDecoder {
    buf: Vec<u8>,
}

impl FrameDecoder {
    pub(crate) fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next complete frame, `None` when more bytes are needed.
    pub(crate) fn next_frame(&mut self) -> Result<Option<Frame>, Violation> {
        let b = &self.buf;
        if b.len() < 2 {
            return Ok(None);
        }
        let (b0, b1) = (b[0], b[1]);
        if b0 & 0x70 != 0 {
            return Err(violation(1002, "reserved bits set"));
        }
        let opcode = b0 & 0x0F;
        if !matches!(opcode, OP_CONT | OP_TEXT | OP_BINARY | OP_CLOSE | OP_PING | OP_PONG) {
            return Err(violation(1002, "unknown opcode"));
        }
        if b1 & 0x80 != 0 {
            return Err(violation(1002, "a server frame must not be masked"));
        }
        let fin = b0 & 0x80 != 0;
        let (len, head) = match b1 & 0x7F {
            126 => {
                if b.len() < 4 {
                    return Ok(None);
                }
                (u16::from_be_bytes([b[2], b[3]]) as u64, 4)
            }
            127 => {
                if b.len() < 10 {
                    return Ok(None);
                }
                let v = u64::from_be_bytes([b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9]]);
                if v >> 63 != 0 {
                    return Err(violation(1002, "frame length has the top bit set"));
                }
                (v, 10)
            }
            n => (n as u64, 2),
        };
        if opcode >= 8 && (!fin || len > 125) {
            return Err(violation(1002, "a control frame must be final and at most 125 bytes"));
        }
        if len > MAX_MESSAGE as u64 {
            return Err(violation(1009, "frame too large"));
        }
        let len = len as usize;
        if b.len() < head + len {
            return Ok(None);
        }
        let payload = b[head..head + len].to_vec();
        self.buf.drain(..head + len);
        Ok(Some(Frame { fin, opcode, payload }))
    }
}

/// What a complete frame or message means to the connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Incoming {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    /// `None` code: the close frame carried no status.
    Close(Option<u16>, String),
}

/// Reassembles fragmented messages (control frames may arrive in between).
#[derive(Default)]
pub(crate) struct Assembler {
    kind: Option<u8>,
    data: Vec<u8>,
}

/// A close code a peer may send (RFC 6455 7.4): 1005/1006/1015 are reserved
/// for local use and never on the wire.
pub(crate) fn valid_wire_close_code(c: u16) -> bool {
    matches!(c, 1000..=1003 | 1007..=1011 | 3000..=4999)
}

impl Assembler {
    pub(crate) fn accept(&mut self, f: Frame) -> Result<Option<Incoming>, Violation> {
        match f.opcode {
            OP_PING => Ok(Some(Incoming::Ping(f.payload))),
            OP_PONG => Ok(Some(Incoming::Pong(f.payload))),
            OP_CLOSE => match f.payload.len() {
                0 => Ok(Some(Incoming::Close(None, String::new()))),
                1 => Err(violation(1002, "close frame with a 1-byte payload")),
                _ => {
                    let code = u16::from_be_bytes([f.payload[0], f.payload[1]]);
                    if !valid_wire_close_code(code) {
                        return Err(violation(1002, "invalid close code"));
                    }
                    let reason = String::from_utf8(f.payload[2..].to_vec()).map_err(|_| violation(1007, "close reason is not UTF-8"))?;
                    Ok(Some(Incoming::Close(Some(code), reason)))
                }
            },
            OP_CONT => {
                let Some(kind) = self.kind else { return Err(violation(1002, "continuation without a message")) };
                self.append(&f.payload)?;
                if f.fin { self.finish(kind) } else { Ok(None) }
            }
            kind => {
                if self.kind.is_some() {
                    return Err(violation(1002, "new message inside a fragmented one"));
                }
                if f.fin {
                    // The common case: no copy through the buffer.
                    self.kind = Some(kind);
                    self.data = f.payload;
                    return self.finish(kind);
                }
                self.kind = Some(kind);
                self.append(&f.payload)?;
                Ok(None)
            }
        }
    }

    fn append(&mut self, p: &[u8]) -> Result<(), Violation> {
        if self.data.len() + p.len() > MAX_MESSAGE {
            return Err(violation(1009, "message too large"));
        }
        self.data.extend_from_slice(p);
        Ok(())
    }

    fn finish(&mut self, kind: u8) -> Result<Option<Incoming>, Violation> {
        self.kind = None;
        let data = std::mem::take(&mut self.data);
        if kind == OP_TEXT {
            String::from_utf8(data).map(|s| Some(Incoming::Text(s))).map_err(|_| violation(1007, "text message is not UTF-8"))
        } else {
            Ok(Some(Incoming::Binary(data)))
        }
    }
}
