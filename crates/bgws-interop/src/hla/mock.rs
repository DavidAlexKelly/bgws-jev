//! The BGWS RTI: a Federate Protocol server with an RTI behind it, for tests,
//! for trying federates on one machine, and for small exercises
//! (`bgws mock-rti`).
//!
//! It speaks the session layer as [`super::session`] does (new session,
//! heartbeats, resume with nothing lost) and serves:
//! - federation management: connect, create, join, resign, destroy, list
//!   members, synchronization points;
//! - the FOM ([`super::fom`]): modules read at create and join, merged; names
//!   and handles of classes, attributes, interactions and parameters both
//!   ways; with no module, any name is taken (a permissive federation);
//! - declarations with the class hierarchy: an object is discovered as the
//!   nearest class its subscriber subscribes to, and reflected with the
//!   attributes subscribed there; an interaction reaches subscribers of its
//!   class or a superclass, as that class, with its parameters;
//! - objects: names reserved, registration, updates, deletion, requests for
//!   values, directed interactions;
//! - time: regulating and constrained, time-advance and next-message
//!   requests, timestamp order (by time, then sender's name, then sender's
//!   order, so a time-managed run is the same every time), query time;
//! - the management object model, in part: `HLAmanager.HLAfederation`
//!   (name, federates, time implementation) and `HLAmanager.HLAfederate`
//!   (handle, name, type, regulating, constrained, time, lookahead), kept up
//!   to date for any federate that subscribes.
//!
//! Not served: ownership transfer, regions, save and restore, retraction,
//! zero lookahead, unsubscribe and unpublish, MOM interactions and most MOM
//! attributes; those calls answer `RTIinternalError`.

use super::encoding::Enc;
use super::fom::{self, Fom, Module};
use super::proto::{self as p, call_request::CallRequest as Rq, call_response::CallResponse as Rs, callback_request::CallbackRequest as Cb};
use super::session::{self as s, frame, is_after, read_frame, Kind};
use prost::Message;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Write;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

/// A handle as the mock gives it out: eight octets, big-endian.
pub fn handle_bytes(h: u64) -> Vec<u8> {
    h.to_be_bytes().to_vec()
}

fn handle_of(b: &[u8]) -> u64 {
    let mut a = [0u8; 8];
    let n = b.len().min(8);
    a[8 - n..].copy_from_slice(&b[b.len() - n..]);
    u64::from_be_bytes(a)
}

/// HLAfloat64Time: an IEEE double, big-endian.
pub fn time_bytes(t: f64) -> Vec<u8> {
    t.to_be_bytes().to_vec()
}

fn time_of(b: &[u8]) -> f64 {
    b.try_into().map(f64::from_be_bytes).unwrap_or(0.0)
}

struct ServerSession {
    next_seq: i32,
    last_from_client: i32,
    history: VecDeque<(i32, Vec<u8>)>,
    stream: Option<TcpStream>,
    /// (federation, federate handle) once joined.
    joined: Option<(String, u64)>,
}

#[derive(Default)]
struct Federate {
    session: u64,
    name: String,
    kind: String,
    /// Its HLAmanager.HLAfederate object.
    mom: u64,
    /// Object class → attributes subscribed (FOM indices).
    subscribed: BTreeMap<usize, BTreeSet<usize>>,
    interactions: BTreeSet<usize>,
    /// Directed interactions subscribed: object class → interaction classes.
    directed: BTreeMap<usize, BTreeSet<usize>>,
    /// Objects discovered → the class it knows each by.
    known: BTreeMap<u64, usize>,
    /// Lookahead, if time-regulating.
    regulating: Option<f64>,
    constrained: bool,
    time: f64,
    /// A time-advance request: (time, asking for the next message).
    requested: Option<(f64, bool)>,
    /// Messages its time order holds back: (time, sender, sender's order) → callback.
    held: BTreeMap<(OrdF64, String, u64), Cb>,
    sent: u64,
}

#[derive(Clone, Copy, Debug)]
struct OrdF64(f64);
impl PartialEq for OrdF64 {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o).is_eq()
    }
}
impl Eq for OrdF64 {}
impl PartialOrd for OrdF64 {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for OrdF64 {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&o.0)
    }
}

struct Instance {
    class: usize,
    /// The owning federate (0: the RTI).
    owner: u64,
    name: String,
    /// The RTI's own objects keep their values, for new subscribers.
    values: BTreeMap<usize, Vec<u8>>,
}

#[derive(Default)]
struct Federation {
    fom: Fom,
    time_impl: String,
    /// Its HLAmanager.HLAfederation object.
    mom: u64,
    federates: BTreeMap<u64, Federate>,
    instances: BTreeMap<u64, Instance>,
    reserved: BTreeSet<String>,
    /// Synchronization points: label → (the federates in it, those that achieved it).
    sync: BTreeMap<String, (BTreeSet<u64>, BTreeSet<u64>)>,
}

#[derive(Default)]
struct State {
    sessions: BTreeMap<u64, ServerSession>,
    next_session: u64,
    next_handle: u64,
    federations: BTreeMap<String, Federation>,
    connections: u64,
    calls: u64,
    heartbeats: u64,
}

/// The mock server, listening until dropped.
pub struct MockRti {
    pub addr: SocketAddr,
    state: Arc<Mutex<State>>,
    listener: Option<TcpListener>,
}

impl MockRti {
    /// Listen on a free local port.
    pub fn start() -> std::io::Result<MockRti> {
        MockRti::bind("127.0.0.1:0")
    }

    pub fn bind(addr: &str) -> std::io::Result<MockRti> {
        let listener = TcpListener::bind(addr)?;
        let addr = listener.local_addr()?;
        let state = Arc::new(Mutex::new(State { next_session: 1, next_handle: 1, ..Default::default() }));
        let l = listener.try_clone()?;
        let st = state.clone();
        std::thread::Builder::new().name("mock-rti".into()).spawn(move || {
            for c in l.incoming() {
                let Ok(c) = c else { continue };
                let st = st.clone();
                std::thread::spawn(move || serve(st, c));
            }
        })?;
        Ok(MockRti { addr, state, listener: Some(listener) })
    }

    pub fn address(&self) -> String {
        self.addr.to_string()
    }

    /// Cut every client's connection, as a network failure would.
    pub fn drop_connections(&self) {
        let mut st = self.state.lock().unwrap();
        for s in st.sessions.values_mut() {
            if let Some(c) = s.stream.take() {
                c.shutdown(Shutdown::Both).ok();
            }
        }
    }

    /// TCP connections accepted so far (a resume is a new one).
    pub fn connections(&self) -> u64 {
        self.state.lock().unwrap().connections
    }

    pub fn heartbeats(&self) -> u64 {
        self.state.lock().unwrap().heartbeats
    }

    pub fn calls(&self) -> u64 {
        self.state.lock().unwrap().calls
    }
}

impl Drop for MockRti {
    fn drop(&mut self) {
        self.drop_connections();
        // Unblock the accept loop.
        if let Some(l) = self.listener.take() {
            TcpStream::connect(l.local_addr().unwrap()).ok();
        }
    }
}

impl State {
    /// Send a numbered message to a session (kept for resume if HLA).
    fn send(&mut self, session: u64, kind: Kind, payload: &[u8]) {
        let Some(ss) = self.sessions.get_mut(&session) else { return };
        let seq = ss.next_seq;
        ss.next_seq = s::next_after(seq);
        let msg = frame(seq, session, ss.last_from_client, kind, payload);
        if let Some(c) = ss.stream.as_mut() {
            if c.write_all(&msg).is_err() {
                ss.stream = None;
            }
        }
        if kind.is_hla() {
            ss.history.push_back((seq, msg));
        }
    }

