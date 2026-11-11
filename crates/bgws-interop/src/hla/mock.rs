//! A Federate Protocol server with a small RTI behind it, for tests and for
//! trying two BGWS federates on one machine without a commercial RTI.
//!
//! It speaks the session layer exactly as [`super::session`] does (new
//! session, heartbeats, resume with nothing lost) and serves the HLA calls
//! BGWS makes: connect; create, join, resign and destroy; class, attribute,
//! interaction and parameter handles by name; publish and subscribe;
//! register, update, reflect and delete; interactions; time regulation,
//! time-constrained and time-advance requests with timestamp order.
//!
//! What it leaves out: FOM parsing (handles are given out for any name),
//! class hierarchy (a subscription is to the exact class published),
//! ownership, regions, save and restore, and every other service, which
//! answers `RTIinternalError`. Timestamp-ordered messages are delivered in
//! (time, sender's federate name, sender's order) order, so a time-managed
//! run is the same every time.

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
    /// Object class → attributes subscribed.
    subscribed: BTreeMap<u64, BTreeSet<u64>>,
    interactions: BTreeSet<u64>,
    /// Lookahead, if time-regulating.
    regulating: Option<f64>,
    constrained: bool,
    time: f64,
    requested: Option<f64>,
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
    class: u64,
    owner: u64,
    name: String,
}

#[derive(Default)]
struct Federation {
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
    /// Handles for names ("class:…", "attr:class:…", …).
    names: BTreeMap<String, u64>,
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

