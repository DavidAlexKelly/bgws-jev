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
        rti.create_federation(&settings.federation, &settings.foms)?;
        rti.join(&settings.federate, "BGWS", &settings.federation, &settings.foms)?;
        let mut classes = vec![];
        for name in rpr::CLASSES {
            let handle = rti.object_class(name)?;
            let attrs = rpr::attributes_for(name);
            let (by_name, by_handle) = names_to_handles(&attrs, |a| rti.attribute(&handle, a))?;
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
            extras_out: ExtrasOut::default(),
            extras_in: ExtrasIn::default(),
            ours: BTreeMap::new(),
            our_names: BTreeMap::new(),
            remote: BTreeMap::new(),
            name_to_key: BTreeMap::new(),
            key_to_name: BTreeMap::new(),
            inbox: vec![],
            notes: vec![],
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
                    if !self.rti.reserve_name(&name, self.settings.session.response_timeout)? {
                        self.notes.push(format!("the RTI would not reserve {name}; {} is not published", t.marking));
                        self.ours.insert(t.id, (class, Handle::default()));
                        continue;
                    }
                    let h = self.rti.register_with_name(&self.classes[class].handle, &name)?;
                    self.ours.insert(t.id, (class, h.clone()));
                    self.our_names.insert(name, t.id);
                    h
                }
            };
            if instance.0.is_empty() {
                continue;
            }
            let c = &self.classes[class];
            let values: Vec<(Handle, Vec<u8>)> = rpr::entity_attributes(&t, c.name).into_iter().filter_map(|(k, v)| c.by_name.get(k).map(|h| (h.clone(), v))).collect();
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

    /// Before a second: what came in, as inputs to apply (and record).
    pub fn inbound(&mut self, sim: &Sim) -> Vec<ExternalInput> {
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