    fn callback(&mut self, session: u64, cb: Cb) {
        let bytes = p::CallbackRequest { callback_request: Some(cb) }.encode_to_vec();
        self.send(session, Kind::HlaCallbackRequest, &bytes);
    }
}

fn exception(name: &str, details: impl Into<String>) -> Rs {
    Rs::ExceptionData(p::ExceptionData { exception_name: name.into(), details: details.into() })
}

fn serve(st: Arc<Mutex<State>>, mut c: TcpStream) {
    c.set_nodelay(true).ok();
    let Ok((h, payload)) = read_frame(&mut c) else { return };
    let session = {
        let mut g = st.lock().unwrap();
        g.connections += 1;
        match h.kind {
            Kind::NewSession => {
                let version = s::be_i32(&payload, 0).unwrap_or(-1);
                let reason = if version == s::PROTOCOL_VERSION { s::REASON_SUCCESS } else { s::REASON_UNSUPPORTED_PROTOCOL_VERSION };
                let id = if reason == s::REASON_SUCCESS {
                    let id = g.next_session;
                    g.next_session += 1;
                    id
                } else {
                    s::NO_SESSION_ID
                };
                let reply = frame(s::INITIAL_SEQUENCE_NUMBER, id, s::NO_SEQUENCE_NUMBER, Kind::NewSessionStatus, &reason.to_be_bytes());
                if c.write_all(&reply).is_err() || reason != s::REASON_SUCCESS {
                    return;
                }
                let Ok(w) = c.try_clone() else { return };
                g.sessions.insert(id, ServerSession { next_seq: 1, last_from_client: s::NO_SEQUENCE_NUMBER, history: VecDeque::new(), stream: Some(w), joined: None });
                id
            }
            Kind::ResumeRequest => {
                let client_has = s::be_i32(&payload, 0).unwrap_or(s::NO_SEQUENCE_NUMBER);
                let Some(ss) = g.sessions.get_mut(&h.session) else {
                    let mut p = s::RESUME_INVALID_SESSION.to_be_bytes().to_vec();
                    p.extend_from_slice(&s::NO_SEQUENCE_NUMBER.to_be_bytes());
                    c.write_all(&frame(s::NO_SEQUENCE_NUMBER, h.session, s::NO_SEQUENCE_NUMBER, Kind::ResumeStatus, &p)).ok();
                    return;
                };
                let mut p = s::RESUME_OK.to_be_bytes().to_vec();
                p.extend_from_slice(&ss.last_from_client.to_be_bytes());
                if c.write_all(&frame(s::NO_SEQUENCE_NUMBER, h.session, ss.last_from_client, Kind::ResumeStatus, &p)).is_err() {
                    return;
                }
                // What the client missed.
                for (seq, m) in &ss.history {
                    if is_after(*seq, client_has) && c.write_all(m).is_err() {
                        return;
                    }
                }
                ss.stream = c.try_clone().ok();
                h.session
            }
            _ => return,
        }
    };
    loop {
        let Ok((h, payload)) = read_frame(&mut c) else {
            // Dropped: keep the session for a resume.
            let mut g = st.lock().unwrap();
            if let Some(ss) = g.sessions.get_mut(&session) {
                if ss.stream.as_ref().is_some_and(|x| x.peer_addr().ok() == c.peer_addr().ok()) {
                    ss.stream = None;
                }
            }
            return;
        };
        let mut g = st.lock().unwrap();
        let Some(ss) = g.sessions.get_mut(&session) else { return };
        // The client has what we sent up to here.
        if h.last_received != s::NO_SEQUENCE_NUMBER {
            while ss.history.front().is_some_and(|(q, _)| !is_after(*q, h.last_received)) {
                ss.history.pop_front();
            }
        }
        if !is_after(h.seq, ss.last_from_client) {
            continue; // Sent again after a resume; already handled.
        }
        ss.last_from_client = h.seq;
        match h.kind {
            Kind::Heartbeat => {
                g.heartbeats += 1;
                g.send(session, Kind::HeartbeatResponse, &h.seq.to_be_bytes());
            }
            Kind::HlaCallRequest => {
                g.calls += 1;
                let rs = match p::CallRequest::decode(payload.as_slice()) {
                    Ok(p::CallRequest { call_request: Some(rq) }) => call(&mut g, session, rq),
                    _ => (exception("RTIinternalError", "undecodable call"), vec![]),
                };
                let mut out = h.seq.to_be_bytes().to_vec();
                out.extend_from_slice(&p::CallResponse { call_response: Some(rs.0) }.encode_to_vec());
                g.send(session, Kind::HlaCallResponse, &out);
                for (to, cb) in rs.1 {
                    g.callback(to, cb);
                }
                grant_all(&mut g);
            }
            Kind::TerminateSession => {
                for (to, cb) in resign(&mut g, session) {
                    g.callback(to, cb);
                }
                grant_all(&mut g);
                g.send(session, Kind::SessionTerminated, &[]);
                g.sessions.remove(&session);
                c.shutdown(Shutdown::Both).ok();
                return;
            }
            _ => {}
        }
    }
}

type Out = (Rs, Vec<(u64, Cb)>);
/// Attribute (or parameter) values by FOM index.
type Values = Vec<(usize, Vec<u8>)>;

// Handles: a kind in the high 32 bits, an index or number below.
const K_CLASS: u64 = 1 << 32;
const K_ATTR: u64 = 2 << 32;
const K_ICLASS: u64 = 3 << 32;
const K_PARAM: u64 = 4 << 32;
const K_INSTANCE: u64 = 5 << 32;
const K_FEDERATE: u64 = 6 << 32;
/// The RTI itself, owner of the management objects.
const RTI_OWNER: u64 = 0;

fn index_of(kind: u64, h: u64) -> Option<usize> {
    (h & !0xFFFF_FFFF == kind).then_some((h & 0xFFFF_FFFF) as usize)
}

fn joined(g: &State, session: u64) -> Option<(String, u64)> {
    g.sessions.get(&session).and_then(|s| s.joined.clone())
}

fn hb(h: &Option<impl HasData>) -> u64 {
    h.as_ref().map(|x| handle_of(x.data())).unwrap_or(0)
}

trait HasData {
    fn data(&self) -> &[u8];
}
macro_rules! has_data {
    ($($t:ty),*) => { $(impl HasData for $t { fn data(&self) -> &[u8] { &self.data } })* };
}
has_data!(p::ObjectClassHandle, p::AttributeHandle, p::ObjectInstanceHandle, p::InteractionClassHandle, p::ParameterHandle, p::LogicalTime, p::LogicalTimeInterval);

/// HLAreliable's handle as Federate Protocol clients know it (four octets,
/// 1; HLAbestEffort is 2): a real RTI names the transport of every
/// reflection and interaction, and clients refuse one without it.
pub const RELIABLE: [u8; 4] = [0, 0, 0, 1];

fn reliable() -> Option<p::TransportationTypeHandle> {
    Some(p::TransportationTypeHandle { data: RELIABLE.to_vec() })
}

fn och(c: usize) -> Option<p::ObjectClassHandle> {
    Some(p::ObjectClassHandle { data: handle_bytes(K_CLASS | c as u64) })
}

fn ich(c: usize) -> Option<p::InteractionClassHandle> {
    Some(p::InteractionClassHandle { data: handle_bytes(K_ICLASS | c as u64) })
}

fn inst_h(h: u64) -> Option<p::ObjectInstanceHandle> {
    Some(p::ObjectInstanceHandle { data: handle_bytes(h) })
}

fn fed_handle(h: u64) -> Option<p::FederateHandle> {
    Some(p::FederateHandle { data: handle_bytes(h) })
}

