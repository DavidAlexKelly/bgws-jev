//! A tiny HTTP/1.1 server for the client tests: no network, no extra crates.
//! Each connection carries one request; the handler sees the call number and
//! the request and says what to send back, and after how long.
#![allow(dead_code)]

use bgws_core::{DecisionOption, Question, QuestionId, QuestionKind, SimDuration, SimTime, Side};
use serde_json::Value;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Clone, Debug)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Value,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

pub struct Reply {
    pub status: u16,
    pub body: String,
    pub delay: Duration,
    pub headers: Vec<(String, String)>,
}

impl Reply {
    pub fn json(status: u16, body: Value) -> Reply {
        Reply { status, body: body.to_string(), delay: Duration::ZERO, headers: vec![] }
    }
    pub fn after(mut self, d: Duration) -> Reply {
        self.delay = d;
        self
    }
    pub fn header(mut self, k: &str, v: &str) -> Reply {
        self.headers.push((k.into(), v.into()));
        self
    }
}

pub struct Mock {
    pub url: String,
    pub seen: Arc<Mutex<Vec<Seen>>>,
    pub in_flight_max: Arc<AtomicUsize>,
}

impl Mock {
    pub fn calls(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
    pub fn request(&self, i: usize) -> Seen {
        self.seen.lock().unwrap()[i].clone()
    }
}

pub async fn serve<F>(handler: F) -> Mock
where
    F: Fn(usize, &Seen) -> Reply + Send + Sync + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let handler = Arc::new(handler);
    let in_flight = Arc::new(AtomicUsize::new(0));
    let in_flight_max = Arc::new(AtomicUsize::new(0));
    let (s2, m2) = (seen.clone(), in_flight_max.clone());
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let (seen, handler, in_flight, max) = (s2.clone(), handler.clone(), in_flight.clone(), m2.clone());
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                let head_end = loop {
                    let n = sock.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i + 4;
                    }
                };
                let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                let mut lines = head.split("\r\n");
                let first: Vec<&str> = lines.next().unwrap_or("").split(' ').collect();
                let headers: Vec<(String, String)> = lines.filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))).collect();
                let len: usize = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
                while buf.len() < head_end + len {
                    let n = sock.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                let body: Value = serde_json::from_slice(&buf[head_end..(head_end + len).min(buf.len())]).unwrap_or(Value::Null);
                let req = Seen { method: first.first().unwrap_or(&"").to_string(), path: first.get(1).unwrap_or(&"").to_string(), headers, body };
                let n = {
                    let mut s = seen.lock().unwrap();
                    s.push(req.clone());
                    s.len() - 1
                };
                let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                max.fetch_max(now, Ordering::SeqCst);
                let reply = handler(n, &req);
                tokio::time::sleep(reply.delay).await;
                in_flight.fetch_sub(1, Ordering::SeqCst);
                let mut out = format!("HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n", reply.status, reply.body.len());
                for (k, v) in &reply.headers {
                    out.push_str(&format!("{k}: {v}\r\n"));
                }
                out.push_str("\r\n");
                out.push_str(&reply.body);
                let _ = sock.write_all(out.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    Mock { url, seen, in_flight_max }
}

/// A client that never goes through a proxy.
pub fn http() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

pub fn opt(id: &str, action: &str, summary: &str, bias: i8) -> DecisionOption {
    DecisionOption { id: id.into(), action: action.into(), summary: summary.into(), bias, params: Default::default() }
}

/// The jevWorldDecider test's question: a call for fire, guns +1, hold −2, fallback hold.
pub fn world_question(id: u64) -> Question {
    Question {
        id: QuestionId(id),
        at: SimTime::ZERO,
        kind: QuestionKind::Entity,
        agent: "b1".into(),
        side: Side::Blue,
        trigger: "fireRequestReceived".into(),
        template: "t".into(),
        node: "w.fireRequestReceived".into(),
        title: "Call for fire".into(),
        question: "Which shooter answers?".into(),
        doctrine: vec!["Mass fires at the decisive point.".into()],
        situation: String::new(),
        options: vec![opt("guns", "fire", "the guns", 1), opt("hold", "hold", "hold fire", -2)],
        fallback: "hold".into(),
        rules_choice: "guns".into(),
        reaction: SimDuration::from_secs_i(30),
        plan_schema: None,
    }
}

pub fn land_question(id: u64, agent: &str) -> Question {
    Question {
        kind: QuestionKind::Unit,
        agent: agent.into(),
        trigger: "D1".into(),
        node: "D1".into(),
        title: "New contact".into(),
        question: "It has found an enemy it did not know about. What now?".into(),
        situation: format!("{agent} has run into the contact R3 at about 1.5 km to the east."),
        options: vec![opt("keep", "land.carryOn", "carry on", 0), opt("engage", "land.engageNow", "engage R3", 1), opt("pullBack", "land.pullBackToFriends", "pull back", 0)],
        fallback: "keep".into(),
        rules_choice: "engage".into(),
        ..world_question(id)
    }
}
