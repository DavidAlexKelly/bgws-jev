//! A game in an HLA federation (RPR FOM, Federate Protocol): the same truth
//! and fire as on DIS, and the same importer for what others send.
//!
//! Phase 4's radios, messages, emitters, aggregates and minefields go out
//! as RPR objects and an interaction too ([`super::rpr_extras`]), and other
//! federates' radios and emitters come in for our electronic warfare.
//!
//! On joining, it publishes and subscribes the RPR entity classes and
//! `WeaponFire` and `MunitionDetonation`. Each second it registers new
//! entities (under reserved names, `BGWS-site-application-entity`, which
//! other federates' interactions name them by), updates those the DIS
//! dead-reckoning rules say are due, and sends the second's fire. What other
//! federates reflect and send becomes inputs at the next tick boundary.
//!
//! Time-managed, it is time-regulating and time-constrained: updates and
//! interactions are stamped the tick's time plus the lookahead, and each
//! tick ends with a time-advance request to the next second, which the RTI
//! grants once every regulating federate has got there. Everything stamped
//! up to it is delivered first, in the RTI's order, so the federation runs
//! the same way every time.

use super::encoding::Dec;
use super::proto::callback_request::CallbackRequest as Cb;
use super::rpr::{self, EntityAttrs, Shot};
use super::netn;
use super::rpr_extras;
use super::rti::{FomFile, Handle, Rti, RtiError};
use super::session::SessionSettings;
use crate::dr::{DeadReckoner, DrSettings};
use crate::events::{FireBook, Outgoing};
use crate::geo::place;
use crate::geoid::Geoid;
use crate::import::Importer;
use crate::model::{truths, ExternalId};
use bgws_core::{EntityId, LogEvent};
use bgws_scenario::v4::InteropSpec;
use bgws_sim::remote::{EmitterKind, ExternalInput, ExternalKey, RemoteEmitter};
use bgws_sim::Sim;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct HlaSettings {
    /// The Federate Protocol server, `host:port`.
    pub address: String,
    pub federation: String,
    /// Our federate name, unique in the federation.
    pub federate: String,
    /// FOM modules to create or join with (the RPR FOM's, for a real RTI).
    pub foms: Vec<FomFile>,
    /// A MIM to create the federation with (NETN's, for `UniqueId`).
    pub mim: Option<FomFile>,
    /// Send the event log as `BGWS_Event` interactions (joining with our
    /// module when FOM modules are given).
    pub events: bool,
    /// Time-regulating and time-constrained, with this lookahead (s).
    pub lookahead: Option<f64>,
    /// Federates expected: with more than one, the game starts only when all
    /// have joined and met at the `ReadyToRun` synchronization point, so a
    /// time-managed federation starts together.
    pub federates: usize,
    /// How long to wait for a time-advance grant.
    pub grant_wait: Duration,
    pub session: SessionSettings,
    pub dr: DrSettings,
}

impl HlaSettings {
    pub fn new(address: &str, federation: &str, federate: &str) -> HlaSettings {
        HlaSettings {
            address: address.into(),
            federation: federation.into(),
            federate: federate.into(),
            foms: vec![],
            mim: None,
            events: true,
            lookahead: None,
            federates: 1,
            grant_wait: Duration::from_secs(600),
            session: SessionSettings::default(),
            dr: DrSettings::default(),
        }
    }
}

struct ClassInfo {
    name: &'static str,
    handle: Handle,
    by_name: BTreeMap<&'static str, Handle>,
    by_handle: BTreeMap<Handle, &'static str>,
}

struct InteractionInfo {
    handle: Handle,
    by_name: BTreeMap<&'static str, Handle>,
    by_handle: BTreeMap<Handle, &'static str>,
}

/// NETN-SMC tasking: the task classes we hear (with their parameters'
/// handles), how we answer, and what we have told of each task we took.
#[derive(Default)]
struct Tasking {
    classes: BTreeMap<Handle, (&'static str, BTreeMap<Handle, &'static str>)>,
    response: Option<InteractionInfo>,
    status: Option<InteractionInfo>,
    /// Task id (text) → its UUID and the status last sent.
    told: BTreeMap<String, ([u8; 16], i32)>,
}

/// Another federate's object instance, as known so far.
struct RemoteInstance {
    class: usize,
    name: String,
    attrs: EntityAttrs,
}

/// What we publish beyond entities and fire (phase 4), and when last.
#[derive(Default)]
struct ExtrasOut {
    last: BTreeMap<String, (f64, String)>,
    /// Our extra objects: key → instance.
    instances: BTreeMap<String, Handle>,
    /// Emitters on the air last second (their keys).
    emitting: BTreeSet<String>,
    aggregated: BTreeSet<u16>,
    messages_after: bgws_core::SimTime,
}

/// Other federates' radios and emitters, as read so far.
#[derive(Default)]
struct ExtrasIn {
    radios: BTreeMap<Handle, rpr_extras::RadioIn>,
    systems: BTreeMap<String, rpr_extras::SystemIn>,
    beams: BTreeMap<Handle, (rpr_extras::BeamIn, bool)>,
}

