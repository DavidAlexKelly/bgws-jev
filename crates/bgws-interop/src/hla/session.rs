//! The Federate Protocol session layer (IEEE 1516.1-2025 Federate Protocol;
//! wire format as in Pitch's FedProClient `session/`).
//!
//! Every message is a 24-octet big-endian header and a payload:
//!
//! | octets | field |
//! |---|---|
//! | 0-3 | packet size (header and payload) |
//! | 4-7 | sequence number |
//! | 8-15 | session ID |
//! | 16-19 | last sequence number received from the other side |
//! | 20-23 | message type |
//!
//! A session starts with New Session (the protocol version) and New Session
//! Status (the server's session ID in the header). The federate's messages
//! are numbered from 1 and kept until the server says it has them (the
//! header's last-received field), so after a dropped connection the
//! federate reconnects, asks to resume, and both sides send again what the
//! other missed; nothing is lost or seen twice. Heartbeats keep a quiet
//! session alive, and a server silent for the time-out counts as a drop.
//! HLA calls are matched to their responses by sequence number; callbacks
//! are answered as they arrive and handed on in order.

use std::collections::{BTreeMap, VecDeque};
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const HEADER_SIZE: usize = 24;
/// The Federate Protocol version this client speaks.
pub const PROTOCOL_VERSION: i32 = 1;
pub const NO_SESSION_ID: u64 = 0;
pub const INITIAL_SEQUENCE_NUMBER: i32 = 0;
pub const NO_SEQUENCE_NUMBER: i32 = i32::MIN;

/// New Session Status reasons.
pub const REASON_SUCCESS: i32 = 0;
pub const REASON_UNSUPPORTED_PROTOCOL_VERSION: i32 = 1;
/// Resume Status values.
pub const RESUME_OK: i32 = 0;
pub const RESUME_INVALID_SESSION: i32 = 1;
pub const RESUME_NOT_ENOUGH_BUFFERED: i32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Kind {
    NewSession = 1,
    NewSessionStatus = 2,
    Heartbeat = 3,
    HeartbeatResponse = 4,
    TerminateSession = 5,
    SessionTerminated = 6,
    ResumeRequest = 10,
    ResumeStatus = 11,
    HlaCallRequest = 20,
    HlaCallResponse = 21,
    HlaCallbackRequest = 22,
    HlaCallbackResponse = 23,
}

impl Kind {
    pub fn from_u32(v: u32) -> Option<Kind> {
        use Kind::*;
        Some(match v {
            1 => NewSession,
            2 => NewSessionStatus,
            3 => Heartbeat,
            4 => HeartbeatResponse,
            5 => TerminateSession,
            6 => SessionTerminated,
            10 => ResumeRequest,
            11 => ResumeStatus,
            20 => HlaCallRequest,
            21 => HlaCallResponse,
            22 => HlaCallbackRequest,
            23 => HlaCallbackResponse,
            _ => return None,
        })
    }
    /// HLA messages are the ones counted as received (and resent on resume).
    pub fn is_hla(self) -> bool {
        self as u32 >= 20
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub size: u32,
    pub seq: i32,
    pub session: u64,
    pub last_received: i32,
    pub kind: Kind,
}

impl Header {
    pub fn encode(&self) -> [u8; HEADER_SIZE] {
        let mut b = [0u8; HEADER_SIZE];
        b[0..4].copy_from_slice(&self.size.to_be_bytes());
        b[4..8].copy_from_slice(&self.seq.to_be_bytes());
        b[8..16].copy_from_slice(&self.session.to_be_bytes());
        b[16..20].copy_from_slice(&self.last_received.to_be_bytes());
        b[20..24].copy_from_slice(&(self.kind as u32).to_be_bytes());
        b
    }

    pub fn decode(b: &[u8; HEADER_SIZE]) -> io::Result<Header> {
        let u32_at = |i: usize| u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let size = u32_at(0);
        if (size as usize) < HEADER_SIZE {
            return Err(bad(format!("packet size {size}")));
        }
        let kind = Kind::from_u32(u32_at(20)).ok_or_else(|| bad(format!("message type {}", u32_at(20))))?;
        Ok(Header { size, seq: u32_at(4) as i32, session: u64::from_be_bytes(b[8..16].try_into().unwrap()), last_received: u32_at(16) as i32, kind })
    }
}

fn bad(what: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("bad Federate Protocol message: {what}"))
}