fn lt(t: f64) -> Option<p::LogicalTime> {
    Some(p::LogicalTime { data: time_bytes(t) })
}

fn values_of(m: Option<p::AttributeHandleValueMap>) -> Vec<(usize, Vec<u8>)> {
    m.map(|m| m.attribute_handle_value.into_iter().filter_map(|a| Some((index_of(K_ATTR, hb(&a.attribute_handle))?, a.value))).collect()).unwrap_or_default()
}

fn params_of(m: Option<p::ParameterHandleValueMap>) -> Vec<(usize, Vec<u8>)> {
    m.map(|m| m.parameter_handle_value.into_iter().filter_map(|a| Some((index_of(K_PARAM, hb(&a.parameter_handle))?, a.value))).collect()).unwrap_or_default()
}

fn attr_map(v: &[(usize, Vec<u8>)]) -> Option<p::AttributeHandleValueMap> {
    Some(p::AttributeHandleValueMap { attribute_handle_value: v.iter().map(|(a, v)| p::AttributeHandleValue { attribute_handle: Some(p::AttributeHandle { data: handle_bytes(K_ATTR | *a as u64) }), value: v.clone() }).collect() })
}

fn param_map(v: &[(usize, Vec<u8>)]) -> Option<p::ParameterHandleValueMap> {
    Some(p::ParameterHandleValueMap { parameter_handle_value: v.iter().map(|(a, v)| p::ParameterHandleValue { parameter_handle: Some(p::ParameterHandle { data: handle_bytes(K_PARAM | *a as u64) }), value: v.clone() }).collect() })
}

impl State {
    fn next(&mut self, kind: u64) -> u64 {
        let h = self.next_handle;
        self.next_handle += 1;
        kind | h
    }
}

/// The modules a create or join brings (file contents, or a file path as URL).
fn read_modules(set: Option<p::FomModuleSet>, one: Option<p::FomModule>) -> Result<Vec<Module>, Rs> {
    let mods: Vec<p::FomModule> = set.map(|s| s.fom_module).unwrap_or_default().into_iter().chain(one).collect();
    let mut out = vec![];
    for m in mods {
        let (name, text) = match m.fom_module {
            Some(p::fom_module::FomModule::File(f)) => (f.name, String::from_utf8_lossy(&f.content).into_owned()),
            Some(p::fom_module::FomModule::Url(u)) => {
                let path = u.strip_prefix("file://").or_else(|| u.strip_prefix("file:")).unwrap_or(&u).to_string();
                match std::fs::read_to_string(&path) {
                    Ok(t) => (u, t),
                    Err(e) => return Err(exception("CouldNotOpenFDD", format!("{u}: {e}"))),
                }
            }
            Some(p::fom_module::FomModule::CompressedModule(z)) => {
                // A zip (ISO/IEC 21320-1) holding the module's XML.
                let read = || -> Result<(String, String), String> {
                    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(z)).map_err(|e| e.to_string())?;
                    for i in 0..zip.len() {
                        let mut f = zip.by_index(i).map_err(|e| e.to_string())?;
                        if f.is_file() {
                            let mut text = String::new();
                            std::io::Read::read_to_string(&mut f, &mut text).map_err(|e| e.to_string())?;
                            return Ok((f.name().to_string(), text));
                        }
                    }
                    Err("an empty zip".into())
                };
                match read() {
                    Ok(x) => x,
                    Err(e) => return Err(exception("CouldNotOpenFDD", format!("a compressed module: {e}"))),
                }
            }
            None => continue,
        };
        out.push(Module::parse(&name, &text).map_err(|e| exception("ErrorReadingFDD", e.0))?);
    }
    Ok(out)
}

fn create(g: &mut State, name: String, modules: Vec<Module>, time_impl: &str) -> Result<(), Rs> {
    if g.federations.contains_key(&name) {
        return Err(exception("FederationExecutionAlreadyExists", name));
    }
    let fom = Fom::from_modules(modules).map_err(|e| exception("InconsistentFOM", e.0))?;
    let mom = g.next(K_INSTANCE);
    let mut f = Federation { fom, time_impl: if time_impl.is_empty() { "HLAfloat64Time".into() } else { time_impl.into() }, ..Default::default() };
    let class = f.fom.objects.class(fom::FEDERATION).unwrap();
    f.instances.insert(mom, Instance { class, owner: RTI_OWNER, name: format!("HLAfederation-{name}"), values: BTreeMap::new() });
    f.mom = mom;
    g.federations.insert(name, f);
    Ok(())
}

fn join(g: &mut State, session: u64, name: String, kind: String, federation: String, modules: Vec<Module>, wrap: impl Fn(p::JoinResult) -> Rs) -> Out {
    if joined(g, session).is_some() {
        return (exception("FederateAlreadyExecutionMember", federation), vec![]);
    }
    let h = g.next(K_FEDERATE);
    let mom = g.next(K_INSTANCE);
    let Some(f) = g.federations.get_mut(&federation) else { return (exception("FederationExecutionDoesNotExist", federation), vec![]) };
    if f.federates.values().any(|x| x.name == name) {
        return (exception("FederateNameAlreadyInUse", name), vec![]);
    }
    for m in modules {
        if let Err(e) = f.fom.merge(m) {
            return (exception("InconsistentFOM", e.0), vec![]);
        }
    }
    let class = f.fom.objects.class(fom::FEDERATE).unwrap();
    f.instances.insert(mom, Instance { class, owner: RTI_OWNER, name: format!("HLAfederate-{name}"), values: BTreeMap::new() });
    f.federates.insert(h, Federate { session, name, kind, mom, ..Default::default() });
    let time_impl = f.time_impl.clone();
    g.sessions.get_mut(&session).unwrap().joined = Some((federation.clone(), h));
    let out = mom_refresh(g, &federation);
    (wrap(p::JoinResult { federate_handle: fed_handle(h), logical_time_implementation_name: time_impl }), out)
}

fn resign(g: &mut State, session: u64) -> Vec<(u64, Cb)> {
    let Some((fx, me)) = joined(g, session) else { return vec![] };
    let mut out = vec![];
    if let Some(f) = g.federations.get_mut(&fx) {
        let mom = f.federates.get(&me).map(|x| x.mom);
        let gone: Vec<u64> = f.instances.iter().filter(|(&h, i)| i.owner == me || Some(h) == mom).map(|(&h, _)| h).collect();
        for h in gone {
            out.extend(delete_instance(f, h, me, vec![]));
        }
        f.federates.remove(&me);
    }
    if let Some(ss) = g.sessions.get_mut(&session) {
        ss.joined = None;
    }
    out.extend(mom_refresh(g, &fx));
    out
}

fn delete_instance(f: &mut Federation, h: u64, by: u64, tag: Vec<u8>) -> Vec<(u64, Cb)> {
    f.instances.remove(&h);
    let mut out = vec![];
    for fed in f.federates.values_mut() {
        if fed.known.remove(&h).is_some() {
            out.push((fed.session, Cb::RemoveObjectInstance(p::RemoveObjectInstance { object_instance: inst_h(h), user_supplied_tag: tag.clone(), producing_federate: fed_handle(by) })));
        }
    }
    out
}

/// The class a federate knows an object of `class` as: the nearest it
/// subscribes to, itself or a superclass.
fn known_class(fom: &Fom, fed: &Federate, class: usize) -> Option<usize> {
    fom.objects.lineage(class).into_iter().find(|c| fed.subscribed.contains_key(c))
}