    fn handle(&mut self, key: String) -> u64 {
        if let Some(&h) = self.names.get(&key) {
            return h;
        }
        let h = self.next_handle;
        self.next_handle += 1;
        self.names.insert(key, h);
        h
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
                resign(&mut g, session);
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

fn och(h: u64) -> Option<p::ObjectClassHandle> {
    Some(p::ObjectClassHandle { data: handle_bytes(h) })
}

fn fed_handle(h: u64) -> Option<p::FederateHandle> {
    Some(p::FederateHandle { data: handle_bytes(h) })
}

fn lt(t: f64) -> Option<p::LogicalTime> {
    Some(p::LogicalTime { data: time_bytes(t) })
}

fn resign(g: &mut State, session: u64) {
    let Some((fx, me)) = joined(g, session) else { return };
    let mut out = vec![];
    if let Some(f) = g.federations.get_mut(&fx) {
        let mine: Vec<u64> = f.instances.iter().filter(|(_, i)| i.owner == me).map(|(&h, _)| h).collect();
        for h in mine {
            let i = f.instances.remove(&h).unwrap();
            for (&fh, fed) in &f.federates {
                if fh != me && fed.subscribed.contains_key(&i.class) {
                    out.push((fed.session, Cb::RemoveObjectInstance(p::RemoveObjectInstance { object_instance: Some(p::ObjectInstanceHandle { data: handle_bytes(h) }), user_supplied_tag: vec![], producing_federate: fed_handle(me) })));
                }
            }
        }
        f.federates.remove(&me);
    }
    if let Some(ss) = g.sessions.get_mut(&session) {
        ss.joined = None;
    }
    for (to, cb) in out {
        g.callback(to, cb);
    }
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
    match rq {
        Rq::ConnectRequest(_) => (Rs::ConnectResponse(p::ConnectResponse { configuration_result: None }), none),
        Rq::ConnectWithConfigurationRequest(_) => (Rs::ConnectWithConfigurationResponse(p::ConnectWithConfigurationResponse { configuration_result: None }), none),
        Rq::DisconnectRequest(_) => (Rs::DisconnectResponse(p::DisconnectResponse {}), none),
        Rq::CreateFederationExecutionRequest(r) => create(g, r.federation_name).map_or_else(|e| (e, vec![]), |_| (Rs::CreateFederationExecutionResponse(p::CreateFederationExecutionResponse {}), vec![])),
        Rq::CreateFederationExecutionWithModulesRequest(r) => create(g, r.federation_name).map_or_else(|e| (e, vec![]), |_| (Rs::CreateFederationExecutionWithModulesResponse(p::CreateFederationExecutionWithModulesResponse {}), vec![])),
        Rq::CreateFederationExecutionWithModulesAndTimeRequest(r) => {
            create(g, r.federation_name).map_or_else(|e| (e, vec![]), |_| (Rs::CreateFederationExecutionWithModulesAndTimeResponse(p::CreateFederationExecutionWithModulesAndTimeResponse {}), vec![]))
        }
        Rq::DestroyFederationExecutionRequest(r) => match g.federations.get(&r.federation_name) {
            None => (exception("FederationExecutionDoesNotExist", r.federation_name), none),
            Some(f) if !f.federates.is_empty() => (exception("FederatesCurrentlyJoined", r.federation_name), none),
            Some(_) => {
                g.federations.remove(&r.federation_name);
                (Rs::DestroyFederationExecutionResponse(p::DestroyFederationExecutionResponse {}), none)
            }
        },
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
        Rq::JoinFederationExecutionRequest(r) => join(g, session, format!("{}-{session}", r.federate_type), r.federation_name, |h| Rs::JoinFederationExecutionResponse(p::JoinFederationExecutionResponse { result: Some(h) })),
        Rq::JoinFederationExecutionWithNameRequest(r) => join(g, session, r.federate_name, r.federation_name, |h| Rs::JoinFederationExecutionWithNameResponse(p::JoinFederationExecutionWithNameResponse { result: Some(h) })),
        Rq::JoinFederationExecutionWithModulesRequest(r) => {
            join(g, session, format!("{}-{session}", r.federate_type), r.federation_name, |h| Rs::JoinFederationExecutionWithModulesResponse(p::JoinFederationExecutionWithModulesResponse { result: Some(h) }))
        }
        Rq::JoinFederationExecutionWithNameAndModulesRequest(r) => {
            join(g, session, r.federate_name, r.federation_name, |h| Rs::JoinFederationExecutionWithNameAndModulesResponse(p::JoinFederationExecutionWithNameAndModulesResponse { result: Some(h) }))
        }
        Rq::ResignFederationExecutionRequest(_) => {
            resign(g, session);
            (Rs::ResignFederationExecutionResponse(p::ResignFederationExecutionResponse {}), none)
        }
        Rq::GetObjectClassHandleRequest(r) => {
            let h = g.handle(format!("class:{}", r.object_class_name));
            (Rs::GetObjectClassHandleResponse(p::GetObjectClassHandleResponse { result: och(h) }), none)
        }
        Rq::GetAttributeHandleRequest(r) => {
            let h = g.handle(format!("attr:{}:{}", hb(&r.object_class), r.attribute_name));
            (Rs::GetAttributeHandleResponse(p::GetAttributeHandleResponse { result: Some(p::AttributeHandle { data: handle_bytes(h) }) }), none)
        }
        Rq::GetInteractionClassHandleRequest(r) => {
            let h = g.handle(format!("interaction:{}", r.interaction_class_name));
            (Rs::GetInteractionClassHandleResponse(p::GetInteractionClassHandleResponse { result: Some(p::InteractionClassHandle { data: handle_bytes(h) }) }), none)
        }
        Rq::GetParameterHandleRequest(r) => {
            let h = g.handle(format!("param:{}:{}", hb(&r.interaction_class), r.parameter_name));
            (Rs::GetParameterHandleResponse(p::GetParameterHandleResponse { result: Some(p::ParameterHandle { data: handle_bytes(h) }) }), none)
        }
        Rq::PublishObjectClassAttributesRequest(_) => {
            fed!();
            (Rs::PublishObjectClassAttributesResponse(p::PublishObjectClassAttributesResponse {}), none)
        }
        Rq::PublishInteractionClassRequest(_) => {
            fed!();
            (Rs::PublishInteractionClassResponse(p::PublishInteractionClassResponse {}), none)
        }
        Rq::SubscribeObjectClassAttributesRequest(r) => {
            let (fx, me) = fed!();
            let class = hb(&r.object_class);
            let attrs: BTreeSet<u64> = r.attributes.map(|a| a.attribute_handle.iter().map(|x| handle_of(&x.data)).collect()).unwrap_or_default();
            let f = g.federations.get_mut(&fx).unwrap();
            let fresh = !f.federates[&me].subscribed.contains_key(&class);
            f.federates.get_mut(&me).unwrap().subscribed.entry(class).or_default().extend(attrs);
            let mut out = vec![];
            if fresh {
                let session = f.federates[&me].session;
                for (&h, i) in &f.instances {
                    if i.class == class && i.owner != me {
                        out.push((session, discover(h, i)));
                    }
                }
            }
            (Rs::SubscribeObjectClassAttributesResponse(p::SubscribeObjectClassAttributesResponse {}), out)
        }
        Rq::SubscribeInteractionClassRequest(r) => {
            let (fx, me) = fed!();
            let f = g.federations.get_mut(&fx).unwrap();
            f.federates.get_mut(&me).unwrap().interactions.insert(hb(&r.interaction_class));
            (Rs::SubscribeInteractionClassResponse(p::SubscribeInteractionClassResponse {}), none)
        }
        Rq::RegisterObjectInstanceRequest(r) => {
            let (fx, me) = fed!();
            let class = hb(&r.object_class);
            let key = format!("instance:{}", g.next_handle);
            let h = g.handle(key);
            let f = g.federations.get_mut(&fx).unwrap();
            let i = Instance { class, owner: me, name: format!("HLAobjectRoot-{h}") };
            let out = f.federates.iter().filter(|(&fh, fed)| fh != me && fed.subscribed.contains_key(&class)).map(|(_, fed)| (fed.session, discover(h, &i))).collect();
            f.instances.insert(h, i);
            (Rs::RegisterObjectInstanceResponse(p::RegisterObjectInstanceResponse { result: Some(p::ObjectInstanceHandle { data: handle_bytes(h) }) }), out)
        }
        Rq::ReserveObjectInstanceNameRequest(r) => {
            let (fx, _) = fed!();
            let f = g.federations.get_mut(&fx).unwrap();
            let cb = if f.instances.values().any(|i| i.name == r.object_instance_name) || !f.reserved.insert(r.object_instance_name.clone()) {
                Cb::ObjectInstanceNameReservationFailed(p::ObjectInstanceNameReservationFailed { object_instance_name: r.object_instance_name })
            } else {
                Cb::ObjectInstanceNameReservationSucceeded(p::ObjectInstanceNameReservationSucceeded { object_instance_name: r.object_instance_name })
            };
            (Rs::ReserveObjectInstanceNameResponse(p::ReserveObjectInstanceNameResponse {}), vec![(session, cb)])
        }
        Rq::RegisterObjectInstanceWithNameRequest(r) => {
            let (fx, me) = fed!();
            let class = hb(&r.object_class);
            let key = format!("instance:{}", g.next_handle);
            let h = g.handle(key);
            let f = g.federations.get_mut(&fx).unwrap();
            if !f.reserved.remove(&r.object_instance_name) {
                return (exception("ObjectInstanceNameNotReserved", r.object_instance_name), none);
            }
            let i = Instance { class, owner: me, name: r.object_instance_name };
            let out = f.federates.iter().filter(|(&fh, fed)| fh != me && fed.subscribed.contains_key(&class)).map(|(_, fed)| (fed.session, discover(h, &i))).collect();
            f.instances.insert(h, i);
            (Rs::RegisterObjectInstanceWithNameResponse(p::RegisterObjectInstanceWithNameResponse { result: Some(p::ObjectInstanceHandle { data: handle_bytes(h) }) }), out)
        }
        Rq::DeleteObjectInstanceRequest(r) => {
            let (fx, me) = fed!();
            let h = hb(&r.object_instance);
            let f = g.federations.get_mut(&fx).unwrap();
            let Some(i) = f.instances.remove(&h) else { return (exception("ObjectInstanceNotKnown", h.to_string()), none) };
            let out = f
                .federates
                .iter()
                .filter(|(&fh, fed)| fh != me && fed.subscribed.contains_key(&i.class))
                .map(|(_, fed)| (fed.session, Cb::RemoveObjectInstance(p::RemoveObjectInstance { object_instance: r.object_instance.clone(), user_supplied_tag: r.user_supplied_tag.clone(), producing_federate: fed_handle(me) })))
                .collect();
            (Rs::DeleteObjectInstanceResponse(p::DeleteObjectInstanceResponse {}), out)
        }
        Rq::UpdateAttributeValuesRequest(r) => {
            let (fx, me) = fed!();
            let out = reflect(g, &fx, me, r.object_instance, r.attribute_values, r.user_supplied_tag, None);
            (Rs::UpdateAttributeValuesResponse(p::UpdateAttributeValuesResponse {}), out)
        }
        Rq::UpdateAttributeValuesWithTimeRequest(r) => {
            let (fx, me) = fed!();
            let out = reflect(g, &fx, me, r.object_instance, r.attribute_values, r.user_supplied_tag, Some(time_of(&r.time.unwrap_or_default().data)));
            (Rs::UpdateAttributeValuesWithTimeResponse(p::UpdateAttributeValuesWithTimeResponse { result: Some(p::MessageRetractionReturn { retraction_handle_is_valid: false, message_retraction_handle: None }) }), out)
        }
        Rq::SendInteractionRequest(r) => {
            let (fx, me) = fed!();
            let out = interaction(g, &fx, me, r.interaction_class, r.parameter_values, r.user_supplied_tag, None);
            (Rs::SendInteractionResponse(p::SendInteractionResponse {}), out)
        }
        Rq::SendInteractionWithTimeRequest(r) => {
            let (fx, me) = fed!();
            let out = interaction(g, &fx, me, r.interaction_class, r.parameter_values, r.user_supplied_tag, Some(time_of(&r.time.unwrap_or_default().data)));
            (Rs::SendInteractionWithTimeResponse(p::SendInteractionWithTimeResponse { result: Some(p::MessageRetractionReturn { retraction_handle_is_valid: false, message_retraction_handle: None }) }), out)
        }
        Rq::EnableTimeRegulationRequest(r) => {
            let (fx, me) = fed!();
            let f = g.federations.get_mut(&fx).unwrap().federates.get_mut(&me).unwrap();
            f.regulating = Some(time_of(&r.lookahead.unwrap_or_default().data));
            let cb = Cb::TimeRegulationEnabled(p::TimeRegulationEnabled { time: lt(f.time) });
            (Rs::EnableTimeRegulationResponse(p::EnableTimeRegulationResponse {}), vec![(session, cb)])
        }
        Rq::EnableTimeConstrainedRequest(_) => {
            let (fx, me) = fed!();
            let f = g.federations.get_mut(&fx).unwrap().federates.get_mut(&me).unwrap();
            f.constrained = true;
            let cb = Cb::TimeConstrainedEnabled(p::TimeConstrainedEnabled { time: lt(f.time) });
            (Rs::EnableTimeConstrainedResponse(p::EnableTimeConstrainedResponse {}), vec![(session, cb)])
        }
        Rq::TimeAdvanceRequestRequest(r) => {
            let (fx, me) = fed!();
            let f = g.federations.get_mut(&fx).unwrap().federates.get_mut(&me).unwrap();
            f.requested = Some(time_of(&r.time.unwrap_or_default().data));
            (Rs::TimeAdvanceRequestResponse(p::TimeAdvanceRequestResponse {}), none)
        }
        _ => (exception("RTIinternalError", "this service is not in the mock RTI"), none),
    }
}

fn create(g: &mut State, name: String) -> Result<(), Rs> {
    if g.federations.contains_key(&name) {
        return Err(exception("FederationExecutionAlreadyExists", name));
    }
    g.federations.insert(name, Federation::default());
    Ok(())
}

fn join(g: &mut State, session: u64, name: String, federation: String, wrap: impl Fn(p::JoinResult) -> Rs) -> Out {
    if joined(g, session).is_some() {
        return (exception("FederateAlreadyExecutionMember", federation), vec![]);
    }
    let h = g.handle(format!("federate:{federation}:{name}:{session}"));
    let Some(f) = g.federations.get_mut(&federation) else { return (exception("FederationExecutionDoesNotExist", federation), vec![]) };
    if f.federates.values().any(|x| x.name == name) {
        return (exception("FederateNameAlreadyInUse", name), vec![]);
    }
    f.federates.insert(h, Federate { session, name, kind: "unknown".into(), ..Default::default() });
    g.sessions.get_mut(&session).unwrap().joined = Some((federation, h));
    (wrap(p::JoinResult { federate_handle: fed_handle(h), logical_time_implementation_name: "HLAfloat64Time".into() }), vec![])
}

fn discover(h: u64, i: &Instance) -> Cb {
    Cb::DiscoverObjectInstance(p::DiscoverObjectInstance { object_instance: Some(p::ObjectInstanceHandle { data: handle_bytes(h) }), object_class: och(i.class), object_instance_name: i.name.clone(), producing_federate: fed_handle(i.owner) })
}

/// Deliver now, or hold for a time-constrained receiver until its grant.
fn deliver(f: &mut Federation, me: u64, to: u64, time: Option<f64>, now: Cb, timed: impl FnOnce() -> Cb) -> Option<(u64, Cb)> {
    let regulating = f.federates[&me].regulating.is_some();
    let order = f.federates[&me].sent;
    let sender = f.federates[&me].name.clone();
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

fn reflect(g: &mut State, fx: &str, me: u64, inst: Option<p::ObjectInstanceHandle>, values: Option<p::AttributeHandleValueMap>, tag: Vec<u8>, time: Option<f64>) -> Vec<(u64, Cb)> {
    let f = g.federations.get_mut(fx).unwrap();
    let h = hb(&inst);
    let Some(class) = f.instances.get(&h).map(|i| i.class) else { return vec![] };
    let values = values.unwrap_or_default();
    f.federates.get_mut(&me).unwrap().sent += 1;
    let targets: Vec<(u64, p::AttributeHandleValueMap)> = f
        .federates
        .iter()
        .filter(|(&fh, _)| fh != me)
        .filter_map(|(&fh, fed)| {
            let wanted = fed.subscribed.get(&class)?;
            let v: Vec<p::AttributeHandleValue> = values.attribute_handle_value.iter().filter(|a| wanted.contains(&hb(&a.attribute_handle))).cloned().collect();
            (!v.is_empty()).then_some((fh, p::AttributeHandleValueMap { attribute_handle_value: v }))
        })
        .collect();
    let mut out = vec![];
    for (to, v) in targets {
        let now = Cb::ReflectAttributeValues(p::ReflectAttributeValues {
            object_instance: inst.clone(),
            attribute_values: Some(v.clone()),
            user_supplied_tag: tag.clone(),
            transportation_type: None,
            producing_federate: fed_handle(me),
            optional_sent_regions: None,
        });
        let timed = || {
            Cb::ReflectAttributeValuesWithTime(p::ReflectAttributeValuesWithTime {
                object_instance: inst.clone(),
                attribute_values: Some(v.clone()),
                user_supplied_tag: tag.clone(),
                transportation_type: None,
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

fn interaction(g: &mut State, fx: &str, me: u64, class: Option<p::InteractionClassHandle>, values: Option<p::ParameterHandleValueMap>, tag: Vec<u8>, time: Option<f64>) -> Vec<(u64, Cb)> {
    let f = g.federations.get_mut(fx).unwrap();
    let c = hb(&class);
    f.federates.get_mut(&me).unwrap().sent += 1;
    let to: Vec<u64> = f.federates.iter().filter(|(&fh, fed)| fh != me && fed.interactions.contains(&c)).map(|(&fh, _)| fh).collect();
    let mut out = vec![];
    for t in to {
        let now = Cb::ReceiveInteraction(p::ReceiveInteraction {
            interaction_class: class.clone(),
            parameter_values: values.clone(),
            user_supplied_tag: tag.clone(),
            transportation_type: None,
            producing_federate: fed_handle(me),
            optional_sent_regions: None,
        });
        let timed = || {
            Cb::ReceiveInteractionWithTime(p::ReceiveInteractionWithTime {
                interaction_class: class.clone(),
                parameter_values: values.clone(),
                user_supplied_tag: tag.clone(),
                transportation_type: None,
                producing_federate: fed_handle(me),
                optional_sent_regions: None,
                time: lt(time.unwrap_or(0.0)),
                sent_order_type: p::OrderType::Timestamp as i32,
                received_order_type: p::OrderType::Timestamp as i32,
                optional_retraction: None,
            })
        };
        out.extend(deliver(f, me, t, time, now, timed));
    }
    out
}

/// Grant every time-advance request that can be: a constrained federate
/// asking for t is granted once no other regulating federate can still send
/// a message stamped t or earlier (its requested or current time plus its
/// lookahead is past t). Held messages up to t go first, in order.
fn grant_all(g: &mut State) {
    let mut out = vec![];
    for f in g.federations.values_mut() {
        loop {
            let mut granted = None;
            for (&h, fed) in &f.federates {
                let Some(t) = fed.requested else { continue };
                let lbts = f.federates.iter().filter(|(&o, _)| o != h).filter_map(|(_, o)| o.regulating.map(|l| o.requested.unwrap_or(o.time).max(o.time) + l)).fold(f64::INFINITY, f64::min);
                if !fed.constrained || t < lbts {
                    granted = Some((h, t));
                    break;
                }
            }
            let Some((h, t)) = granted else { break };
            let fed = f.federates.get_mut(&h).unwrap();
            let keep = fed.held.split_off(&(OrdF64(f64::from_bits((t.to_bits()) + 1)), String::new(), 0));
            for (_, cb) in std::mem::replace(&mut fed.held, keep) {
                out.push((fed.session, cb));
            }
            fed.time = t;
            fed.requested = None;
            out.push((fed.session, Cb::TimeAdvanceGrant(p::TimeAdvanceGrant { time: lt(t) })));
        }
    }
    for (to, cb) in out {
        g.callback(to, cb);
    }
}