/// One message as sent: header and payload.
pub fn frame(seq: i32, session: u64, last_received: i32, kind: Kind, payload: &[u8]) -> Vec<u8> {
    let h = Header { size: (HEADER_SIZE + payload.len()) as u32, seq, session, last_received, kind };
    let mut out = Vec::with_capacity(HEADER_SIZE + payload.len());
    out.extend_from_slice(&h.encode());
    out.extend_from_slice(payload);
    out
}

/// Read one message.
pub fn read_frame(r: &mut impl Read) -> io::Result<(Header, Vec<u8>)> {
    let mut hb = [0u8; HEADER_SIZE];
    r.read_exact(&mut hb)?;
    let h = Header::decode(&hb)?;
    let mut payload = vec![0u8; h.size as usize - HEADER_SIZE];
    r.read_exact(&mut payload)?;
    Ok((h, payload))
}

/// The sequence number after `n` (wrapping to the initial one).
pub fn next_after(n: i32) -> i32 {
    if n == i32::MAX || n < INITIAL_SEQUENCE_NUMBER {
        INITIAL_SEQUENCE_NUMBER
    } else {
        n + 1
    }
}

/// Is `a` after `b`, allowing for wrap-around?
pub fn is_after(a: i32, b: i32) -> bool {
    if b == NO_SEQUENCE_NUMBER {
        return true;
    }
    let d = (a as i64 - b as i64).rem_euclid(1 << 31);
    d != 0 && d < (1 << 30)
}

pub fn be_i32(b: &[u8], at: usize) -> io::Result<i32> {
    b.get(at..at + 4).map(|s| i32::from_be_bytes(s.try_into().unwrap())).ok_or_else(|| bad("short payload".into()))
}

/// Session timings (FedProClient's `FED_INT_HEART`, `FED_TIMEOUT_HEART`,
/// `FED_TIMEOUT_RECONNECT`).
#[derive(Clone, Copy, Debug)]
pub struct SessionSettings {
    pub heartbeat: Duration,
    pub timeout: Duration,
    pub reconnect_limit: Duration,
    pub reconnect_delay: Duration,
    /// How long an HLA call may wait for its response.
    pub response_timeout: Duration,
}

impl Default for SessionSettings {
    fn default() -> Self {
        SessionSettings { heartbeat: Duration::from_secs(60), timeout: Duration::from_secs(180), reconnect_limit: Duration::from_secs(600), reconnect_delay: Duration::from_millis(500), response_timeout: Duration::from_secs(180) }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionError {
    /// The server refused or the network failed past every reconnection.
    Lost(String),
    /// No response in time.
    Timeout,
    Terminated,
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::Lost(why) => write!(f, "Federate Protocol session lost: {why}"),
            SessionError::Timeout => write!(f, "no response from the RTI in time"),
            SessionError::Terminated => write!(f, "the session is terminated"),
        }
    }
}

impl std::error::Error for SessionError {}

/// An HLA callback from the RTI (an encoded `CallbackRequest`).
#[derive(Clone, Debug)]
pub struct Callback {
    pub seq: i32,
    pub payload: Vec<u8>,
}

type Pending = Sender<Result<Vec<u8>, SessionError>>;

struct Writer {
    stream: TcpStream,
    next_seq: i32,
    /// Sent and not yet acknowledged: sequence number → whole message.
    history: VecDeque<(i32, Vec<u8>)>,
    last_sent: Instant,
}

struct Inner {
    addr: String,
    settings: SessionSettings,
    session: AtomicU64,
    writer: Mutex<Writer>,
    /// The last HLA message received from the server.
    last_received: AtomicI32,
    pending: Mutex<BTreeMap<i32, Pending>>,
    callbacks: Mutex<Option<Sender<Callback>>>,
    /// The answer to every callback (an encoded `CallbackResponse`).
    callback_ok: Vec<u8>,
    last_heard: Mutex<Instant>,
    closing: AtomicBool,
    lost: Mutex<Option<String>>,
    resumes: AtomicI32,
}