/// Tell a federate of an object it has not yet discovered, if it now
/// subscribes to its class or a superclass. The RTI's own objects come with
/// their current values.
fn discover_for(f: &mut Federation, to: u64, h: u64) -> Vec<(u64, Cb)> {
    let Some(i) = f.instances.get(&h) else { return vec![] };
    let Some(fed) = f.federates.get(&to) else { return vec![] };
    if i.owner == to || fed.known.contains_key(&h) {
        return vec![];
    }
    let Some(known) = known_class(&f.fom, fed, i.class) else { return vec![] };
    let session = fed.session;
    let mut out = vec![(session, Cb::DiscoverObjectInstance(p::DiscoverObjectInstance { object_instance: inst_h(h), object_class: och(known), object_instance_name: i.name.clone(), producing_federate: fed_handle(i.owner) }))];
    if i.owner == RTI_OWNER {
        let wanted = &fed.subscribed[&known];
        let v: Vec<(usize, Vec<u8>)> = i.values.iter().filter(|(a, _)| wanted.contains(a)).map(|(a, v)| (*a, v.clone())).collect();
        if !v.is_empty() {
            out.push((
                session,
                Cb::ReflectAttributeValues(p::ReflectAttributeValues {
                    object_instance: inst_h(h),
                    attribute_values: attr_map(&v),
                    user_supplied_tag: vec![],
                    transportation_type: reliable(),
                    producing_federate: fed_handle(RTI_OWNER),
                    optional_sent_regions: None,
                }),
            ));
        }
    }
    f.federates.get_mut(&to).unwrap().known.insert(h, known);
    out
}

/// Deliver now, or hold for a time-constrained receiver until its grant.
fn deliver(f: &mut Federation, me: u64, to: u64, time: Option<f64>, now: Cb, timed: impl FnOnce() -> Cb) -> Option<(u64, Cb)> {
    let (regulating, order, sender) = match f.federates.get(&me) {
        Some(m) => (m.regulating.is_some(), m.sent, m.name.clone()),
        None => (false, 0, String::new()),
    };
    let r = f.federates.get_mut(&to)?;
    match time {
        Some(t) if regulating && r.constrained => {
            r.held.insert((OrdF64(t), sender, order), timed());
            None
        }
        Some(_) => Some((r.session, timed())),
        None => Some((r.session, now)),
    }
}

/// An update: to each federate that knows the object, the attributes it
/// subscribes to at the class it knows it by.
fn reflect(f: &mut Federation, me: u64, h: u64, values: Vec<(usize, Vec<u8>)>, tag: Vec<u8>, time: Option<f64>) -> Vec<(u64, Cb)> {
    if let Some(m) = f.federates.get_mut(&me) {
        m.sent += 1;
    }
    let targets: Vec<(u64, Values)> = f
        .federates
        .iter()
        .filter(|(&fh, _)| fh != me)
        .filter_map(|(&fh, fed)| {
            let known = fed.known.get(&h)?;
            let wanted = fed.subscribed.get(known)?;
            let v: Vec<(usize, Vec<u8>)> = values.iter().filter(|(a, _)| wanted.contains(a)).cloned().collect();
            (!v.is_empty()).then_some((fh, v))
        })
        .collect();
    let mut out = vec![];
    for (to, v) in targets {
        let now = Cb::ReflectAttributeValues(p::ReflectAttributeValues {
            object_instance: inst_h(h),
            attribute_values: attr_map(&v),
            user_supplied_tag: tag.clone(),
            transportation_type: reliable(),
            producing_federate: fed_handle(me),
            optional_sent_regions: None,
        });
        let timed = || {
            Cb::ReflectAttributeValuesWithTime(p::ReflectAttributeValuesWithTime {
                object_instance: inst_h(h),
                attribute_values: attr_map(&v),
                user_supplied_tag: tag.clone(),
                transportation_type: reliable(),
                producing_federate: fed_handle(me),
                optional_sent_regions: None,
                time: lt(time.unwrap_or(0.0)),
                sent_order_type: p::OrderType::Timestamp as i32,
                received_order_type: p::OrderType::Timestamp as i32,
                optional_retraction: None,
            })
        };
        out.extend(deliver(f, me, to, time, now, timed));
    }
    out
}

/// An interaction: to each federate subscribed to its class or a
/// superclass, as the nearest one it subscribes to, with that class's
/// parameters.
fn interaction(f: &mut Federation, me: u64, class: usize, params: Vec<(usize, Vec<u8>)>, tag: Vec<u8>, time: Option<f64>) -> Vec<(u64, Cb)> {
    if let Some(m) = f.federates.get_mut(&me) {
        m.sent += 1;
    }
    let lineage = f.fom.interactions.lineage(class);
    let targets: Vec<(u64, usize)> = f.federates.iter().filter(|(&fh, _)| fh != me).filter_map(|(&fh, fed)| lineage.iter().find(|c| fed.interactions.contains(c)).map(|&c| (fh, c))).collect();
    let mut out = vec![];
    for (to, as_class) in targets {
        let v: Vec<(usize, Vec<u8>)> = params.iter().filter(|(pm, _)| f.fom.interactions.has_member(as_class, *pm)).cloned().collect();
        let now = Cb::ReceiveInteraction(p::ReceiveInteraction {
            interaction_class: ich(as_class),
            parameter_values: param_map(&v),
            user_supplied_tag: tag.clone(),
            transportation_type: reliable(),
            producing_federate: fed_handle(me),
            optional_sent_regions: None,
        });
        let timed = || {
            Cb::ReceiveInteractionWithTime(p::ReceiveInteractionWithTime {
                interaction_class: ich(as_class),
                parameter_values: param_map(&v),
                user_supplied_tag: tag.clone(),
                transportation_type: reliable(),
                producing_federate: fed_handle(me),
                optional_sent_regions: None,
                time: lt(time.unwrap_or(0.0)),
                sent_order_type: p::OrderType::Timestamp as i32,
                received_order_type: p::OrderType::Timestamp as i32,
                optional_retraction: None,
            })
        };
        out.extend(deliver(f, me, to, time, now, timed));
    }
    out
}

// ---- the management object model ----

fn mom_handle(h: u64) -> Vec<u8> {
    let b = handle_bytes(h);
    Enc::new().u32(b.len() as u32).bytes(&b).done()
}

fn mom_bool(v: bool) -> Vec<u8> {
    Enc::new().i32(v as i32).done()
}

fn mom_time(t: f64) -> Vec<u8> {
    Enc::new().u32(8).bytes(&time_bytes(t)).done()
}

/// Bring the federation's management objects up to date, reflecting what
/// changed to their subscribers.
fn mom_refresh(g: &mut State, fx: &str) -> Vec<(u64, Cb)> {
    let Some(f) = g.federations.get_mut(fx) else { return vec![] };
    let a = |f: &Fom, class: &str, name: &str| f.objects.member_at(f.objects.class(class).unwrap(), name).unwrap();
    let mut want: Vec<(u64, Values)> = vec![];
    let mut list = Enc::new();
    list.u32(f.federates.len() as u32);
    for &h in f.federates.keys() {
        let b = handle_bytes(h);
        list.u32(b.len() as u32).bytes(&b);
    }
    want.push((
        f.mom,
        vec![
            (a(&f.fom, fom::FEDERATION, "HLAfederationName"), Enc::new().unicode(fx).done()),
            (a(&f.fom, fom::FEDERATION, "HLAfederatesInFederation"), list.done()),
            (a(&f.fom, fom::FEDERATION, "HLAtimeImplementationName"), Enc::new().unicode(&f.time_impl).done()),
        ],
    ));
    for (&h, fed) in &f.federates {
        let at = |n: &str| a(&f.fom, fom::FEDERATE, n);
        want.push((
            fed.mom,
            vec![
                (at("HLAfederateHandle"), mom_handle(h)),
                (at("HLAfederateName"), Enc::new().unicode(&fed.name).done()),
                (at("HLAfederateType"), Enc::new().unicode(&fed.kind).done()),
                (at("HLAtimeRegulating"), mom_bool(fed.regulating.is_some())),
                (at("HLAtimeConstrained"), mom_bool(fed.constrained)),
                (at("HLAlogicalTime"), mom_time(fed.time)),
                (at("HLAlookahead"), mom_time(fed.regulating.unwrap_or(0.0))),
            ],
        ));
    }
    let mut out = vec![];
    for (h, values) in want {
        let Some(i) = f.instances.get_mut(&h) else { continue };
        let changed: Vec<(usize, Vec<u8>)> = values.into_iter().filter(|(a, v)| i.values.get(a) != Some(v)).collect();
        if changed.is_empty() {
            continue;
        }
        for (a, v) in &changed {
            i.values.insert(*a, v.clone());
        }
        out.extend(reflect(f, RTI_OWNER, h, changed, vec![], None));
    }
    // New subscribers, or objects new to them.
    let feds: Vec<u64> = f.federates.keys().copied().collect();
    let moms: Vec<u64> = f.instances.iter().filter(|(_, i)| i.owner == RTI_OWNER).map(|(&h, _)| h).collect();
    for to in feds {
        for &h in &moms {
            out.extend(discover_for(f, to, h));
        }
    }
    out
}

