//! A typed RTI ambassador over a Federate Protocol session: the HLA services
//! BGWS uses, each a blocking call, and the callbacks as they come.

use super::proto::{self as p, call_request::CallRequest as Rq, call_response::CallResponse as Rs, callback_request::CallbackRequest as Cb};
use super::session::{Session, SessionError, SessionSettings};
use prost::Message;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// An RTI handle: opaque octets, as the RTI gave them.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Handle(pub Vec<u8>);

#[derive(Debug, Clone, PartialEq)]
pub enum RtiError {
    Session(SessionError),
    /// The RTI raised an HLA exception.
    Hla {
        name: String,
        details: String,
    },
    Unexpected(String),
}

impl std::fmt::Display for RtiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RtiError::Session(e) => write!(f, "{e}"),
            RtiError::Hla { name, details } => write!(f, "{name}: {details}"),
            RtiError::Unexpected(w) => write!(f, "unexpected answer from the RTI: {w}"),
        }
    }
}

impl std::error::Error for RtiError {}

impl From<SessionError> for RtiError {
    fn from(e: SessionError) -> Self {
        RtiError::Session(e)
    }
}

pub type Result<T> = std::result::Result<T, RtiError>;

/// A FOM module given by its file's name and contents.
#[derive(Clone, Debug)]
pub struct FomFile {
    pub name: String,
    pub content: Vec<u8>,
}

impl FomFile {
    pub fn read(path: &std::path::Path) -> std::io::Result<FomFile> {
        Ok(FomFile { name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), content: std::fs::read(path)? })
    }
}

fn modules(foms: &[FomFile]) -> Option<p::FomModuleSet> {
    Some(p::FomModuleSet { fom_module: foms.iter().map(|f| p::FomModule { fom_module: Some(p::fom_module::FomModule::File(p::FileFomModule { name: f.name.clone(), content: f.content.clone() })) }).collect() })
}

/// HLAfloat64Time, the time representation BGWS federates use.
pub fn float_time(t: f64) -> Vec<u8> {
    t.to_be_bytes().to_vec()
}

pub fn float_time_of(b: &[u8]) -> Option<f64> {
    b.try_into().ok().map(f64::from_be_bytes)
}

/// The RTI ambassador.
pub struct Rti {
    session: Session,
    held: Mutex<VecDeque<Cb>>,
}

macro_rules! expect {
    ($rs:expr, $v:ident) => {
        match $rs {
            Rs::$v(x) => x,
            other => return Err(RtiError::Unexpected(format!("{other:?}").chars().take(200).collect())),
        }
    };
}