/// A Federate Protocol session with an RTI.
pub struct Session {
    inner: Arc<Inner>,
    callbacks: Mutex<Receiver<Callback>>,
}

impl Session {
    /// Open a session at `addr` (`host:port`). Each callback is answered
    /// with `callback_ok`.
    pub fn connect(addr: &str, settings: SessionSettings, callback_ok: Vec<u8>) -> Result<Session, SessionError> {
        let lost = |e: io::Error| SessionError::Lost(format!("{addr}: {e}"));
        let mut stream = TcpStream::connect(addr).map_err(lost)?;
        stream.set_nodelay(true).ok();
        let hello = frame(INITIAL_SEQUENCE_NUMBER, NO_SESSION_ID, NO_SEQUENCE_NUMBER, Kind::NewSession, &PROTOCOL_VERSION.to_be_bytes());
        stream.write_all(&hello).map_err(lost)?;
        stream.set_read_timeout(Some(settings.response_timeout)).ok();
        let (h, payload) = read_frame(&mut stream).map_err(lost)?;
        if h.kind != Kind::NewSessionStatus {
            return Err(SessionError::Lost(format!("expected New Session Status, got {:?}", h.kind)));
        }
        let reason = be_i32(&payload, 0).map_err(lost)?;
        if reason != REASON_SUCCESS {
            return Err(SessionError::Lost(format!("the server refused the session (reason {reason})")));
        }
        stream.set_read_timeout(None).ok();
        let (tx, rx) = mpsc::channel();
        let inner = Arc::new(Inner {
            addr: addr.to_string(),
            settings,
            session: AtomicU64::new(h.session),
            writer: Mutex::new(Writer { stream: stream.try_clone().map_err(lost)?, next_seq: next_after(INITIAL_SEQUENCE_NUMBER), history: VecDeque::new(), last_sent: Instant::now() }),
            last_received: AtomicI32::new(NO_SEQUENCE_NUMBER),
            pending: Mutex::new(BTreeMap::new()),
            callbacks: Mutex::new(Some(tx)),
            callback_ok,
            last_heard: Mutex::new(Instant::now()),
            closing: AtomicBool::new(false),
            lost: Mutex::new(None),
            resumes: AtomicI32::new(0),
        });
        let r = inner.clone();
        std::thread::Builder::new().name("fedpro-reader".into()).spawn(move || reader(r, stream)).map_err(lost)?;
        let hb = inner.clone();
        std::thread::Builder::new().name("fedpro-heartbeat".into()).spawn(move || heartbeat(hb)).map_err(lost)?;
        Ok(Session { inner, callbacks: Mutex::new(rx) })
    }

    pub fn id(&self) -> u64 {
        self.inner.session.load(Ordering::SeqCst)
    }

    /// How many times the session has been resumed after a drop.
    pub fn resumes(&self) -> i32 {
        self.inner.resumes.load(Ordering::SeqCst)
    }

    /// Send an HLA call (an encoded `CallRequest`); the response comes on the receiver.
    pub fn call_async(&self, request: &[u8]) -> Result<Receiver<Result<Vec<u8>, SessionError>>, SessionError> {
        self.inner.check()?;
        let (tx, rx) = mpsc::channel();
        let mut w = self.inner.writer.lock().unwrap();
        self.inner.pending.lock().unwrap().insert(w.next_seq, tx);
        self.inner.send_locked(&mut w, Kind::HlaCallRequest, request);
        Ok(rx)
    }

    /// An HLA call, waiting for its response (an encoded `CallResponse`).
    pub fn call(&self, request: &[u8]) -> Result<Vec<u8>, SessionError> {
        let rx = self.call_async(request)?;
        match rx.recv_timeout(self.inner.settings.response_timeout) {
            Ok(r) => r,
            Err(RecvTimeoutError::Timeout) => Err(SessionError::Timeout),
            Err(RecvTimeoutError::Disconnected) => Err(self.inner.check().err().unwrap_or(SessionError::Terminated)),
        }
    }

    /// Callbacks received so far, in order, without waiting.
    pub fn poll_callbacks(&self) -> Vec<Callback> {
        self.callbacks.lock().unwrap().try_iter().collect()
    }