fn call(g: &mut State, session: u64, rq: Rq) -> Out {
    let none = vec![];
    macro_rules! fed {
        () => {
            match joined(g, session) {
                Some(x) => x,
                None => return (exception("FederateNotExecutionMember", "join a federation first"), none),
            }
        };
    }
    macro_rules! fom_of {
        ($fx:expr) => {
            &mut g.federations.get_mut(&$fx).unwrap().fom
        };
    }
    macro_rules! class_of {
        ($fx:expr, $h:expr) => {
            match index_of(K_CLASS, hb(&$h)).filter(|&c| c < g.federations[&$fx].fom.objects.classes.len()) {
                Some(c) => c,
                None => return (exception("InvalidObjectClassHandle", format!("{:x}", hb(&$h))), none),
            }
        };
    }
    macro_rules! iclass_of {
        ($fx:expr, $h:expr) => {
            match index_of(K_ICLASS, hb(&$h)).filter(|&c| c < g.federations[&$fx].fom.interactions.classes.len()) {
                Some(c) => c,
                None => return (exception("InvalidInteractionClassHandle", format!("{:x}", hb(&$h))), none),
            }
        };
    }
    match rq {
        Rq::ConnectRequest(_) => (Rs::ConnectResponse(p::ConnectResponse { configuration_result: None }), none),
        Rq::ConnectWithConfigurationRequest(_) => (Rs::ConnectWithConfigurationResponse(p::ConnectWithConfigurationResponse { configuration_result: None }), none),
        Rq::DisconnectRequest(_) => (Rs::DisconnectResponse(p::DisconnectResponse {}), none),
        Rq::CreateFederationExecutionRequest(r) => match read_modules(None, r.fom_module).and_then(|m| create(g, r.federation_name, m, "")) {
            Ok(()) => (Rs::CreateFederationExecutionResponse(p::CreateFederationExecutionResponse {}), none),
            Err(e) => (e, none),
        },
        Rq::CreateFederationExecutionWithTimeRequest(r) => match read_modules(None, r.fom_module).and_then(|m| create(g, r.federation_name, m, &r.logical_time_implementation_name)) {
            Ok(()) => (Rs::CreateFederationExecutionWithTimeResponse(p::CreateFederationExecutionWithTimeResponse {}), none),
            Err(e) => (e, none),
        },
        Rq::CreateFederationExecutionWithModulesRequest(r) => match read_modules(r.fom_modules, None).and_then(|m| create(g, r.federation_name, m, "")) {
            Ok(()) => (Rs::CreateFederationExecutionWithModulesResponse(p::CreateFederationExecutionWithModulesResponse {}), none),
            Err(e) => (e, none),
        },
        Rq::CreateFederationExecutionWithModulesAndTimeRequest(r) => match read_modules(r.fom_modules, None).and_then(|m| create(g, r.federation_name, m, &r.logical_time_implementation_name)) {
            Ok(()) => (Rs::CreateFederationExecutionWithModulesAndTimeResponse(p::CreateFederationExecutionWithModulesAndTimeResponse {}), none),
            Err(e) => (e, none),
        },
        Rq::DestroyFederationExecutionRequest(r) => match g.federations.get(&r.federation_name) {
            None => (exception("FederationExecutionDoesNotExist", r.federation_name), none),
            Some(f) if !f.federates.is_empty() => (exception("FederatesCurrentlyJoined", r.federation_name), none),
            Some(_) => {
                g.federations.remove(&r.federation_name);
                (Rs::DestroyFederationExecutionResponse(p::DestroyFederationExecutionResponse {}), none)
            }
        },
        Rq::JoinFederationExecutionRequest(r) => join(g, session, format!("{}-{session}", r.federate_type), r.federate_type, r.federation_name, vec![], |h| Rs::JoinFederationExecutionResponse(p::JoinFederationExecutionResponse { result: Some(h) })),
        Rq::JoinFederationExecutionWithNameRequest(r) => join(g, session, r.federate_name, r.federate_type, r.federation_name, vec![], |h| Rs::JoinFederationExecutionWithNameResponse(p::JoinFederationExecutionWithNameResponse { result: Some(h) })),
        Rq::JoinFederationExecutionWithModulesRequest(r) => match read_modules(r.additional_fom_modules, None) {
            Ok(m) => join(g, session, format!("{}-{session}", r.federate_type), r.federate_type, r.federation_name, m, |h| Rs::JoinFederationExecutionWithModulesResponse(p::JoinFederationExecutionWithModulesResponse { result: Some(h) })),
            Err(e) => (e, none),
        },
        Rq::JoinFederationExecutionWithNameAndModulesRequest(r) => match read_modules(r.additional_fom_modules, None) {
            Ok(m) => join(g, session, r.federate_name, r.federate_type, r.federation_name, m, |h| Rs::JoinFederationExecutionWithNameAndModulesResponse(p::JoinFederationExecutionWithNameAndModulesResponse { result: Some(h) })),
            Err(e) => (e, none),
        },
        Rq::ResignFederationExecutionRequest(_) => {
            let out = resign(g, session);
            (Rs::ResignFederationExecutionResponse(p::ResignFederationExecutionResponse {}), out)
        }
        Rq::ListFederationExecutionMembersRequest(r) => {
            let cb = match g.federations.get(&r.federation_name) {
                Some(f) => Cb::ReportFederationExecutionMembers(p::ReportFederationExecutionMembers {
                    federation_name: r.federation_name.clone(),
                    report: Some(p::FederationExecutionMemberInformationSet {
                        federation_execution_member_information: f.federates.values().map(|x| p::FederationExecutionMemberInformation { federate_name: x.name.clone(), federate_type: x.kind.clone() }).collect(),
                    }),
                }),
                None => Cb::ReportFederationExecutionDoesNotExist(p::ReportFederationExecutionDoesNotExist { federation_name: r.federation_name.clone() }),
            };
            (Rs::ListFederationExecutionMembersResponse(p::ListFederationExecutionMembersResponse {}), vec![(session, cb)])
        }
        Rq::RegisterFederationSynchronizationPointRequest(r) => {
            let (fx, _) = fed!();
            let f = g.federations.get_mut(&fx).unwrap();
            let label = r.synchronization_point_label;
            let out = if f.sync.contains_key(&label) {
                vec![(session, Cb::SynchronizationPointRegistrationFailed(p::SynchronizationPointRegistrationFailed { synchronization_point_label: label, reason: p::SynchronizationPointFailureReason::SynchronizationPointLabelNotUnique as i32 }))]
            } else {
                let set: BTreeSet<u64> = f.federates.keys().copied().collect();
                let mut out = vec![(session, Cb::SynchronizationPointRegistrationSucceeded(p::SynchronizationPointRegistrationSucceeded { synchronization_point_label: label.clone() }))];
                for h in &set {
                    out.push((f.federates[h].session, Cb::AnnounceSynchronizationPoint(p::AnnounceSynchronizationPoint { synchronization_point_label: label.clone(), user_supplied_tag: r.user_supplied_tag.clone() })));
                }
                f.sync.insert(label, (set, BTreeSet::new()));
                out
            };
            (Rs::RegisterFederationSynchronizationPointResponse(p::RegisterFederationSynchronizationPointResponse {}), out)
        }
        Rq::SynchronizationPointAchievedRequest(r) => {
            let (fx, me) = fed!();
            let f = g.federations.get_mut(&fx).unwrap();
            let label = r.synchronization_point_label;
            let mut out = vec![];
            if let Some((set, done)) = f.sync.get_mut(&label) {
                done.insert(me);
                if set.iter().all(|h| done.contains(h) || !f.federates.contains_key(h)) {
                    let set = f.sync.remove(&label).unwrap().0;
                    for h in set.iter().filter(|h| f.federates.contains_key(h)) {
                        out.push((f.federates[h].session, Cb::FederationSynchronized(p::FederationSynchronized { synchronization_point_label: label.clone(), failed_to_sync_set: None })));
                    }
                }
            }
            (Rs::SynchronizationPointAchievedResponse(p::SynchronizationPointAchievedResponse {}), out)
        }
        // ---- names and handles ----
        Rq::GetObjectClassHandleRequest(r) => {
            let (fx, _) = fed!();
            match fom_of!(fx).object_class(&r.object_class_name) {
                Some(c) => (Rs::GetObjectClassHandleResponse(p::GetObjectClassHandleResponse { result: och(c) }), none),
                None => (exception("NameNotFound", r.object_class_name), none),
            }
        }
        Rq::GetObjectClassNameRequest(r) => {
            let (fx, _) = fed!();
            let c = class_of!(fx, r.object_class);
            (Rs::GetObjectClassNameResponse(p::GetObjectClassNameResponse { result: g.federations[&fx].fom.objects.classes[c].name.clone() }), none)
        }
        Rq::GetAttributeHandleRequest(r) => {
            let (fx, _) = fed!();
            let c = class_of!(fx, r.object_class);
            match fom_of!(fx).attribute(c, &r.attribute_name) {
                Some(a) => (Rs::GetAttributeHandleResponse(p::GetAttributeHandleResponse { result: Some(p::AttributeHandle { data: handle_bytes(K_ATTR | a as u64) }) }), none),
                None => (exception("NameNotFound", r.attribute_name), none),
            }
        }
        Rq::GetAttributeNameRequest(r) => {
            let (fx, _) = fed!();
            let c = class_of!(fx, r.object_class);
            let t = &g.federations[&fx].fom.objects;
            match index_of(K_ATTR, hb(&r.attribute)).filter(|&a| t.has_member(c, a)) {
                Some(a) => (Rs::GetAttributeNameResponse(p::GetAttributeNameResponse { result: t.members[a].1.clone() }), none),
                None => (exception("AttributeNotDefined", format!("{:x}", hb(&r.attribute))), none),
            }
        }
        Rq::GetInteractionClassHandleRequest(r) => {
            let (fx, _) = fed!();
            match fom_of!(fx).interaction_class(&r.interaction_class_name) {
                Some(c) => (Rs::GetInteractionClassHandleResponse(p::GetInteractionClassHandleResponse { result: ich(c) }), none),
                None => (exception("NameNotFound", r.interaction_class_name), none),
            }
        }
        Rq::GetInteractionClassNameRequest(r) => {
            let (fx, _) = fed!();
            let c = iclass_of!(fx, r.interaction_class);
            (Rs::GetInteractionClassNameResponse(p::GetInteractionClassNameResponse { result: g.federations[&fx].fom.interactions.classes[c].name.clone() }), none)
        }
        Rq::GetParameterHandleRequest(r) => {
            let (fx, _) = fed!();
            let c = iclass_of!(fx, r.interaction_class);
            match fom_of!(fx).parameter(c, &r.parameter_name) {
                Some(a) => (Rs::GetParameterHandleResponse(p::GetParameterHandleResponse { result: Some(p::ParameterHandle { data: handle_bytes(K_PARAM | a as u64) }) }), none),
                None => (exception("NameNotFound", r.parameter_name), none),
            }
        }
        Rq::GetParameterNameRequest(r) => {
            let (fx, _) = fed!();
            let c = iclass_of!(fx, r.interaction_class);
            let t = &g.federations[&fx].fom.interactions;
            match index_of(K_PARAM, hb(&r.parameter)).filter(|&a| t.has_member(c, a)) {
                Some(a) => (Rs::GetParameterNameResponse(p::GetParameterNameResponse { result: t.members[a].1.clone() }), none),
                None => (exception("InteractionParameterNotDefined", format!("{:x}", hb(&r.parameter))), none),
            }
        }
        Rq::GetKnownObjectClassHandleRequest(r) => {
            let (fx, me) = fed!();
            let f = &g.federations[&fx];
            let h = hb(&r.object_instance);
            let known = f.federates[&me].known.get(&h).copied().or_else(|| f.instances.get(&h).filter(|i| i.owner == me).map(|i| i.class));
            match known {
                Some(c) => (Rs::GetKnownObjectClassHandleResponse(p::GetKnownObjectClassHandleResponse { result: och(c) }), none),
                None => (exception("ObjectInstanceNotKnown", format!("{h:x}")), none),
            }
        }
        Rq::GetObjectInstanceNameRequest(r) => {
            let (fx, _) = fed!();
            match g.federations[&fx].instances.get(&hb(&r.object_instance)) {
                Some(i) => (Rs::GetObjectInstanceNameResponse(p::GetObjectInstanceNameResponse { result: i.name.clone() }), none),
                None => (exception("ObjectInstanceNotKnown", format!("{:x}", hb(&r.object_instance))), none),
            }
        }
        Rq::GetObjectInstanceHandleRequest(r) => {
            let (fx, _) = fed!();
            match g.federations[&fx].instances.iter().find(|(_, i)| i.name == r.object_instance_name) {
                Some((&h, _)) => (Rs::GetObjectInstanceHandleResponse(p::GetObjectInstanceHandleResponse { result: inst_h(h) }), none),
                None => (exception("ObjectInstanceNotKnown", r.object_instance_name), none),
            }
        }
        // ---- declarations ----
        Rq::PublishObjectClassAttributesRequest(r) => {
            let (fx, _) = fed!();
            class_of!(fx, r.object_class);
            (Rs::PublishObjectClassAttributesResponse(p::PublishObjectClassAttributesResponse {}), none)
        }
        Rq::PublishInteractionClassRequest(r) => {
            let (fx, _) = fed!();
            iclass_of!(fx, r.interaction_class);
            (Rs::PublishInteractionClassResponse(p::PublishInteractionClassResponse {}), none)
        }
        Rq::SubscribeObjectClassAttributesRequest(r) => {
            let (fx, me) = fed!();
            let class = class_of!(fx, r.object_class);
            let attrs: BTreeSet<usize> = r.attributes.map(|a| a.attribute_handle.iter().filter_map(|x| index_of(K_ATTR, handle_of(&x.data))).collect()).unwrap_or_default();
            let f = g.federations.get_mut(&fx).unwrap();
            if let Some(bad) = attrs.iter().find(|&&a| !f.fom.objects.has_member(class, a)) {
                return (exception("AttributeNotDefined", format!("{bad}")), none);
            }
            f.federates.get_mut(&me).unwrap().subscribed.entry(class).or_default().extend(attrs);
            let all: Vec<u64> = f.instances.keys().copied().collect();
            let mut out = vec![];
            for h in all {
                out.extend(discover_for(f, me, h));
            }
            (Rs::SubscribeObjectClassAttributesResponse(p::SubscribeObjectClassAttributesResponse {}), out)
        }
        Rq::SubscribeInteractionClassRequest(r) => {
            let (fx, me) = fed!();
            let c = iclass_of!(fx, r.interaction_class);
            g.federations.get_mut(&fx).unwrap().federates.get_mut(&me).unwrap().interactions.insert(c);
            (Rs::SubscribeInteractionClassResponse(p::SubscribeInteractionClassResponse {}), none)
        }
        Rq::PublishObjectClassDirectedInteractionsRequest(r) => {
            let (fx, _) = fed!();
            class_of!(fx, r.object_class);
            (Rs::PublishObjectClassDirectedInteractionsResponse(p::PublishObjectClassDirectedInteractionsResponse {}), none)
        }
        Rq::SubscribeObjectClassDirectedInteractionsRequest(r) => {
            let (fx, me) = fed!();
            let class = class_of!(fx, r.object_class);
            let set: BTreeSet<usize> = r.interaction_classes.map(|s| s.interaction_class_handle.iter().filter_map(|h| index_of(K_ICLASS, handle_of(&h.data))).collect()).unwrap_or_default();
            g.federations.get_mut(&fx).unwrap().federates.get_mut(&me).unwrap().directed.entry(class).or_default().extend(set);
            (Rs::SubscribeObjectClassDirectedInteractionsResponse(p::SubscribeObjectClassDirectedInteractionsResponse {}), none)
        }
        // ---- objects ----
        Rq::ReserveObjectInstanceNameRequest(r) => {
            let (fx, _) = fed!();
            let f = g.federations.get_mut(&fx).unwrap();
            let cb = if r.object_instance_name.starts_with("HLA") || f.instances.values().any(|i| i.name == r.object_instance_name) || !f.reserved.insert(r.object_instance_name.clone()) {
                Cb::ObjectInstanceNameReservationFailed(p::ObjectInstanceNameReservationFailed { object_instance_name: r.object_instance_name })
            } else {
                Cb::ObjectInstanceNameReservationSucceeded(p::ObjectInstanceNameReservationSucceeded { object_instance_name: r.object_instance_name })
            };
            (Rs::ReserveObjectInstanceNameResponse(p::ReserveObjectInstanceNameResponse {}), vec![(session, cb)])
        }
        Rq::RegisterObjectInstanceRequest(r) => {
            let (fx, me) = fed!();
            let class = class_of!(fx, r.object_class);
            let h = g.next(K_INSTANCE);
            let f = g.federations.get_mut(&fx).unwrap();
            f.instances.insert(h, Instance { class, owner: me, name: format!("HLAobjectRoot-{}", h & 0xFFFF_FFFF), values: BTreeMap::new() });
            let feds: Vec<u64> = f.federates.keys().copied().collect();
            let out = feds.into_iter().flat_map(|to| discover_for(f, to, h)).collect();
            (Rs::RegisterObjectInstanceResponse(p::RegisterObjectInstanceResponse { result: inst_h(h) }), out)
        }
        Rq::RegisterObjectInstanceWithNameRequest(r) => {
            let (fx, me) = fed!();
            let class = class_of!(fx, r.object_class);
            let h = g.next(K_INSTANCE);
            let f = g.federations.get_mut(&fx).unwrap();
            if !f.reserved.remove(&r.object_instance_name) {
                return (exception("ObjectInstanceNameNotReserved", r.object_instance_name), none);
            }
            f.instances.insert(h, Instance { class, owner: me, name: r.object_instance_name, values: BTreeMap::new() });
            let feds: Vec<u64> = f.federates.keys().copied().collect();
            let out = feds.into_iter().flat_map(|to| discover_for(f, to, h)).collect();
            (Rs::RegisterObjectInstanceWithNameResponse(p::RegisterObjectInstanceWithNameResponse { result: inst_h(h) }), out)
        }
        Rq::DeleteObjectInstanceRequest(r) => {
            let (fx, me) = fed!();
            let h = hb(&r.object_instance);
            let f = g.federations.get_mut(&fx).unwrap();
            match f.instances.get(&h) {
                None => (exception("ObjectInstanceNotKnown", format!("{h:x}")), none),
                Some(i) if i.owner != me => (exception("DeletePrivilegeNotHeld", format!("{h:x}")), none),
                Some(_) => {
                    let out = delete_instance(f, h, me, r.user_supplied_tag);
                    (Rs::DeleteObjectInstanceResponse(p::DeleteObjectInstanceResponse {}), out)
                }
            }
        }
        Rq::UpdateAttributeValuesRequest(r) => {
            let (fx, me) = fed!();
            let h = hb(&r.object_instance);
            let f = g.federations.get_mut(&fx).unwrap();
            if f.instances.get(&h).is_none_or(|i| i.owner != me) {
                return (exception("ObjectInstanceNotKnown", format!("{h:x}")), none);
            }
            let out = reflect(f, me, h, values_of(r.attribute_values), r.user_supplied_tag, None);
            (Rs::UpdateAttributeValuesResponse(p::UpdateAttributeValuesResponse {}), out)
        }
        Rq::UpdateAttributeValuesWithTimeRequest(r) => {
            let (fx, me) = fed!();
            let h = hb(&r.object_instance);
            let f = g.federations.get_mut(&fx).unwrap();
            if f.instances.get(&h).is_none_or(|i| i.owner != me) {
                return (exception("ObjectInstanceNotKnown", format!("{h:x}")), none);
            }
            let out = reflect(f, me, h, values_of(r.attribute_values), r.user_supplied_tag, Some(time_of(&r.time.unwrap_or_default().data)));
            (Rs::UpdateAttributeValuesWithTimeResponse(p::UpdateAttributeValuesWithTimeResponse { result: Some(p::MessageRetractionReturn { retraction_handle_is_valid: false, message_retraction_handle: None }) }), out)
        }
        Rq::RequestInstanceAttributeValueUpdateRequest(r) => {
            let (fx, me) = fed!();
            let h = hb(&r.object_instance);
            let f = g.federations.get_mut(&fx).unwrap();
            let Some(i) = f.instances.get(&h) else { return (exception("ObjectInstanceNotKnown", format!("{h:x}")), none) };
            let out = if i.owner == RTI_OWNER {
                let v: Vec<(usize, Vec<u8>)> = i.values.iter().map(|(a, v)| (*a, v.clone())).collect();
                reflect(f, RTI_OWNER, h, v, vec![], None).into_iter().filter(|(s, _)| *s == session).collect()
            } else {
                let _ = me;
                f.federates
                    .get(&i.owner)
                    .map(|o| (o.session, Cb::ProvideAttributeValueUpdate(p::ProvideAttributeValueUpdate { object_instance: r.object_instance.clone(), attributes: r.attributes.clone(), user_supplied_tag: r.user_supplied_tag.clone() })))
                    .into_iter()
                    .collect()
            };
            (Rs::RequestInstanceAttributeValueUpdateResponse(p::RequestInstanceAttributeValueUpdateResponse {}), out)
        }
        Rq::RequestClassAttributeValueUpdateRequest(r) => {
            let (fx, me) = fed!();
            let class = class_of!(fx, r.object_class);
            let f = &g.federations[&fx];
            let out = f
                .instances
                .iter()
                .filter(|(_, i)| f.fom.objects.is_a(i.class, class) && i.owner != me && i.owner != RTI_OWNER)
                .filter_map(|(&h, i)| {
                    f.federates.get(&i.owner).map(|o| (o.session, Cb::ProvideAttributeValueUpdate(p::ProvideAttributeValueUpdate { object_instance: inst_h(h), attributes: r.attributes.clone(), user_supplied_tag: r.user_supplied_tag.clone() })))
                })
                .collect();
            (Rs::RequestClassAttributeValueUpdateResponse(p::RequestClassAttributeValueUpdateResponse {}), out)
        }
        // ---- interactions ----
        Rq::SendInteractionRequest(r) => {
            let (fx, me) = fed!();
            let c = iclass_of!(fx, r.interaction_class);
            let out = interaction(g.federations.get_mut(&fx).unwrap(), me, c, params_of(r.parameter_values), r.user_supplied_tag, None);
            (Rs::SendInteractionResponse(p::SendInteractionResponse {}), out)
        }
        Rq::SendInteractionWithTimeRequest(r) => {
            let (fx, me) = fed!();
            let c = iclass_of!(fx, r.interaction_class);
            let out = interaction(g.federations.get_mut(&fx).unwrap(), me, c, params_of(r.parameter_values), r.user_supplied_tag, Some(time_of(&r.time.unwrap_or_default().data)));
            (Rs::SendInteractionWithTimeResponse(p::SendInteractionWithTimeResponse { result: Some(p::MessageRetractionReturn { retraction_handle_is_valid: false, message_retraction_handle: None }) }), out)
        }
        Rq::SendDirectedInteractionRequest(r) => {
            let (fx, me) = fed!();
            let ic = iclass_of!(fx, r.interaction_class);
            let f = &g.federations[&fx];
            let Some(class) = f.instances.get(&hb(&r.object_instance)).map(|i| i.class) else { return (exception("ObjectInstanceNotKnown", format!("{:x}", hb(&r.object_instance))), none) };
            let classes = f.fom.objects.lineage(class);
            let interactions = f.fom.interactions.lineage(ic);
            // To every other federate subscribed to it (or a superclass) at
            // the object's class (or a superclass).
            let out = f
                .federates
                .iter()
                .filter(|(&fh, _)| fh != me)
                .filter_map(|(_, fed)| {
                    let as_class = classes.iter().find_map(|c| fed.directed.get(c).and_then(|set| interactions.iter().find(|i| set.contains(i))))?;
                    let v: Vec<(usize, Vec<u8>)> = params_of(r.parameter_values.clone()).into_iter().filter(|(pm, _)| f.fom.interactions.has_member(*as_class, *pm)).collect();
                    Some((
                        fed.session,
                        Cb::ReceiveDirectedInteraction(p::ReceiveDirectedInteraction {
                            interaction_class: ich(*as_class),
                            object_instance: r.object_instance.clone(),
                            parameter_values: param_map(&v),
                            user_supplied_tag: r.user_supplied_tag.clone(),
                            transportation_type: reliable(),
                            producing_federate: fed_handle(me),
                        }),
                    ))
                })
                .collect();
            (Rs::SendDirectedInteractionResponse(p::SendDirectedInteractionResponse {}), out)
        }
        // ---- time ----
        Rq::EnableTimeRegulationRequest(r) => {
            let (fx, me) = fed!();
            let f = g.federations.get_mut(&fx).unwrap().federates.get_mut(&me).unwrap();
            f.regulating = Some(time_of(&r.lookahead.unwrap_or_default().data));
            let mut out = vec![(session, Cb::TimeRegulationEnabled(p::TimeRegulationEnabled { time: lt(f.time) }))];
            out.extend(mom_refresh(g, &fx));
            (Rs::EnableTimeRegulationResponse(p::EnableTimeRegulationResponse {}), out)
        }
        Rq::EnableTimeConstrainedRequest(_) => {
            let (fx, me) = fed!();
            let f = g.federations.get_mut(&fx).unwrap().federates.get_mut(&me).unwrap();
            f.constrained = true;
            let mut out = vec![(session, Cb::TimeConstrainedEnabled(p::TimeConstrainedEnabled { time: lt(f.time) }))];
            out.extend(mom_refresh(g, &fx));
            (Rs::EnableTimeConstrainedResponse(p::EnableTimeConstrainedResponse {}), out)
        }
        Rq::TimeAdvanceRequestRequest(r) => {
            let (fx, me) = fed!();
            g.federations.get_mut(&fx).unwrap().federates.get_mut(&me).unwrap().requested = Some((time_of(&r.time.unwrap_or_default().data), false));
            (Rs::TimeAdvanceRequestResponse(p::TimeAdvanceRequestResponse {}), none)
        }
        Rq::NextMessageRequestRequest(r) => {
            let (fx, me) = fed!();
            g.federations.get_mut(&fx).unwrap().federates.get_mut(&me).unwrap().requested = Some((time_of(&r.time.unwrap_or_default().data), true));
            (Rs::NextMessageRequestResponse(p::NextMessageRequestResponse {}), none)
        }
        Rq::QueryLogicalTimeRequest(_) => {
            let (fx, me) = fed!();
            (Rs::QueryLogicalTimeResponse(p::QueryLogicalTimeResponse { result: lt(g.federations[&fx].federates[&me].time) }), none)
        }
        _ => (exception("RTIinternalError", "this service is not in the BGWS RTI"), none),
    }
}