impl Rti {
    /// Open a session and connect.
    pub fn connect(addr: &str, settings: SessionSettings) -> Result<Rti> {
        let ok = p::CallbackResponse { callback_response: Some(p::callback_response::CallbackResponse::CallbackSucceeded(p::CallbackSucceeded {})) }.encode_to_vec();
        let session = Session::connect(addr, settings, ok)?;
        let rti = Rti { session, held: Mutex::new(VecDeque::new()) };
        expect!(rti.call(Rq::ConnectRequest(p::ConnectRequest {}))?, ConnectResponse);
        Ok(rti)
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    fn call(&self, rq: Rq) -> Result<Rs> {
        let bytes = self.session.call(&p::CallRequest { call_request: Some(rq) }.encode_to_vec())?;
        let rs = p::CallResponse::decode(bytes.as_slice()).map_err(|e| RtiError::Unexpected(e.to_string()))?.call_response.ok_or_else(|| RtiError::Unexpected("empty response".into()))?;
        match rs {
            Rs::ExceptionData(e) => Err(RtiError::Hla { name: e.exception_name, details: e.details }),
            rs => Ok(rs),
        }
    }

    /// Create the federation; `false` if it already exists.
    pub fn create_federation(&self, name: &str, foms: &[FomFile]) -> Result<bool> {
        let r = self.call(Rq::CreateFederationExecutionWithModulesAndTimeRequest(p::CreateFederationExecutionWithModulesAndTimeRequest {
            federation_name: name.into(),
            fom_modules: modules(foms),
            logical_time_implementation_name: "HLAfloat64Time".into(),
        }));
        match r {
            Ok(_) => Ok(true),
            Err(RtiError::Hla { name, .. }) if name == "FederationExecutionAlreadyExists" => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Join as `federate` (a name unique in the federation) of type `kind`.
    pub fn join(&self, federate: &str, kind: &str, federation: &str, foms: &[FomFile]) -> Result<Handle> {
        let r = self.call(Rq::JoinFederationExecutionWithNameAndModulesRequest(p::JoinFederationExecutionWithNameAndModulesRequest {
            federate_name: federate.into(),
            federate_type: kind.into(),
            federation_name: federation.into(),
            additional_fom_modules: modules(foms),
        }))?;
        let j = expect!(r, JoinFederationExecutionWithNameAndModulesResponse);
        Ok(Handle(j.result.and_then(|r| r.federate_handle).map(|h| h.data).unwrap_or_default()))
    }

    /// Resign, deleting our objects.
    pub fn resign(&self) -> Result<()> {
        expect!(self.call(Rq::ResignFederationExecutionRequest(p::ResignFederationExecutionRequest { resign_action: p::ResignAction::DeleteObjectsThenDivest as i32 }))?, ResignFederationExecutionResponse);
        Ok(())
    }

    /// Destroy the federation; `false` if others are still in it or it is gone.
    pub fn destroy_federation(&self, name: &str) -> Result<bool> {
        match self.call(Rq::DestroyFederationExecutionRequest(p::DestroyFederationExecutionRequest { federation_name: name.into() })) {
            Ok(_) => Ok(true),
            Err(RtiError::Hla { name, .. }) if name == "FederatesCurrentlyJoined" || name == "FederationExecutionDoesNotExist" => Ok(false),
            Err(e) => Err(e),
        }
    }

    pub fn object_class(&self, name: &str) -> Result<Handle> {
        let r = expect!(self.call(Rq::GetObjectClassHandleRequest(p::GetObjectClassHandleRequest { object_class_name: name.into() }))?, GetObjectClassHandleResponse);
        Ok(Handle(r.result.map(|h| h.data).unwrap_or_default()))
    }

    pub fn attribute(&self, class: &Handle, name: &str) -> Result<Handle> {
        let r = expect!(self.call(Rq::GetAttributeHandleRequest(p::GetAttributeHandleRequest { object_class: Some(p::ObjectClassHandle { data: class.0.clone() }), attribute_name: name.into() }))?, GetAttributeHandleResponse);
        Ok(Handle(r.result.map(|h| h.data).unwrap_or_default()))
    }

    pub fn interaction_class(&self, name: &str) -> Result<Handle> {
        let r = expect!(self.call(Rq::GetInteractionClassHandleRequest(p::GetInteractionClassHandleRequest { interaction_class_name: name.into() }))?, GetInteractionClassHandleResponse);
        Ok(Handle(r.result.map(|h| h.data).unwrap_or_default()))
    }

    pub fn parameter(&self, class: &Handle, name: &str) -> Result<Handle> {
        let r = expect!(self.call(Rq::GetParameterHandleRequest(p::GetParameterHandleRequest { interaction_class: Some(p::InteractionClassHandle { data: class.0.clone() }), parameter_name: name.into() }))?, GetParameterHandleResponse);
        Ok(Handle(r.result.map(|h| h.data).unwrap_or_default()))
    }

    fn attrs(a: &[Handle]) -> Option<p::AttributeHandleSet> {
        Some(p::AttributeHandleSet { attribute_handle: a.iter().map(|h| p::AttributeHandle { data: h.0.clone() }).collect() })
    }

    pub fn publish_object(&self, class: &Handle, attributes: &[Handle]) -> Result<()> {
        expect!(
            self.call(Rq::PublishObjectClassAttributesRequest(p::PublishObjectClassAttributesRequest { object_class: Some(p::ObjectClassHandle { data: class.0.clone() }), attributes: Self::attrs(attributes) }))?,
            PublishObjectClassAttributesResponse
        );
        Ok(())
    }

    pub fn subscribe_object(&self, class: &Handle, attributes: &[Handle]) -> Result<()> {
        expect!(
            self.call(Rq::SubscribeObjectClassAttributesRequest(p::SubscribeObjectClassAttributesRequest { object_class: Some(p::ObjectClassHandle { data: class.0.clone() }), attributes: Self::attrs(attributes) }))?,
            SubscribeObjectClassAttributesResponse
        );
        Ok(())
    }

    pub fn publish_interaction(&self, class: &Handle) -> Result<()> {
        expect!(self.call(Rq::PublishInteractionClassRequest(p::PublishInteractionClassRequest { interaction_class: Some(p::InteractionClassHandle { data: class.0.clone() }) }))?, PublishInteractionClassResponse);
        Ok(())
    }

    pub fn subscribe_interaction(&self, class: &Handle) -> Result<()> {
        expect!(self.call(Rq::SubscribeInteractionClassRequest(p::SubscribeInteractionClassRequest { interaction_class: Some(p::InteractionClassHandle { data: class.0.clone() }) }))?, SubscribeInteractionClassResponse);
        Ok(())
    }

    pub fn register(&self, class: &Handle) -> Result<Handle> {
        let r = expect!(self.call(Rq::RegisterObjectInstanceRequest(p::RegisterObjectInstanceRequest { object_class: Some(p::ObjectClassHandle { data: class.0.clone() }) }))?, RegisterObjectInstanceResponse);
        Ok(Handle(r.result.map(|h| h.data).unwrap_or_default()))
    }

    /// The federates in a federation now (their names).
    pub fn members(&self, federation: &str, wait: Duration) -> Result<Vec<String>> {
        expect!(self.call(Rq::ListFederationExecutionMembersRequest(p::ListFederationExecutionMembersRequest { federation_name: federation.into() }))?, ListFederationExecutionMembersResponse);
        match self.wait_for(wait, |c| matches!(c, Cb::ReportFederationExecutionMembers(_) | Cb::ReportFederationExecutionDoesNotExist(_))) {
            Some(Cb::ReportFederationExecutionMembers(r)) => Ok(r.report.map(|s| s.federation_execution_member_information.into_iter().map(|m| m.federate_name).collect()).unwrap_or_default()),
            Some(_) => Ok(vec![]),
            None => Err(RtiError::Session(SessionError::Timeout)),
        }
    }

    /// Meet the other federates at a synchronization point: register it
    /// (someone else may have), achieve it when announced, and wait until
    /// the federation is synchronized.
    pub fn synchronize(&self, label: &str, wait: Duration) -> Result<()> {
        expect!(self.call(Rq::RegisterFederationSynchronizationPointRequest(p::RegisterFederationSynchronizationPointRequest { synchronization_point_label: label.into(), user_supplied_tag: vec![] }))?, RegisterFederationSynchronizationPointResponse);
        let l = label.to_string();
        self.wait_for(wait, move |c| matches!(c, Cb::AnnounceSynchronizationPoint(a) if a.synchronization_point_label == l)).ok_or(RtiError::Session(SessionError::Timeout))?;
        expect!(self.call(Rq::SynchronizationPointAchievedRequest(p::SynchronizationPointAchievedRequest { synchronization_point_label: label.into(), successfully: true }))?, SynchronizationPointAchievedResponse);
        let l = label.to_string();
        self.wait_for(wait, move |c| matches!(c, Cb::FederationSynchronized(a) if a.synchronization_point_label == l)).ok_or(RtiError::Session(SessionError::Timeout))?;
        Ok(())
    }

    /// Reserve an object instance name and wait for the RTI's answer.
    pub fn reserve_name(&self, name: &str, wait: Duration) -> Result<bool> {
        expect!(self.call(Rq::ReserveObjectInstanceNameRequest(p::ReserveObjectInstanceNameRequest { object_instance_name: name.into() }))?, ReserveObjectInstanceNameResponse);
        let n = name.to_string();
        match self.wait_for(wait, move |c| matches!(c, Cb::ObjectInstanceNameReservationSucceeded(x) if x.object_instance_name == n) || matches!(c, Cb::ObjectInstanceNameReservationFailed(x) if x.object_instance_name == n)) {
            Some(Cb::ObjectInstanceNameReservationSucceeded(_)) => Ok(true),
            Some(_) => Ok(false),
            None => Err(RtiError::Session(SessionError::Timeout)),
        }
    }

    pub fn register_with_name(&self, class: &Handle, name: &str) -> Result<Handle> {
        let r = expect!(
            self.call(Rq::RegisterObjectInstanceWithNameRequest(p::RegisterObjectInstanceWithNameRequest { object_class: Some(p::ObjectClassHandle { data: class.0.clone() }), object_instance_name: name.into() }))?,
            RegisterObjectInstanceWithNameResponse
        );
        Ok(Handle(r.result.map(|h| h.data).unwrap_or_default()))
    }

    /// Update attributes, stamped with `time` if time-managed.
    pub fn update(&self, instance: &Handle, values: &[(Handle, Vec<u8>)], time: Option<f64>) -> Result<()> {
        let object_instance = Some(p::ObjectInstanceHandle { data: instance.0.clone() });
        let attribute_values = Some(p::AttributeHandleValueMap { attribute_handle_value: values.iter().map(|(h, v)| p::AttributeHandleValue { attribute_handle: Some(p::AttributeHandle { data: h.0.clone() }), value: v.clone() }).collect() });
        match time {
            None => {
                expect!(self.call(Rq::UpdateAttributeValuesRequest(p::UpdateAttributeValuesRequest { object_instance, attribute_values, user_supplied_tag: vec![] }))?, UpdateAttributeValuesResponse);
            }
            Some(t) => {
                expect!(
                    self.call(Rq::UpdateAttributeValuesWithTimeRequest(p::UpdateAttributeValuesWithTimeRequest { object_instance, attribute_values, user_supplied_tag: vec![], time: Some(p::LogicalTime { data: float_time(t) }) }))?,
                    UpdateAttributeValuesWithTimeResponse
                );
            }
        }
        Ok(())
    }

    pub fn delete(&self, instance: &Handle) -> Result<()> {
        expect!(self.call(Rq::DeleteObjectInstanceRequest(p::DeleteObjectInstanceRequest { object_instance: Some(p::ObjectInstanceHandle { data: instance.0.clone() }), user_supplied_tag: vec![] }))?, DeleteObjectInstanceResponse);
        Ok(())
    }

    pub fn send_interaction(&self, class: &Handle, params: &[(Handle, Vec<u8>)], time: Option<f64>) -> Result<()> {
        let interaction_class = Some(p::InteractionClassHandle { data: class.0.clone() });
        let parameter_values = Some(p::ParameterHandleValueMap { parameter_handle_value: params.iter().map(|(h, v)| p::ParameterHandleValue { parameter_handle: Some(p::ParameterHandle { data: h.0.clone() }), value: v.clone() }).collect() });
        match time {
            None => {
                expect!(self.call(Rq::SendInteractionRequest(p::SendInteractionRequest { interaction_class, parameter_values, user_supplied_tag: vec![] }))?, SendInteractionResponse);
            }
            Some(t) => {
                expect!(
                    self.call(Rq::SendInteractionWithTimeRequest(p::SendInteractionWithTimeRequest { interaction_class, parameter_values, user_supplied_tag: vec![], time: Some(p::LogicalTime { data: float_time(t) }) }))?,
                    SendInteractionWithTimeResponse
                );
            }
        }
        Ok(())
    }

    /// Become time-regulating with this lookahead (s), and wait for the RTI to agree.
    pub fn enable_time_regulation(&self, lookahead: f64, wait: Duration) -> Result<()> {
        expect!(self.call(Rq::EnableTimeRegulationRequest(p::EnableTimeRegulationRequest { lookahead: Some(p::LogicalTimeInterval { data: float_time(lookahead) }) }))?, EnableTimeRegulationResponse);
        self.wait_for(wait, |c| matches!(c, Cb::TimeRegulationEnabled(_))).map(|_| ()).ok_or(RtiError::Session(SessionError::Timeout))
    }

    pub fn enable_time_constrained(&self, wait: Duration) -> Result<()> {
        expect!(self.call(Rq::EnableTimeConstrainedRequest(p::EnableTimeConstrainedRequest {}))?, EnableTimeConstrainedResponse);
        self.wait_for(wait, |c| matches!(c, Cb::TimeConstrainedEnabled(_))).map(|_| ()).ok_or(RtiError::Session(SessionError::Timeout))
    }

    pub fn time_advance_request(&self, t: f64) -> Result<()> {
        expect!(self.call(Rq::TimeAdvanceRequestRequest(p::TimeAdvanceRequestRequest { time: Some(p::LogicalTime { data: float_time(t) }) }))?, TimeAdvanceRequestResponse);
        Ok(())
    }

    pub fn unsubscribe_object(&self, class: &Handle) -> Result<()> {
        expect!(self.call(Rq::UnsubscribeObjectClassRequest(p::UnsubscribeObjectClassRequest { object_class: Some(p::ObjectClassHandle { data: class.0.clone() }) }))?, UnsubscribeObjectClassResponse);
        Ok(())
    }

    pub fn unsubscribe_interaction(&self, class: &Handle) -> Result<()> {
        expect!(self.call(Rq::UnsubscribeInteractionClassRequest(p::UnsubscribeInteractionClassRequest { interaction_class: Some(p::InteractionClassHandle { data: class.0.clone() }) }))?, UnsubscribeInteractionClassResponse);
        Ok(())
    }

    pub fn unpublish_object(&self, class: &Handle) -> Result<()> {
        expect!(self.call(Rq::UnpublishObjectClassRequest(p::UnpublishObjectClassRequest { object_class: Some(p::ObjectClassHandle { data: class.0.clone() }) }))?, UnpublishObjectClassResponse);
        Ok(())
    }

    pub fn unpublish_interaction(&self, class: &Handle) -> Result<()> {
        expect!(self.call(Rq::UnpublishInteractionClassRequest(p::UnpublishInteractionClassRequest { interaction_class: Some(p::InteractionClassHandle { data: class.0.clone() }) }))?, UnpublishInteractionClassResponse);
        Ok(())
    }

    /// Ask for the next timestamped message, or `t` if none comes sooner.
    pub fn next_message_request(&self, t: f64) -> Result<()> {
        expect!(self.call(Rq::NextMessageRequestRequest(p::NextMessageRequestRequest { time: Some(p::LogicalTime { data: float_time(t) }) }))?, NextMessageRequestResponse);
        Ok(())
    }

    fn take_new(&self) {
        let mut held = self.held.lock().unwrap();
        for c in self.session.poll_callbacks() {
            if let Ok(p::CallbackRequest { callback_request: Some(cb) }) = p::CallbackRequest::decode(c.payload.as_slice()) {
                held.push_back(cb);
            }
        }
    }

    /// Every callback received so far, in order.
    pub fn callbacks(&self) -> Vec<Cb> {
        self.take_new();
        self.held.lock().unwrap().drain(..).collect()
    }

    /// Wait up to `timeout` for a callback `want` accepts; return the
    /// callbacks before it (still to handle) and it.
    pub fn wait_until(&self, timeout: Duration, want: impl Fn(&Cb) -> bool) -> Option<(Vec<Cb>, Cb)> {
        let end = Instant::now() + timeout;
        loop {
            self.take_new();
            {
                let mut held = self.held.lock().unwrap();
                if let Some(i) = held.iter().position(&want) {
                    let before: Vec<Cb> = held.drain(..i).collect();
                    let it = held.pop_front().unwrap();
                    return Some((before, it));
                }
            }
            let now = Instant::now();
            if now >= end {
                return None;
            }
            if let Some(c) = self.session.next_callback((end - now).min(Duration::from_millis(50))) {
                if let Ok(p::CallbackRequest { callback_request: Some(cb) }) = p::CallbackRequest::decode(c.payload.as_slice()) {
                    self.held.lock().unwrap().push_back(cb);
                }
            }
        }
    }

    /// Wait for a callback `want` accepts, leaving the others held.
    pub fn wait_for(&self, timeout: Duration, want: impl Fn(&Cb) -> bool) -> Option<Cb> {
        let (before, it) = self.wait_until(timeout, want)?;
        let mut held = self.held.lock().unwrap();
        for c in before.into_iter().rev() {
            held.push_front(c);
        }
        Some(it)
    }
}