    /// The next callback, waiting up to `timeout`.
    pub fn next_callback(&self, timeout: Duration) -> Option<Callback> {
        self.callbacks.lock().unwrap().recv_timeout(timeout).ok()
    }

    /// Cut the connection (as a network failure would); the session resumes on its own.
    pub fn drop_connection(&self) {
        let w = self.inner.writer.lock().unwrap();
        w.stream.shutdown(Shutdown::Both).ok();
    }

    /// End the session.
    pub fn terminate(&self) {
        if self.inner.closing.swap(true, Ordering::SeqCst) {
            return;
        }
        let mut w = self.inner.writer.lock().unwrap();
        self.inner.send_locked(&mut w, Kind::TerminateSession, &[]);
        std::thread::sleep(Duration::from_millis(20));
        w.stream.shutdown(Shutdown::Both).ok();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.terminate();
    }
}

impl Inner {
    fn check(&self) -> Result<(), SessionError> {
        if let Some(why) = self.lost.lock().unwrap().clone() {
            return Err(SessionError::Lost(why));
        }
        if self.closing.load(Ordering::SeqCst) {
            return Err(SessionError::Terminated);
        }
        Ok(())
    }

    /// Number, keep and write a message. A failed write is not an error
    /// here: the reader sees the drop, and resuming sends it again.
    fn send_locked(&self, w: &mut Writer, kind: Kind, payload: &[u8]) -> i32 {
        let seq = w.next_seq;
        w.next_seq = next_after(seq);
        let msg = frame(seq, self.session.load(Ordering::SeqCst), self.last_received.load(Ordering::SeqCst), kind, payload);
        w.stream.write_all(&msg).ok();
        w.history.push_back((seq, msg));
        w.last_sent = Instant::now();
        seq
    }

    fn acknowledged(&self, upto: i32) {
        if upto == NO_SEQUENCE_NUMBER {
            return;
        }
        let mut w = self.writer.lock().unwrap();
        while w.history.front().is_some_and(|(s, _)| !is_after(*s, upto)) {
            w.history.pop_front();
        }
    }

    fn fail(&self, why: String) {
        *self.lost.lock().unwrap() = Some(why.clone());
        for (_, p) in std::mem::take(&mut *self.pending.lock().unwrap()) {
            p.send(Err(SessionError::Lost(why.clone()))).ok();
        }
        self.callbacks.lock().unwrap().take();
    }

    /// Reconnect and resume; the new stream to read from.
    fn resume(&self) -> Option<TcpStream> {
        let started = Instant::now();
        while started.elapsed() < self.settings.reconnect_limit && !self.closing.load(Ordering::SeqCst) {
            if let Some(s) = self.try_resume() {
                self.resumes.fetch_add(1, Ordering::SeqCst);
                return Some(s);
            }
            std::thread::sleep(self.settings.reconnect_delay);
        }
        None
    }

    fn try_resume(&self) -> Option<TcpStream> {
        let mut s = TcpStream::connect(&self.addr).ok()?;
        s.set_nodelay(true).ok();
        let mut w = self.writer.lock().unwrap();
        let last = self.last_received.load(Ordering::SeqCst);
        let oldest = w.history.front().map(|(s, _)| *s).unwrap_or(w.next_seq);
        let mut payload = last.to_be_bytes().to_vec();
        payload.extend_from_slice(&oldest.to_be_bytes());
        s.write_all(&frame(NO_SEQUENCE_NUMBER, self.session.load(Ordering::SeqCst), last, Kind::ResumeRequest, &payload)).ok()?;
        s.set_read_timeout(Some(self.settings.response_timeout)).ok();
        let (h, p) = read_frame(&mut s).ok()?;
        s.set_read_timeout(None).ok();
        if h.kind != Kind::ResumeStatus || be_i32(&p, 0).ok()? != RESUME_OK {
            let why = format!("the server would not resume the session ({:?} {:?})", h.kind, be_i32(&p, 0).ok());
            drop(w);
            self.fail(why);
            return None;
        }
        let theirs = be_i32(&p, 4).ok()?;
        // Send again what the server missed.
        let again: Vec<Vec<u8>> = w.history.iter().filter(|(seq, _)| is_after(*seq, theirs)).map(|(_, m)| m.clone()).collect();
        for m in again {
            s.write_all(&m).ok()?;
        }
        w.stream = s.try_clone().ok()?;
        drop(w);
        *self.last_heard.lock().unwrap() = Instant::now();
        Some(s)
    }
}