impl Federate {
    /// The earliest message it holds.
    fn next_held(&self) -> Option<f64> {
        self.held.keys().next().map(|k| k.0 .0)
    }

    /// No message it sends from now on will be stamped earlier than this:
    /// its time, its time-advance request, or (asking for the next message)
    /// the earlier of that and its next held message, plus its lookahead.
    fn promise(&self) -> Option<f64> {
        let l = self.regulating?;
        let from = match self.requested {
            Some((t, false)) => t,
            Some((t, true)) => self.next_held().map_or(t, |m| m.min(t)),
            None => self.time,
        };
        Some(from.max(self.time) + l)
    }
}

/// Grant every time advance that can be: a constrained federate asking for
/// t (or, asking for the next message, for its earliest held message if
/// sooner) is granted once no other regulating federate can still send a
/// message stamped then or earlier. Held messages up to the grant go first,
/// in order. Lookahead must be above zero.
fn grant_all(g: &mut State) {
    let mut out = vec![];
    let mut changed = vec![];
    for (fx, f) in g.federations.iter_mut() {
        loop {
            let mut granted = None;
            for (&h, fed) in &f.federates {
                let Some((t, next)) = fed.requested else { continue };
                let to = if next { fed.next_held().map_or(t, |m| m.min(t)) } else { t };
                let lbts = f.federates.iter().filter(|(&o, _)| o != h).filter_map(|(_, o)| o.promise()).fold(f64::INFINITY, f64::min);
                if !fed.constrained || to < lbts {
                    granted = Some((h, to));
                    break;
                }
            }
            let Some((h, t)) = granted else { break };
            let fed = f.federates.get_mut(&h).unwrap();
            let keep = fed.held.split_off(&(OrdF64(f64::from_bits(t.to_bits() + 1)), String::new(), 0));
            for (_, cb) in std::mem::replace(&mut fed.held, keep) {
                out.push((fed.session, cb));
            }
            fed.time = t;
            fed.requested = None;
            out.push((fed.session, Cb::TimeAdvanceGrant(p::TimeAdvanceGrant { time: lt(t) })));
            changed.push(fx.clone());
        }
    }
    for (to, cb) in out {
        g.callback(to, cb);
    }
    changed.dedup();
    for fx in changed {
        for (to, cb) in mom_refresh(g, &fx) {
            g.callback(to, cb);
        }
    }
}
