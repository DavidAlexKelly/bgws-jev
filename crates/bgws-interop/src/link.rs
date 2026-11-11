//! A game's DIS link: what it publishes each second and what it hears.
//!
//! The host calls [`DisLink::tick`] after each second with that second's
//! events (to publish states, fires and detonations), and
//! [`DisLink::inbound`] before each second (for what came in, as inputs to
//! apply and record).

use crate::dis::{encode, DisPublisher};
use crate::dr::DrSettings;
use crate::import::Importer;
use bgws_core::LogEvent;
use bgws_scenario::v4::InteropSpec;
use bgws_sim::remote::ExternalInput;
use bgws_sim::Sim;
use dis_rs::acknowledge::model::Acknowledge;
use dis_rs::enumerations::{AcknowledgeFlag, PduType, ResponseFlag};
use dis_rs::model::{Pdu, PduHeader, Timestamp};
use dis_rs::BodyRaw;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};

/// Receives DIS on a port (any version the parser knows: 6 and 7).
pub struct DisReceiver {
    socket: UdpSocket,
    buf: Vec<u8>,
    /// Datagrams that did not parse.
    pub bad: u64,
}

impl DisReceiver {
    /// Listen on `bind` (`host:port`); with `group`, join that IPv4
    /// multicast group too.
    pub fn new(bind: &str, group: Option<Ipv4Addr>) -> std::io::Result<DisReceiver> {
        let socket = UdpSocket::bind(bind)?;
        socket.set_nonblocking(true)?;
        if let Some(g) = group {
            socket.join_multicast_v4(&g, &Ipv4Addr::UNSPECIFIED)?;
        }
        Ok(DisReceiver { socket, buf: vec![0; 65_536], bad: 0 })
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// Every PDU waiting now.
    pub fn poll(&mut self) -> Vec<Pdu> {
        let mut out = Vec::new();
        while let Ok((n, _)) = self.socket.recv_from(&mut self.buf) {
            match dis_rs::parse(&self.buf[..n]) {
                Ok(p) => out.extend(p),
                Err(_) => self.bad += 1,
            }
        }
        out
    }
}

/// Publisher, receiver and importer together.
pub struct DisLink {
    pub publisher: DisPublisher,
    pub receiver: Option<DisReceiver>,
    importer: Option<Importer>,
    /// Inputs taken in so far.
    pub received: u64,
}

impl DisLink {
    /// Send to `dest`; with `listen`, also receive on it (and join `group`).
    pub fn new(dest: &str, listen: Option<&str>, group: Option<Ipv4Addr>, spec: InteropSpec, settings: DrSettings) -> std::io::Result<DisLink> {
        let publisher = DisPublisher::new(dest, spec, settings)?;
        let receiver = listen.map(|l| DisReceiver::new(l, group)).transpose()?;
        Ok(DisLink { publisher, receiver, importer: None, received: 0 })
    }

    /// From a publisher and a receiver already made (a receiver bound to a
    /// free port, in a test).
    pub fn from_parts(publisher: DisPublisher, receiver: Option<DisReceiver>) -> DisLink {
        DisLink { publisher, receiver, importer: None, received: 0 }
    }

    /// After a second: publish it.
    pub fn tick(&mut self, sim: &Sim, events: &[LogEvent]) -> std::io::Result<usize> {
        let mut n = self.publisher.publish(sim, events)?;
        // Acknowledge the exercise control we were sent.
        if let Some(imp) = self.importer.as_mut() {
            for a in std::mem::take(&mut imp.acks) {
                let body = Acknowledge::builder()
                    .with_origination_id(a.from)
                    .with_receiving_id(a.to)
                    .with_acknowledge_flag(AcknowledgeFlag::from(a.flag))
                    .with_response_flag(ResponseFlag::AbleToComply)
                    .with_request_id(a.request_id)
                    .build()
                    .into_pdu_body();
                let pdu = Pdu::finalize_from_parts(PduHeader::new_v7(self.publisher.spec.exercise, PduType::Acknowledge), body, Timestamp::new(crate::dis::relative_timestamp(sim.ctx.now.secs())));
                self.publisher.send(&encode(&pdu))?;
                n += 1;
            }
        }
        Ok(n)
    }

    /// Before a second: what came in, as inputs to apply (and record).
    pub fn inbound(&mut self, sim: &Sim) -> Vec<ExternalInput> {
        let Some(rx) = self.receiver.as_mut() else { return vec![] };
        let pdus = rx.poll();
        let imp = self.importer.get_or_insert_with(|| Importer::new(sim, self.publisher.spec.clone()));
        let inputs: Vec<ExternalInput> = pdus.iter().flat_map(|p| imp.from_pdu(sim, p)).collect();
        self.received += inputs.len() as u64;
        inputs
    }

    /// Stand-ins chosen since the last call (for the log).
    pub fn notes(&mut self) -> Vec<String> {
        self.importer.as_mut().map(|i| std::mem::take(&mut i.notes)).unwrap_or_default()
    }
}