pub struct HlaLink {
    pub rti: Rti,
    pub settings: HlaSettings,
    spec: InteropSpec,
    importer: Option<Importer>,
    dr: DeadReckoner,
    fires: FireBook,
    classes: Vec<ClassInfo>,
    fire: InteractionInfo,
    detonation: InteractionInfo,
    signal: InteractionInfo,
    /// `BGWS_Event`, when the federation has it.
    event: Option<InteractionInfo>,
    tasking: Tasking,
    /// `BGWS_Control`, StartResume and StopFreeze, where the federation has them.
    control: Option<InteractionInfo>,
    sim_management: Vec<(Handle, &'static str)>,
    /// Control messages for us, not yet taken.
    controls: Vec<netn::ControlMsg>,
    /// Callbacks that came while no game was loaded, for the next one.
    held: Vec<Cb>,
    /// Instance names we have registered before (still ours to reuse).
    used_names: BTreeSet<String>,
    /// Our entities by their NETN UniqueId.
    by_uuid: BTreeMap<[u8; 16], EntityId>,
    extras_out: ExtrasOut,
    extras_in: ExtrasIn,
    /// Ours: entity → (class, instance).
    ours: BTreeMap<EntityId, (usize, Handle)>,
    our_names: BTreeMap<String, EntityId>,
    remote: BTreeMap<Handle, RemoteInstance>,
    name_to_key: BTreeMap<String, ExternalKey>,
    key_to_name: BTreeMap<ExternalKey, String>,
    inbox: Vec<ExternalInput>,
    pub notes: Vec<String>,
    /// Updates and interactions sent, inputs taken in.
    pub sent: u64,
    pub received: u64,
    /// The federation time last granted.
    pub granted: Option<f64>,
}

/// Names to handles and back.
type HandleMaps = (BTreeMap<&'static str, Handle>, BTreeMap<Handle, &'static str>);

fn names_to_handles(names: &[&'static str], get: impl Fn(&str) -> Result<Handle, RtiError>) -> Result<HandleMaps, RtiError> {
    let mut a = BTreeMap::new();
    let mut b = BTreeMap::new();
    for &n in names {
        let h = get(n)?;
        a.insert(n, h.clone());
        b.insert(h, n);
    }
    Ok((a, b))
}

impl HlaLink {
    /// Connect, create the federation if need be, join, and declare.
    pub fn join(settings: HlaSettings, spec: InteropSpec) -> Result<HlaLink, RtiError> {
        let rti = Rti::connect(&settings.address, settings.session)?;
        // With a FOM given, ours for the event log goes with it (with none,
        // the federation takes any name, ours included).
        let mut foms = settings.foms.clone();
        if settings.events && !foms.is_empty() && !foms.iter().any(|f| f.name == netn::EVENTS_MODULE_NAME) {
            foms.push(FomFile { name: netn::EVENTS_MODULE_NAME.into(), content: netn::EVENTS_MODULE.as_bytes().to_vec() });
        }
        match &settings.mim {
            Some(mim) => rti.create_federation_with_mim(&settings.federation, &foms, mim)?,
            None => rti.create_federation(&settings.federation, &foms)?,
        };
        rti.join(&settings.federate, "BGWS", &settings.federation, &foms)?;
        let mut classes = vec![];
        for name in rpr::CLASSES {
            let handle = rti.object_class(name)?;
            let attrs = rpr::attributes_for(name);
            let (mut by_name, mut by_handle) = names_to_handles(&attrs, |a| rti.attribute(&handle, a))?;
            // NETN's identity, where the federation's FOM has it.
            for a in netn::IDENTITY_ATTRIBUTES {
                if let Ok(h) = rti.attribute(&handle, a) {
                    by_name.insert(a, h.clone());
                    by_handle.insert(h, a);
                }
            }
            let all: Vec<Handle> = by_name.values().cloned().collect();
            rti.publish_object(&handle, &all)?;
            rti.subscribe_object(&handle, &all)?;
            classes.push(ClassInfo { name, handle, by_name, by_handle });
        }
        // Phase 4's: radios, emitters, aggregates, minefields.
        for name in rpr_extras::CLASSES {
            let handle = rti.object_class(name)?;
            let attrs = rpr_extras::attributes_for(name);
            let (by_name, by_handle) = names_to_handles(&attrs, |a| rti.attribute(&handle, a))?;
            let all: Vec<Handle> = by_name.values().cloned().collect();
            rti.publish_object(&handle, &all)?;
            rti.subscribe_object(&handle, &all)?;
            classes.push(ClassInfo { name, handle, by_name, by_handle });
        }
        let interaction = |name: &str, params: &[&'static str]| -> Result<InteractionInfo, RtiError> {
            let handle = rti.interaction_class(name)?;
            let (by_name, by_handle) = names_to_handles(params, |p| rti.parameter(&handle, p))?;
            rti.publish_interaction(&handle)?;
            rti.subscribe_interaction(&handle)?;
            Ok(InteractionInfo { handle, by_name, by_handle })
        };
        let fire = interaction(rpr::WEAPON_FIRE, &rpr::WEAPON_FIRE_PARAMETERS)?;
        let detonation = interaction(rpr::MUNITION_DETONATION, &rpr::MUNITION_DETONATION_PARAMETERS)?;
        let signal = interaction(rpr_extras::RAW_BINARY_SIGNAL, &rpr_extras::SIGNAL_PARAMETERS)?;
        let mut notes = vec![];
        let event = if settings.events {
            // Publish only: what other BGWS federates logged is theirs.
            let ev = rti.interaction_class(netn::EVENT_CLASS).and_then(|handle| {
                let (by_name, by_handle) = names_to_handles(&netn::EVENT_PARAMETERS, |p| rti.parameter(&handle, p))?;
                rti.publish_interaction(&handle)?;
                Ok(InteractionInfo { handle, by_name, by_handle })
            });
            match ev {
                Ok(e) => Some(e),
                Err(e) => {
                    notes.push(format!("the federation has no {} ({e}); the event log is not sent", netn::EVENT_CLASS));
                    None
                }
            }
        } else {
            None
        };
        // NETN-SMC tasking, where the federation has it: orders for our units
        // (Foundry's, through the HLA–Palantir bridge).
        let mut tasking = Tasking::default();
        for (short, _, _) in netn::TASK_CLASSES {
            let Ok(h) = rti.interaction_class(&netn::task_class(short)) else { continue };
            let mut by_handle = BTreeMap::new();
            for p in ["Entity", "TaskId", "UniqueId", "TaskParameters", "Tasks"] {
                if let Ok(ph) = rti.parameter(&h, p) {
                    by_handle.insert(ph, p);
                }
            }
            if rti.subscribe_interaction(&h).is_ok() {
                tasking.classes.insert(h, (short, by_handle));
            }
        }
        if !tasking.classes.is_empty() {
            let answer = |name: &str, params: &[&'static str]| -> Option<InteractionInfo> {
                let handle = rti.interaction_class(name).ok()?;
                let (by_name, by_handle) = names_to_handles(params, |p| rti.parameter(&handle, p)).ok()?;
                rti.publish_interaction(&handle).ok()?;
                Some(InteractionInfo { handle, by_name, by_handle })
            };
            tasking.response = answer(netn::SMC_RESPONSE, &netn::SMC_RESPONSE_PARAMETERS);
            tasking.status = answer(netn::TASK_STATUS, &netn::TASK_STATUS_PARAMETERS);
        }
        // Control of a waiting simulation (bgws federate): our module's
        // BGWS_Control, and RPR simulation management.
        let control = rti.interaction_class(netn::CONTROL_CLASS).ok().and_then(|handle| {
            let (by_name, by_handle) = names_to_handles(&netn::CONTROL_PARAMETERS, |p| rti.parameter(&handle, p)).ok()?;
            rti.subscribe_interaction(&handle).ok()?;
            Some(InteractionInfo { handle, by_name, by_handle })
        });
        let mut sim_management = vec![];
        for (name, command) in [(netn::START_RESUME, "start"), (netn::STOP_FREEZE, "pause")] {
            if let Ok(h) = rti.interaction_class(name) {
                if rti.subscribe_interaction(&h).is_ok() {
                    sim_management.push((h, command));
                }
            }
        }
        if let Some(l) = settings.lookahead {
            rti.enable_time_regulation(l, settings.grant_wait)?;
            rti.enable_time_constrained(settings.grant_wait)?;
        }
        if settings.federates > 1 {
            let started = std::time::Instant::now();
            while rti.members(&settings.federation, settings.session.response_timeout)?.len() < settings.federates {
                if started.elapsed() > settings.grant_wait {
                    return Err(RtiError::Hla { name: "FederatesMissing".into(), details: format!("fewer than {} federates joined {} in {:?}", settings.federates, settings.federation, settings.grant_wait) });
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            rti.synchronize("ReadyToRun", settings.grant_wait)?;
        }
        Ok(HlaLink {
            rti,
            dr: DeadReckoner::new(settings.dr),
            settings,
            spec,
            importer: None,
            fires: FireBook::default(),
            classes,
            fire,
            detonation,
            signal,
            event,
            tasking,
            control,
            sim_management,
            controls: vec![],
            held: vec![],
            used_names: BTreeSet::new(),
            by_uuid: BTreeMap::new(),
            extras_out: ExtrasOut::default(),
            extras_in: ExtrasIn::default(),
            ours: BTreeMap::new(),
            our_names: BTreeMap::new(),
            remote: BTreeMap::new(),
            name_to_key: BTreeMap::new(),
            key_to_name: BTreeMap::new(),
            inbox: vec![],
            notes,
            sent: 0,
            received: 0,
            granted: None,
        })
    }

    fn stamp(&self, sim: &Sim) -> Option<f64> {
        self.settings.lookahead.map(|l| sim.ctx.now.secs() + l)
    }

    /// The RTIobjectId another federate knows an entity by.
    fn name_of(&self, sim: &Sim, id: Option<EntityId>) -> String {
        let Some(id) = id else { return String::new() };
        if let Some(&e) = sim.ctx.order.get(id.index()) {
            if let Some(r) = sim.world.get::<bgws_sim::remote::Remote>(e) {
                return self.key_to_name.get(&r.key).cloned().unwrap_or_default();
            }
        }
        ExternalId::ours(&self.spec, id).map(|x| rpr::instance_name(x.site, x.application, x.entity)).unwrap_or_default()
    }

    /// After a second: publish it; time-managed, advance to the next second.
    pub fn tick(&mut self, sim: &Sim, events: &[LogEvent]) -> Result<usize, RtiError> {
        let now = sim.ctx.now.secs();
        let stamp = self.stamp(sim);
        let mut n = 0;
        for t in truths(sim, &self.spec) {
            if self.dr.due(now, &t).is_none() {
                continue;
            }
            let class = rpr::CLASSES.iter().position(|&c| c == rpr::class_for(t.entity_type[0], t.entity_type[1])).unwrap_or(rpr::CLASSES.len() - 1);
            let instance = match self.ours.get(&t.id) {
                Some((_, h)) => h.clone(),
                None => {
                    let name = rpr::instance_name(t.external.site, t.external.application, t.external.entity);
                    // A name we used in an earlier game stays ours (HLA keeps a
                    // reservation until the federate resigns); an RTI that
                    // released it wants it reserved again.
                    let again = if self.used_names.contains(&name) { self.rti.register_with_name(&self.classes[class].handle, &name).ok() } else { None };
                    let h = match again {
                        Some(h) => h,
                        None => {
                            if !self.rti.reserve_name(&name, self.settings.session.response_timeout)? {
                                self.notes.push(format!("the RTI would not reserve {name}; {} is not published", t.marking));
                                self.ours.insert(t.id, (class, Handle::default()));
                                continue;
                            }
                            self.rti.register_with_name(&self.classes[class].handle, &name)?
                        }
                    };
                    self.used_names.insert(name.clone());
                    self.ours.insert(t.id, (class, h.clone()));
                    self.by_uuid.insert(netn::unique_id(&name), t.id);
                    self.our_names.insert(name, t.id);
                    h
                }
            };
            if instance.0.is_empty() {
                continue;
            }
            let c = &self.classes[class];
            let mut attrs = rpr::entity_attributes(&t, c.name);
            if c.by_name.contains_key("UniqueId") || c.by_name.contains_key("Callsign") {
                let name = rpr::instance_name(t.external.site, t.external.application, t.external.entity);
                attrs.push(("UniqueId", netn::unique_id(&name).to_vec()));
                attrs.push(("Callsign", rpr::unicode(&callsign(sim, t.id))));
            }
            if !self.tasking.classes.is_empty() {
                attrs.push(("SupportedActions", netn::supported_actions()));
            }
            let values: Vec<(Handle, Vec<u8>)> = attrs.into_iter().filter_map(|(k, v)| c.by_name.get(k).map(|h| (h.clone(), v))).collect();
            self.rti.update(&instance, &values, stamp)?;
            self.dr.sent(now, &t);
            n += 1;
        }
        for o in self.fires.from_events(sim, events) {
            let (info, params) = match &o {
                Outgoing::Fire(f) => {
                    let (ty, warhead, fuse) = crate::dis::munition_codes(sim, &self.spec, Some(f.firer), &f.munition);
                    let shot = Shot {
                        event: (f.event, self.name_of(sim, Some(f.firer))),
                        firer: self.name_of(sim, Some(f.firer)),
                        target: self.name_of(sim, f.target),
                        location: place(&sim.ctx.frame, Geoid::egm96(), f.from).ecef,
                        munition_type: ty,
                        warhead,
                        fuse,
                        quantity: f.rounds.min(u16::MAX as u32) as u16,
                        rate: 0,
                        range_m: f.range_m as f32,
                        result: 0,
                    };
                    (&self.fire, rpr::weapon_fire(&shot))
                }
                Outgoing::Detonation(d) => {
                    let (ty, warhead, fuse) = crate::dis::munition_codes(sim, &self.spec, d.firer, &d.munition);
                    let firer = self.name_of(sim, d.firer);
                    let shot = Shot {
                        event: (d.event, firer.clone()),
                        firer,
                        target: self.name_of(sim, d.target),
                        location: place(&sim.ctx.frame, Geoid::egm96(), d.at).ecef,
                        munition_type: ty,
                        warhead,
                        fuse,
                        quantity: d.rounds.min(u16::MAX as u32) as u16,
                        rate: 0,
                        range_m: 0.0,
                        result: d.result as u8,
                    };
                    (&self.detonation, rpr::munition_detonation(&shot))
                }
            };
            let values: Vec<(Handle, Vec<u8>)> = params.into_iter().filter_map(|(k, v)| info.by_name.get(k).map(|h| (h.clone(), v))).collect();
            self.rti.send_interaction(&info.handle, &values, stamp)?;
            n += 1;
        }
        n += self.publish_extras(sim, stamp)?;
        n += self.publish_events(sim, events)?;
        n += self.report_tasks(sim);
        self.sent += n as u64;
        if self.settings.lookahead.is_some() {
            let next = now.floor() + 1.0;
            self.rti.time_advance_request(next)?;
            let (before, _grant) = self
                .rti
                .wait_until(self.settings.grant_wait, |c| matches!(c, Cb::TimeAdvanceGrant(_)))
                .ok_or_else(|| RtiError::Hla { name: "TimeAdvanceGrant".into(), details: format!("no grant to {next} s within {:?}", self.settings.grant_wait) })?;
            self.granted = Some(next);
            for cb in before {
                self.take(sim, cb);
            }
        }
        Ok(n)
    }

    /// A control message (ours, or RPR simulation management): queued for
    /// [`HlaLink::control`]. True if it was one.
    fn control_in(&mut self, class: &Handle, values: &[(Vec<u8>, Vec<u8>)]) -> bool {
        if let Some(&(_, command)) = self.sim_management.iter().find(|(h, _)| h == class) {
            self.controls.push(netn::ControlMsg { command: command.into(), options: "{}".into(), ..Default::default() });
            return true;
        }
        let Some(info) = self.control.as_ref().filter(|c| &c.handle == class) else { return false };
        match netn::read_control(&Self::by_name(values, &info.by_handle)) {
            Ok(c) if c.for_me(&self.settings.federate) => self.controls.push(c),
            Ok(_) => {}
            Err(e) => self.notes.push(format!("a BGWS_Control does not decode: {e}")),
        }
        true
    }

    /// Control messages for this federate since last asked. Works with no
    /// game loaded: everything else that comes meanwhile is kept for the
    /// next game's first [`HlaLink::inbound`].
    pub fn control(&mut self) -> Vec<netn::ControlMsg> {
        for cb in self.rti.callbacks() {
            let is_control = match &cb {
                Cb::ReceiveInteraction(r) => {
                    let class = Handle(r.interaction_class.clone().map(|x| x.data).unwrap_or_default());
                    let values: Vec<(Vec<u8>, Vec<u8>)> = r.parameter_values.clone().map(|m| m.parameter_handle_value.into_iter().map(|a| (a.parameter_handle.map(|x| x.data).unwrap_or_default(), a.value)).collect()).unwrap_or_default();
                    self.control_in(&class, &values)
                }
                _ => false,
            };
            if !is_control {
                self.held.push(cb);
                // Bounded: a long idle wait keeps the latest.
                if self.held.len() > 100_000 {
                    self.held.drain(..50_000);
                }
            }
        }
        std::mem::take(&mut self.controls)
    }

    /// Send a `BGWS_Event` that no game logged (a waiting simulation's
    /// status), when the federation has the class.
    pub fn send_event(&mut self, out: &netn::EventOut) -> Result<(), RtiError> {
        let Some(info) = self.event.as_ref() else { return Ok(()) };
        let values: Vec<(Handle, Vec<u8>)> = netn::event_parameters(out).into_iter().filter_map(|(k, v)| info.by_name.get(k).map(|h| (h.clone(), v))).collect();
        self.rti.send_interaction(&info.handle, &values, None)
    }

    /// Between games: take our objects off, and forget the last game's
    /// state. Other federates' objects stay known (they are still there).
    pub fn reset(&mut self) {
        for (_, (_, h)) in std::mem::take(&mut self.ours) {
            if !h.0.is_empty() {
                let _ = self.rti.delete(&h);
            }
        }
        for (_, h) in std::mem::take(&mut self.extras_out.instances) {
            let _ = self.rti.delete(&h);
        }
        self.extras_out = ExtrasOut::default();
        self.extras_in = ExtrasIn::default();
        self.our_names.clear();
        self.by_uuid.clear();
        self.dr = DeadReckoner::new(self.settings.dr);
        self.fires = FireBook::default();
        self.importer = None;
        self.inbox.clear();
        self.tasking.told.clear();
        // Other federates' attributes are kept: their next update carries only
        // what changed, and the next game's importer builds them from all of it.
    }

    /// A task for one of our units: answered at once (accepted or refused),
    /// and taken in as an outside order at the next tick boundary. A task for
    /// another federate's entity is not ours to answer.
    fn task_in(&mut self, sim: &Sim, short: &'static str, names: &BTreeMap<Handle, &'static str>, values: Vec<(Vec<u8>, Vec<u8>)>) {
        let named = Self::by_name(&values, names);
        if std::env::var_os("BGWS_DEBUG_TASKING").is_some() {
            eprintln!("task in: {short}, parameters {:?}, entity {:?}, ours {:?}", named.keys().collect::<Vec<_>>(), named.get("Entity").map(|b| b.len()), self.by_uuid.len());
        }
        let u16_of = |k: &str| named.get(k).and_then(|b| <[u8; 16]>::try_from(b.as_slice()).ok());
        let Some(id) = u16_of("Entity").and_then(|u| self.by_uuid.get(&u).copied()) else { return };
        let task_id = u16_of("TaskId");
        let action = u16_of("UniqueId").or(task_id);
        let params = named.get(if short == "CancelTasks" { "Tasks" } else { "TaskParameters" }).map(|v| v.as_slice());
        let order = match netn::decode_task(short, params) {
            Ok(t) => self.order_of(t),
            Err(e) => Err(e.to_string()),
        };
        let order = order.and_then(|o| match bgws_sim::tasking::refusal(sim, id) {
            Some(why) => Err(why.to_string()),
            None => Ok(o),
        });
        if let (Some(a), Some(info)) = (action, self.tasking.response.as_ref()) {
            let values: Vec<(Handle, Vec<u8>)> = netn::smc_response(&a, order.is_ok()).into_iter().filter_map(|(k, v)| info.by_name.get(k).map(|h| (h.clone(), v))).collect();
            if let Err(e) = self.rti.send_interaction(&info.handle, &values, None) {
                self.notes.push(format!("SMC_Response not sent: {e}"));
            }
        }
        if let Some(t) = task_id {
            self.send_status(&t, if order.is_ok() { 0 } else { 1 });
        }
        match order {
            Ok(order) => {
                let task = task_id.or(action).map(|u| netn::uuid_text(&u)).unwrap_or_else(|| format!("{short}-{}", sim.ctx.now.secs()));
                if let Some(t) = task_id {
                    self.tasking.told.insert(task.clone(), (t, 0));
                }
                self.inbox.push(ExternalInput::Order { entity: id, task, order });
            }
            Err(why) => self.notes.push(format!("{short} for {} refused: {why}", callsign(sim, id))),
        }
    }

    /// A NETN task as one of our outside orders.
    fn order_of(&self, t: netn::Task) -> Result<bgws_sim::tasking::ExternalOrder, String> {
        use bgws_sim::tasking::ExternalOrder as O;
        let ll = |x: [f64; 3]| {
            let g = bgws_core::Ecef { x: x[0], y: x[1], z: x[2] }.to_geodetic();
            (g.lat, g.lon)
        };
        Ok(match t {
            netn::Task::Move { to, path, move_type, .. } if path.is_empty() => {
                let (lat, lon) = ll(to);
                O::Move { lat, lon, roads: move_type == 1 }
            }
            netn::Task::Move { to, path, .. } => O::Route { points: path.into_iter().chain([to]).map(ll).collect(), repeat: false },
            netn::Task::Patrol { path, patrol_type, .. } => {
                if path.is_empty() {
                    return Err("a patrol with no waypoints".into());
                }
                let mut points: Vec<(f64, f64)> = path.into_iter().map(ll).collect();
                // Reverse: there and back along the same points.
                if patrol_type == 2 && points.len() > 2 {
                    let back: Vec<(f64, f64)> = points[1..points.len() - 1].iter().rev().copied().collect();
                    points.extend(back);
                }
                O::Route { points, repeat: patrol_type != 0 }
            }
            netn::Task::MagicMove { to, heading_deg } => {
                let (lat, lon) = ll(to);
                O::Teleport { lat, lon, heading_deg: heading_deg as f64 }
            }
            netn::Task::Stop => O::Halt,
            netn::Task::Cancel { .. } => O::Cancel,
            netn::Task::Roe(r) => O::Roe {
                roe: match r {
                    0 => "never",
                    1 => "always",
                    _ => "ifFiredUpon",
                }
                .into(),
            },
            netn::Task::DirectFire { target } => O::Engage { target: *self.by_uuid.get(&target).ok_or("the target is not one of this simulation's entities")? },
            netn::Task::IndirectFire { area } => {
                let first = *area.first().ok_or("a fire mission with no target area")?;
                let (lat, lon) = ll(first);
                O::FireAt { lat, lon }
            }
            netn::Task::Unsupported(n) => return Err(format!("{n} is not modelled")),
        })
    }

    fn send_status(&mut self, task: &[u8; 16], status: i32) {
        let Some(info) = self.tasking.status.as_ref() else { return };
        let values: Vec<(Handle, Vec<u8>)> = netn::task_status(task, status).into_iter().filter_map(|(k, v)| info.by_name.get(k).map(|h| (h.clone(), v))).collect();
        if let Err(e) = self.rti.send_interaction(&info.handle, &values, None) {
            self.notes.push(format!("ETR_TaskStatus not sent: {e}"));
        }
    }

    /// Each task we took whose state has changed since we last said.
    fn report_tasks(&mut self, sim: &Sim) -> usize {
        let Some(book) = sim.world.get_resource::<bgws_sim::tasking::TaskBook>() else { return 0 };
        let changes: Vec<(String, [u8; 16], i32)> = book
            .tasks
            .iter()
            .filter_map(|t| {
                let (u, last) = self.tasking.told.get(&t.task)?;
                let code = t.status as i32;
                (code != *last).then_some((t.task.clone(), *u, code))
            })
            .collect();
        for (task, u, code) in &changes {
            self.send_status(u, *code);
            self.tasking.told.insert(task.clone(), (*u, *code));
        }
        changes.len()
    }

    /// An entity named in an event: ours with our UUID, another federate's
    /// by its instance name only (its UUID is its owner's to give).
    fn party(&self, sim: &Sim, id: Option<EntityId>) -> netn::Party {
        let Some(id) = id else { return netn::Party::default() };
        let name = self.name_of(sim, Some(id));
        let remote = sim.ctx.order.get(id.index()).is_some_and(|&e| bgws_sim::remote::is_remote(sim, e));
        if remote {
            netn::Party { name, unique_id: String::new(), callsign: callsign(sim, id) }
        } else {
            netn::Party::of(&name, &callsign(sim, id))
        }
    }

    /// The second's events as `BGWS_Event` interactions (receive order: they
    /// are for people and tools, not for other simulations to judge by).
    fn publish_events(&mut self, sim: &Sim, events: &[LogEvent]) -> Result<usize, RtiError> {
        let Some(info) = self.event.as_ref() else { return Ok(0) };
        let mut n = 0;
        for e in events {
            let at = e.entity.and_then(|id| sim.ctx.order.get(id.index()).and_then(|&x| sim.world.get::<bgws_sim::components::Kin>(x))).map(|k| {
                let g = place(&sim.ctx.frame, Geoid::egm96(), k.pos).geodetic;
                [g.lat, g.lon, g.h]
            });
            let out = netn::EventOut {
                kind: e.kind.clone(),
                seq: e.seq,
                t: e.t.secs(),
                federate: self.settings.federate.clone(),
                side: e.side.map(|s| s.to_string()).unwrap_or_default(),
                entity: self.party(sim, e.entity),
                target: self.party(sim, e.target),
                at,
                text: e.text.clone(),
                data: netn::data_json(&e.data),
            };
            let values: Vec<(Handle, Vec<u8>)> = netn::event_parameters(&out).into_iter().filter_map(|(k, v)| info.by_name.get(k).map(|h| (h.clone(), v))).collect();
            self.rti.send_interaction(&info.handle, &values, None)?;
            n += 1;
        }
        Ok(n)
    }

    /// Before a second: what came in, as inputs to apply (and record).
    pub fn inbound(&mut self, sim: &Sim) -> Vec<ExternalInput> {
        for cb in std::mem::take(&mut self.held) {
            self.take(sim, cb);
        }
        if self.settings.lookahead.is_none() {
            for cb in self.rti.callbacks() {
                self.take(sim, cb);
            }
        }
        let inputs = std::mem::take(&mut self.inbox);
        self.received += inputs.len() as u64;
        if let Some(imp) = self.importer.as_mut() {
            self.notes.append(&mut imp.notes);
        }
        inputs
    }

    fn by_name(values: &[(Vec<u8>, Vec<u8>)], names: &BTreeMap<Handle, &'static str>) -> BTreeMap<String, Vec<u8>> {
        values.iter().filter_map(|(h, v)| names.get(&Handle(h.clone())).map(|n| (n.to_string(), v.clone()))).collect()
    }

    fn take(&mut self, sim: &Sim, cb: Cb) {
        if self.importer.is_none() {
            self.importer = Some(Importer::new(sim, self.spec.clone()));
        }
        match cb {
            Cb::DiscoverObjectInstance(d) => {
                let class = d.object_class.map(|c| Handle(c.data)).and_then(|h| self.classes.iter().position(|c| c.handle == h)).unwrap_or(rpr::CLASSES.len() - 1);
                let h = Handle(d.object_instance.map(|x| x.data).unwrap_or_default());
                self.remote.insert(h, RemoteInstance { class, name: d.object_instance_name, attrs: EntityAttrs::default() });
            }
            Cb::ReflectAttributeValues(r) => {
                let values = r.attribute_values.map(|m| m.attribute_handle_value.into_iter().map(|a| (a.attribute_handle.map(|x| x.data).unwrap_or_default(), a.value)).collect()).unwrap_or_default();
                self.reflect(sim, Handle(r.object_instance.map(|x| x.data).unwrap_or_default()), values);
            }
            Cb::ReflectAttributeValuesWithTime(r) => {
                let values = r.attribute_values.map(|m| m.attribute_handle_value.into_iter().map(|a| (a.attribute_handle.map(|x| x.data).unwrap_or_default(), a.value)).collect()).unwrap_or_default();
                self.reflect(sim, Handle(r.object_instance.map(|x| x.data).unwrap_or_default()), values);
            }
            Cb::RemoveObjectInstance(r) => {
                let h = Handle(r.object_instance.map(|x| x.data).unwrap_or_default());
                if let Some(radio) = self.extras_in.radios.remove(&h) {
                    if let (Some(host), Some(index)) = (radio.host, radio.index) {
                        self.inbox.push(ExternalInput::Emitter { key: key_of(host), slot: (0, index), emitter: None });
                    }
                }
                if let Some((beam, _)) = self.extras_in.beams.remove(&h) {
                    if let Some((key, slot)) = self.beam_slot(&beam) {
                        self.inbox.push(ExternalInput::Emitter { key, slot, emitter: None });
                    }
                }
                if let Some(i) = self.remote.remove(&h) {
                    if let Some(k) = self.name_to_key.get(&i.name) {
                        self.inbox.push(ExternalInput::EntityRemoved { key: *k });
                    }
                }
            }
            Cb::ReceiveInteraction(r) => {
                let values = r.parameter_values.map(|m| m.parameter_handle_value.into_iter().map(|a| (a.parameter_handle.map(|x| x.data).unwrap_or_default(), a.value)).collect()).unwrap_or_default();
                self.interaction(sim, Handle(r.interaction_class.map(|x| x.data).unwrap_or_default()), values);
            }
            Cb::ReceiveInteractionWithTime(r) => {
                let values = r.parameter_values.map(|m| m.parameter_handle_value.into_iter().map(|a| (a.parameter_handle.map(|x| x.data).unwrap_or_default(), a.value)).collect()).unwrap_or_default();
                self.interaction(sim, Handle(r.interaction_class.map(|x| x.data).unwrap_or_default()), values);
            }
            _ => {}
        }
    }

    fn reflect(&mut self, sim: &Sim, h: Handle, values: Vec<(Vec<u8>, Vec<u8>)>) {
        let Some(inst) = self.remote.get_mut(&h) else { return };
        let named = Self::by_name(&values, &self.classes[inst.class].by_handle);
        if inst.class >= rpr::CLASSES.len() {
            let (class, name) = (self.classes[inst.class].name, inst.name.clone());
            self.reflect_extra(class, h, name, &named);
            return;
        }
        for bad in inst.attrs.merge(&named) {
            self.notes.push(format!("{}: {bad}", inst.name));
        }
        let a = &inst.attrs;
        let (Some(id), Some(t), Some(s)) = (a.id, a.entity_type, a.spatial) else { return };
        let key = ExternalKey { site: id[0], application: id[1], entity: id[2] };
        self.name_to_key.insert(inst.name.clone(), key);
        self.key_to_name.insert(key, inst.name.clone());
        let (force, label, damage) = (a.force.unwrap_or(0), a.marking.clone().unwrap_or_default(), a.damage.unwrap_or(0).clamp(0, 3) as u8);
        let imp = self.importer.as_mut().unwrap();
        let inputs = imp.entity(sim, key, t, force, label, s.location, s.velocity.map(f64::from), s.orientation.map(f64::from), damage, false);
        self.inbox.extend(inputs);
    }

    fn our_entity(&self, name: &str) -> Option<EntityId> {
        self.our_names.get(name).copied()
    }

    fn interaction(&mut self, sim: &Sim, class: Handle, values: Vec<(Vec<u8>, Vec<u8>)>) {
        if self.control_in(&class, &values) {
            return;
        }
        if let Some((short, names)) = self.tasking.classes.get(&class).cloned() {
            self.task_in(sim, short, &names, values);
            return;
        }
        let is_fire = class == self.fire.handle;
        let names = if is_fire {
            &self.fire.by_handle
        } else if class == self.detonation.handle {
            &self.detonation.by_handle
        } else {
            return;
        };
        let named = Self::by_name(&values, names);
        let shot = match rpr::shot_of(&named) {
            Ok(s) => s,
            Err(e) => {
                self.notes.push(format!("an interaction does not decode: {e}"));
                return;
            }
        };
        let firer = self.name_to_key.get(&shot.firer).copied();
        let target = self.our_entity(&shot.target);
        let imp = self.importer.as_mut().unwrap();
        if is_fire {
            if let Some(f) = firer {
                self.inbox.push(imp.fired(f, target, shot.munition_type));
            }
        } else {
            let issuer = self.name_to_key.get(&shot.event.1).map(|k| (k.site, k.application)).unwrap_or((0, 0));
            self.inbox.push(imp.detonation(sim, firer, target, shot.munition_type, shot.quantity, shot.location, shot.result, (issuer.0, issuer.1, shot.event.0)));
        }
    }

    /// Our extra object for `key`, registered under `name` if new.
    fn extra_instance(&mut self, key: &str, class: &str, name: &str) -> Result<Option<Handle>, RtiError> {
        if let Some(h) = self.extras_out.instances.get(key) {
            return Ok(Some(h.clone()));
        }
        let Some(c) = self.classes.iter().position(|c| c.name == class) else { return Ok(None) };
        if !self.rti.reserve_name(name, self.settings.session.response_timeout)? {
            self.notes.push(format!("the RTI would not reserve {name}"));
            return Ok(None);
        }
        let h = self.rti.register_with_name(&self.classes[c].handle, name)?;
        self.extras_out.instances.insert(key.to_string(), h.clone());
        Ok(Some(h))
    }

    /// Update an extra object if due (changed, or `every` seconds on).
    #[allow(clippy::too_many_arguments)]
    fn update_extra(&mut self, key: &str, class: &str, name: &str, sig: String, every: f64, now: f64, attrs: Vec<(&'static str, Vec<u8>)>, stamp: Option<f64>) -> Result<usize, RtiError> {
        if self.extras_out.last.get(key).is_some_and(|(t, s)| *s == sig && now - t < every) {
            return Ok(0);
        }
        let Some(h) = self.extra_instance(key, class, name)? else { return Ok(0) };
        let c = self.classes.iter().find(|c| c.name == class).unwrap();
        let values: Vec<(Handle, Vec<u8>)> = attrs.into_iter().filter_map(|(k, v)| c.by_name.get(k).map(|a| (a.clone(), v))).collect();
        self.rti.update(&h, &values, stamp)?;
        self.extras_out.last.insert(key.to_string(), (now, sig));
        Ok(1)
    }

    fn delete_extra(&mut self, key: &str) -> Result<usize, RtiError> {
        self.extras_out.last.remove(key);
        match self.extras_out.instances.remove(key) {
            Some(h) => self.rti.delete(&h).map(|_| 1),
            None => Ok(0),
        }
    }

    /// Phase 4: radios, messages, emitters, aggregates and minefields.
    fn publish_extras(&mut self, sim: &Sim, stamp: Option<f64>) -> Result<usize, RtiError> {
        use crate::extras;
        let now = sim.ctx.now.secs();
        let spec = self.spec.clone();
        let ext = |id: EntityId| ExternalId::ours(&spec, id).map(|x| [x.site, x.application, x.entity]).unwrap_or_default();
        let pos = |id: EntityId| sim.ctx.order.get(id.index()).and_then(|&e| sim.world.get::<bgws_sim::components::Kin>(e)).map(|k| k.pos).unwrap_or_default();
        let country = |id: EntityId| sim.ctx.order.get(id.index()).and_then(|&e| sim.world.get::<bgws_sim::components::Ident>(e)).map(|i| spec.country_of(i.side)).unwrap_or(0);
        let mut n = 0;
        for r in extras::radios(sim, &spec) {
            let host = self.name_of(sim, Some(r.id));
            let attrs = rpr_extras::radio_attributes(&r, (ext(r.id), &host), place(&sim.ctx.frame, Geoid::egm96(), pos(r.id)).ecef, country(r.id));
            let name = rpr_extras::radio_name(&host, r.number);
            n += self.update_extra(&format!("radio:{}:{}", r.id.0, r.number), rpr_extras::RADIO_TRANSMITTER, &name, format!("{:?}{}", r.state, r.freq_hz), 5.0, now, attrs, stamp)?;
        }
        for m in extras::messages(sim, &spec, self.extras_out.messages_after) {
            self.extras_out.messages_after = self.extras_out.messages_after.max(m.done);
            let host = rpr_extras::radio_name(&self.name_of(sim, Some(m.from)), 1);
            let params = rpr_extras::signal_parameters(&m, &host, ext(m.from)[2], ExternalId::ours(&spec, m.to).map(|x| x.entity).unwrap_or(0));
            let values: Vec<(Handle, Vec<u8>)> = params.into_iter().filter_map(|(k, v)| self.signal.by_name.get(k).map(|h| (h.clone(), v))).collect();
            self.rti.send_interaction(&self.signal.handle, &values, stamp)?;
            n += 1;
        }
        let mut emitting = BTreeSet::new();
        for e in extras::emitters(sim, &spec) {
            let host = self.name_of(sim, Some(e.id));
            let (sys, beam) = (rpr_extras::emitter_name(&host, e.number), rpr_extras::beam_name(&host, e.number));
            let key = format!("emitter:{}:{}", e.id.0, e.number);
            let sig = format!("{}:{:.0}:{:.0}:{:.1}", e.jammer, e.freq_hz, e.bw_hz, e.erp_w);
            n += self.update_extra(&key, rpr_extras::EMITTER_SYSTEM, &sys, sig.clone(), 10.0, now, rpr_extras::emitter_system_attributes(&e, (ext(e.id), &host)), stamp)?;
            let class = if e.jammer { rpr_extras::JAMMER_BEAM } else { rpr_extras::RADAR_BEAM };
            n += self.update_extra(&format!("{key}:beam"), class, &beam, sig, 10.0, now, rpr_extras::beam_attributes(&e, &sys), stamp)?;
            emitting.insert(key);
        }
        // Emitters gone quiet: their objects are deleted.
        let quiet: Vec<String> = self.extras_out.emitting.difference(&emitting).cloned().collect();
        for key in quiet {
            n += self.delete_extra(&format!("{key}:beam"))?;
            n += self.delete_extra(&key)?;
        }
        self.extras_out.emitting = emitting;
        for a in extras::aggregates(sim, &spec) {
            let key = format!("aggregate:{}", a.number);
            if !a.aggregated && !self.extras_out.aggregated.contains(&a.number) {
                continue;
            }
            let placed = place(&sim.ctx.frame, Geoid::egm96(), a.centre);
            let heading = a.velocity.e.atan2(a.velocity.n);
            let o = crate::geo::dis_orientation(&placed.axes, if a.velocity.horizontal() > 0.2 { heading } else { 0.0 }, 0.0, 0.0).map(|x| x as f32);
            let v = crate::geo::to_ecef_vec(&placed.axes, a.velocity).map(|x| x as f32);
            let members: Vec<String> = a.members.iter().map(|&m| self.name_of(sim, Some(m))).collect();
            let id = [spec.site, spec.application, crate::dis_extras::AGGREGATE_BASE + a.number];
            let attrs = rpr_extras::aggregate_attributes(&a, id, spec.force_of(a.side), spec.country_of(a.side), placed.ecef, o, v, &members, a.aggregated);
            let name = rpr_extras::aggregate_name(spec.site, spec.application, a.number);
            if a.aggregated {
                self.extras_out.aggregated.insert(a.number);
                let sig = format!("{:.0}:{:.0}:{}:{:.0}", a.centre.e, a.centre.n, a.members.len(), a.radius_m);
                n += self.update_extra(&key, rpr_extras::AGGREGATE, &name, sig, 10.0, now, attrs, stamp)?;
            } else {
                // Split up: said once, then the object goes.
                self.extras_out.aggregated.remove(&a.number);
                self.extras_out.last.remove(&key);
                n += self.update_extra(&key, rpr_extras::AGGREGATE, &name, "disaggregated".into(), f64::INFINITY, now, attrs, stamp)?;
                n += self.delete_extra(&key)?;
            }
        }
        let mut designating = BTreeSet::new();
        for d in extras::designations(sim, &spec) {
            let host = self.name_of(sim, Some(d.designator));
            let target = self.name_of(sim, d.target);
            let key = format!("designator:{}", d.id);
            let attrs = rpr_extras::designator_attributes(&d, (ext(d.designator), &host), &target, place(&sim.ctx.frame, Geoid::egm96(), d.spot).ecef);
            let name = rpr_extras::designator_name(&host, d.id);
            let sig = format!("{:.0}:{:.0}:{}", d.spot.e, d.spot.n, d.lasing);
            n += self.update_extra(&key, rpr_extras::DESIGNATOR, &name, sig, 1.0, now, attrs, stamp)?;
            designating.insert(key);
        }
        let ended: Vec<String> = self.extras_out.instances.keys().filter(|k| k.starts_with("designator:") && !designating.contains(*k)).cloned().collect();
        for key in ended {
            n += self.delete_extra(&key)?;
        }
        for m in extras::minefields(sim) {
            let placed = place(&sim.ctx.frame, Geoid::egm96(), bgws_core::Enu::new(m.centre.e, m.centre.n, sim.ctx.ground.height(m.centre.e, m.centre.n)));
            let o = crate::geo::dis_orientation(&placed.axes, m.bearing, 0.0, 0.0).map(|x| x as f32);
            let attrs = rpr_extras::minefield_attributes(&m, [spec.site, spec.application, m.number], spec.force_of(m.owner), placed.ecef, o);
            let name = rpr_extras::minefield_name(spec.site, spec.application, m.number);
            n += self.update_extra(&format!("minefield:{}", m.number), rpr_extras::MINEFIELD, &name, format!("{}", m.lanes), 10.0, now, attrs, stamp)?;
        }
        Ok(n)
    }

    /// A remote beam's entity and emitter slot, once its system is known.
    fn beam_slot(&self, beam: &rpr_extras::BeamIn) -> Option<(ExternalKey, (u8, u16))> {
        let sys = self.extras_in.systems.get(beam.system.as_ref()?)?;
        Some((key_of(sys.host?), (1, ((sys.index? as u16) << 8) | beam.id.unwrap_or(1) as u16)))
    }

    fn beam_input(&self, beam: &rpr_extras::BeamIn, jammer_class: bool) -> Option<ExternalInput> {
        let (key, slot) = self.beam_slot(beam)?;
        let sys = self.extras_in.systems.get(beam.system.as_ref()?)?;
        let jammer = jammer_class || beam.function == Some(rpr_extras::jamming_beam()) || sys.function == Some(rpr_extras::jammer_function());
        let emitter = RemoteEmitter { what: if jammer { EmitterKind::Jammer } else { EmitterKind::Radar }, freq_hz: beam.freq_hz?, bw_hz: beam.range_hz.unwrap_or(1e6), erp_w: crate::dis_extras::watts(beam.erp_dbm.unwrap_or(30.0)), duty: 1.0 };
        Some(ExternalInput::Emitter { key, slot, emitter: Some(emitter) })
    }

    /// Another federate's radio, emitter system or beam.
    fn reflect_extra(&mut self, class: &str, h: Handle, name: String, v: &BTreeMap<String, Vec<u8>>) {
        let bad = match class {
            rpr_extras::RADIO_TRANSMITTER => {
                let r = self.extras_in.radios.entry(h).or_default();
                let res = r.merge(v);
                if let (Some(host), Some(index), Some(status)) = (r.host, r.index, r.status) {
                    let emitter = (status == (crate::extras::RadioState::Transmitting as u8)).then(|| RemoteEmitter {
                        what: EmitterKind::Radio,
                        freq_hz: r.freq_hz.unwrap_or(0.0),
                        bw_hz: r.bw_hz.unwrap_or(25e3),
                        erp_w: crate::dis_extras::watts(r.power_dbm.unwrap_or(30.0)),
                        duty: 1.0,
                    });
                    self.inbox.push(ExternalInput::Emitter { key: key_of(host), slot: (0, index), emitter });
                }
                res.err()
            }
            rpr_extras::EMITTER_SYSTEM => {
                let res = self.extras_in.systems.entry(name.clone()).or_default().merge(v);
                // Beams that came before their system.
                let waiting: Vec<(rpr_extras::BeamIn, bool)> = self.extras_in.beams.values().filter(|(b, _)| b.system.as_deref() == Some(&name)).cloned().collect();
                for (b, j) in waiting {
                    self.inbox.extend(self.beam_input(&b, j));
                }
                res.err()
            }
            rpr_extras::RADAR_BEAM | rpr_extras::JAMMER_BEAM => {
                let entry = self.extras_in.beams.entry(h).or_insert_with(|| (Default::default(), class == rpr_extras::JAMMER_BEAM));
                let res = entry.0.merge(v);
                let (b, j) = entry.clone();
                self.inbox.extend(self.beam_input(&b, j));
                res.err()
            }
            _ => None,
        };
        if let Some(e) = bad {
            self.notes.push(format!("{name}: {e}"));
        }
    }

    /// Resign (deleting our objects) and destroy the federation if we were last.
    pub fn leave(&self) -> Result<(), RtiError> {
        self.rti.resign()?;
        self.rti.destroy_federation(&self.settings.federation)?;
        Ok(())
    }
}

fn key_of(id: [u16; 3]) -> ExternalKey {
    ExternalKey { site: id[0], application: id[1], entity: id[2] }
}

/// HLAunicodeString decoding, for tools and tests.
pub fn unicode_of(b: &[u8]) -> String {
    Dec::new(b).unicode().unwrap_or_default()
}

/// An entity's name for people: its label, or its scenario id.
fn callsign(sim: &Sim, id: EntityId) -> String {
    sim.ctx.order.get(id.index()).and_then(|&e| sim.world.get::<bgws_sim::components::Ident>(e)).map(|i| if i.label.is_empty() { i.name.clone() } else { i.label.clone() }).unwrap_or_default()
}