fn reader(inner: Arc<Inner>, mut stream: TcpStream) {
    loop {
        match read_frame(&mut stream) {
            Ok((h, payload)) => {
                *inner.last_heard.lock().unwrap() = Instant::now();
                inner.acknowledged(h.last_received);
                if h.kind.is_hla() {
                    let last = inner.last_received.load(Ordering::SeqCst);
                    if !is_after(h.seq, last) {
                        continue; // Already had it (sent again after a resume).
                    }
                    inner.last_received.store(h.seq, Ordering::SeqCst);
                }
                match h.kind {
                    Kind::HlaCallResponse => {
                        let Ok(to) = be_i32(&payload, 0) else { continue };
                        if let Some(p) = inner.pending.lock().unwrap().remove(&to) {
                            p.send(Ok(payload[4..].to_vec())).ok();
                        }
                    }
                    Kind::HlaCallbackRequest => {
                        let mut answer = h.seq.to_be_bytes().to_vec();
                        answer.extend_from_slice(&inner.callback_ok);
                        {
                            let mut w = inner.writer.lock().unwrap();
                            inner.send_locked(&mut w, Kind::HlaCallbackResponse, &answer);
                        }
                        if let Some(tx) = inner.callbacks.lock().unwrap().as_ref() {
                            tx.send(Callback { seq: h.seq, payload }).ok();
                        }
                    }
                    Kind::SessionTerminated => {
                        inner.fail("terminated by the server".into());
                        return;
                    }
                    _ => {}
                }
            }
            Err(e) => {
                if inner.closing.load(Ordering::SeqCst) {
                    inner.fail("terminated".into());
                    return;
                }
                if inner.lost.lock().unwrap().is_some() {
                    return;
                }
                match inner.resume() {
                    Some(s) => stream = s,
                    None => {
                        if inner.lost.lock().unwrap().is_none() {
                            inner.fail(format!("connection dropped ({e}) and not resumed"));
                        }
                        return;
                    }
                }
            }
        }
    }
}

fn heartbeat(inner: Arc<Inner>) {
    let tick = (inner.settings.heartbeat / 4).max(Duration::from_millis(10));
    loop {
        std::thread::sleep(tick);
        if inner.closing.load(Ordering::SeqCst) || inner.lost.lock().unwrap().is_some() {
            return;
        }
        let quiet = inner.last_heard.lock().unwrap().elapsed();
        if quiet > inner.settings.timeout {
            // Silent too long: treat as a drop (the reader resumes).
            let w = inner.writer.lock().unwrap();
            w.stream.shutdown(Shutdown::Both).ok();
            drop(w);
            *inner.last_heard.lock().unwrap() = Instant::now();
            continue;
        }
        let mut w = inner.writer.lock().unwrap();
        if w.last_sent.elapsed() >= inner.settings.heartbeat {
            inner.send_locked(&mut w, Kind::Heartbeat, &[]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_is_big_endian_in_fedpro_order() {
        let h = Header { size: 28, seq: 1, session: 0x0102_0304_0506_0708, last_received: NO_SEQUENCE_NUMBER, kind: Kind::HlaCallRequest };
        let b = h.encode();
        assert_eq!(b, [0, 0, 0, 28, 0, 0, 0, 1, 1, 2, 3, 4, 5, 6, 7, 8, 0x80, 0, 0, 0, 0, 0, 0, 20]);
        assert_eq!(Header::decode(&b).unwrap(), h);
        let mut short = b;
        short[3] = 23;
        assert!(Header::decode(&short).is_err(), "a packet shorter than its header");
    }

    #[test]
    fn sequence_numbers_wrap() {
        assert_eq!(next_after(i32::MAX), 0);
        assert_eq!(next_after(NO_SEQUENCE_NUMBER), 0);
        assert!(is_after(0, i32::MAX));
        assert!(is_after(5, 4) && !is_after(4, 5) && !is_after(4, 4));
        assert!(is_after(0, NO_SEQUENCE_NUMBER));
    }
}
