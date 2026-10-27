//! Communications and networks (RUST_ENGINE.md §8.5; research R2).
//!
//! - **Bearers** with real figures: SINCGARS-like VHF at 0.2–40 km by power
//!   **[P4]**, UHF, HF NVIS **[P5]**, satcom GEO and LEO with latency **[P7]**,
//!   a Link 16-like TDMA data link (7.8125 ms slots, 12.8 min epoch,
//!   31.6–115.2 kbps **[P6]**), a line-of-sight mesh, field wire and fibre,
//!   and couriers that walk or drive.
//! - **Messages have sizes and kinds** (voice contact report, data-link track,
//!   image, plan/order). A link's success comes from its SINR through the
//!   bearer's waveform: Eb/N₀ → BER → packet success → message success, with
//!   ARQ retransmissions on data and "say again" on voice.
//! - **Nets have queues and channel access:** push-to-talk contention on
//!   voice nets, assigned slots on TDMA, random access (ALOHA-like
//!   collisions) on the mesh, a dedicated pipe on wire, fibre and satcom
//!   terminals.
//! - **Routing** runs over a `petgraph` graph whose edges carry the expected
//!   hop time for this message; it degrades as links fail. Mesh nodes all
//!   forward; elsewhere only relays (HQs) pass traffic between nets.
//! - **PACE:** every message kind has a primary, alternate, contingency and
//!   emergency bearer; the sender falls back after a switch delay.
//! - **Crypto:** a per-message delay by crypto class; compromised keys mark
//!   what the enemy can read (§8.12 cyber effect).
//! - **Signature:** every radio transmission is an [`Emission`] for ESM (§8.2, §8.7).
//!
//! [`CommsNetwork`] is a discrete-event state machine: [`CommsNetwork::send`]
//! a message at a time, then [`CommsNetwork::step`] to a time, and get back
//! deliveries and losses with the reasons (queue wait, airtime, retries, link
//! SINR) for the AAR (rule 10). Every random draw is keyed by
//! (message, hop, attempt), so the outcome never depends on processing order.

use crate::math::{db_to_ratio, q_function, ratio_to_db, soft_step};
use bgws_core::units::{BOLTZMANN, STANDARD_TEMPERATURE_K};
use bgws_core::{figure, hazard_probability, Enu, Figure, Figures, MessageId, Seed, SimDuration, SimTime, StreamKey};
use petgraph::graph::{NodeIndex, UnGraph};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
use thiserror::Error;

/// Figures for communications (rule 3).
pub fn figures() -> Vec<Figure> {
    vec![
        // VHF combat net radio [P4].
        figure!("comms.vhf.range_low_m", 400.0, "m", 200.0, 400.0, "P4", High, "SINCGARS low power: 200–400 m"),
        figure!("comms.vhf.range_medium_m", 5_000.0, "m", 400.0, 5_000.0, "P4", High, "SINCGARS medium power: 0.4–5 km"),
        figure!("comms.vhf.range_high_m", 10_000.0, "m", 5_000.0, 10_000.0, "P4", High, "SINCGARS high power: 5–10 km"),
        figure!("comms.vhf.range_vehicle_m", 40_000.0, "m", 10_000.0, 40_000.0, "P4", High, "SINCGARS vehicle amplifier: 10–40 km"),
        figure!("comms.vhf.rate_bps", 16_000.0, "bit/s", 1_200.0, 16_000.0, "default", Default, "VHF CNR data rate"),
        figure!("comms.vhf.bandwidth_hz", 25_000.0, "Hz", 12_500.0, 25_000.0, "default", Default, "VHF channel bandwidth"),
        figure!("comms.vhf.freq_hz", 60e6, "Hz", 30e6, 88e6, "default", Default, "VHF CNR band 30–88 MHz"),
        figure!("comms.vhf.power_w", 4.0, "W", 0.0005, 50.0, "default", Default, "manpack high power"),
        figure!("comms.uhf.range_m", 15_000.0, "m", 2_000.0, 50_000.0, "default", Default, "UHF LOS nominal range"),
        figure!("comms.uhf.rate_bps", 64_000.0, "bit/s", 9_600.0, 1e6, "default", Default, "UHF LOS data rate"),
        figure!("comms.uhf.bandwidth_hz", 100_000.0, "Hz", 25_000.0, 5e6, "default", Default, "UHF channel bandwidth"),
        figure!("comms.uhf.freq_hz", 300e6, "Hz", 225e6, 400e6, "default", Default, "UHF band"),
        figure!("comms.uhf.power_w", 10.0, "W", 1.0, 50.0, "default", Default, "UHF transmit power"),
        figure!("comms.hf.range_m", 300_000.0, "m", 35_000.0, 450_000.0, "P5", Medium, "HF NVIS reach 35–450 km"),
        figure!("comms.hf.rate_bps", 2_400.0, "bit/s", 300.0, 9_600.0, "default", Default, "HF data modem rate"),
        figure!("comms.hf.bandwidth_hz", 3_000.0, "Hz", 3_000.0, 24_000.0, "default", Default, "HF channel bandwidth"),
        figure!("comms.hf.freq_hz", 6e6, "Hz", 2e6, 10e6, "P5", Medium, "NVIS 2–10 MHz"),
        figure!("comms.hf.power_w", 20.0, "W", 1.0, 400.0, "default", Default, "HF manpack power"),
        figure!("comms.satcom_geo.latency_s", 0.27, "s", 0.25, 0.28, "R2", Low, "GEO TACSAT one-way latency 250–280 ms [default in R2]"),
        figure!("comms.satcom_geo.rate_bps", 64_000.0, "bit/s", 2_400.0, 2e6, "default", Default, "GEO terminal throughput"),
        figure!("comms.satcom_leo.latency_s", 0.05, "s", 0.025, 0.1, "P7", High, "Starlink-like latency 25–100 ms"),
        figure!("comms.satcom_leo.rate_bps", 10e6, "bit/s", 1e6, 220e6, "P7", High, "crowded front-line cell ~10 Mbps/terminal"),
        figure!("comms.satcom.bandwidth_hz", 5e6, "Hz", 25_000.0, 250e6, "default", Default, "satcom channel bandwidth"),
        figure!("comms.satcom.freq_hz", 12e9, "Hz", 250e6, 30e9, "default", Default, "satcom uplink band (Ku)"),
        figure!("comms.satcom.power_w", 10.0, "W", 1.0, 200.0, "default", Default, "terminal transmit power"),
        figure!("comms.tdl.slot_s", 0.007_812_5, "s", 0.007_812_5, 0.007_812_5, "P6", High, "Link 16 time slot"),
        figure!("comms.tdl.epoch_s", 768.0, "s", 768.0, 768.0, "P6", High, "Link 16 epoch, 12.8 min"),
        figure!("comms.tdl.frame_s", 12.0, "s", 12.0, 12.0, "P6", High, "Link 16 frame, 1,536 slots"),
        figure!("comms.tdl.rate_bps", 57_600.0, "bit/s", 31_600.0, 115_200.0, "P6", High, "Link 16 data rate 31.6/57.6/115.2 kbps"),
        figure!("comms.tdl.slots_per_frame", 32.0, "1", 1.0, 1_536.0, "default", Default, "slots a terminal is assigned per frame"),
        figure!("comms.tdl.range_m", 480_000.0, "m", 100_000.0, 600_000.0, "R2", Medium, "Link 16 range >300 miles (air)"),
        figure!("comms.tdl.bandwidth_hz", 3e6, "Hz", 3e6, 3e6, "default", Default, "Link 16 spread bandwidth"),
        figure!("comms.tdl.freq_hz", 1.0e9, "Hz", 960e6, 1_215e6, "default", Default, "Lx band"),
        figure!("comms.tdl.power_w", 200.0, "W", 25.0, 1_000.0, "default", Default, "terminal transmit power"),
        figure!("comms.mesh.range_m", 3_000.0, "m", 500.0, 20_000.0, "default", Default, "mesh radio LOS range"),
        figure!("comms.mesh.rate_bps", 1e6, "bit/s", 50_000.0, 20e6, "default", Default, "mesh data rate"),
        figure!("comms.mesh.bandwidth_hz", 1.25e6, "Hz", 200_000.0, 20e6, "default", Default, "mesh channel bandwidth"),
        figure!("comms.mesh.freq_hz", 1.4e9, "Hz", 225e6, 6e9, "default", Default, "mesh radio band"),
        figure!("comms.mesh.power_w", 2.0, "W", 0.1, 20.0, "default", Default, "mesh transmit power"),
        figure!("comms.mesh.hop_s", 0.02, "s", 0.001, 0.5, "default", Default, "mesh per-hop processing"),
        figure!("comms.mesh.backoff_window_s", 0.2, "s", 0.01, 2.0, "default", Default, "random-access backoff window"),
        figure!("comms.wire.rate_bps", 16_000.0, "bit/s", 1_200.0, 64_000.0, "default", Default, "field wire rate"),
        figure!("comms.fibre.rate_bps", 100e6, "bit/s", 1e6, 10e9, "default", Default, "tactical fibre rate"),
        figure!("comms.line.latency_s", 0.001, "s", 0.0, 0.05, "default", Default, "wire/fibre switching latency"),
        figure!("comms.courier.speed_mps", 1.2, "m/s", 0.8, 20.0, "default", Default, "runner on foot (vehicle up to 20 m/s)"),
        figure!("comms.courier.loss_rate_per_h", 0.02, "1/h", 0.0, 1.0, "default", Default, "hazard a courier is killed or lost"),
        // Messages.
        figure!("comms.voice.report_s", 20.0, "s", 5.0, 60.0, "R2", Low, "voice contact report airtime; keep <20 s against DF"),
        figure!("comms.voice.threshold_db", 10.0, "dB", 4.0, 16.0, "default", Default, "SINR for 50% intelligible voice"),
        figure!("comms.voice.width_db", 2.0, "dB", 0.5, 5.0, "default", Default, "voice intelligibility transition width"),
        figure!("comms.msg.voice_as_data_bits", 2_000.0, "bit", 500.0, 10_000.0, "default", Default, "a contact report sent as text"),
        figure!("comms.msg.track_bits", 225.0, "bit", 70.0, 1_000.0, "R2", Low, "a J-series track message, about three words"),
        figure!("comms.msg.image_bits", 2e6, "bit", 1e5, 1e8, "default", Default, "a compressed still image"),
        figure!("comms.msg.order_bits", 80_000.0, "bit", 8_000.0, 1e6, "default", Default, "a structured plan or order"),
        figure!("comms.packet_bits", 1_024.0, "bit", 128.0, 12_000.0, "default", Default, "ARQ packet size"),
        figure!("comms.max_retries", 3.0, "1", 0.0, 10.0, "default", Default, "retransmissions per packet or 'say again's"),
        figure!("comms.coding_gain_db", 5.0, "dB", 0.0, 9.0, "default", Default, "FEC coding gain on data waveforms"),
        figure!("comms.noise_figure_db", 8.0, "dB", 2.0, 15.0, "default", Default, "receiver noise figure"),
        figure!("comms.antenna_gain_dbi", 2.0, "dBi", -5.0, 30.0, "default", Default, "tactical whip antenna gain"),
        figure!("comms.range_slope", 40.0, "dB/decade", 20.0, 40.0, "default", Default, "SINR slope with range for nominal links"),
        figure!("comms.route_min_p", 0.05, "fraction", 0.0, 0.5, "default", Default, "worst hop success still used for routing"),
        // Channel access.
        figure!("comms.ptt.turnaround_s", 0.5, "s", 0.1, 2.0, "default", Default, "key-up and turnaround per voice transmission"),
        figure!("comms.ptt.contention_s", 3.0, "s", 0.0, 15.0, "default", Default, "max wait for a break on a busy voice net"),
        figure!("comms.queue_capacity", 50.0, "1", 1.0, 1_000.0, "default", Default, "messages a channel queue holds"),
        figure!("comms.relay_s", 5.0, "s", 0.5, 60.0, "default", Default, "HQ relay: receive, decide, pass on"),
        figure!("comms.pace_switch_s", 30.0, "s", 1.0, 600.0, "default", Default, "time to switch to the next PACE bearer"),
        // Crypto.
        figure!("comms.crypto.voice_s", 0.5, "s", 0.0, 3.0, "default", Default, "COMSEC voice sync delay"),
        figure!("comms.crypto.data_s", 1.0, "s", 0.0, 10.0, "default", Default, "Type-1 data encrypt+decrypt"),
        figure!("comms.crypto.commercial_s", 0.2, "s", 0.0, 2.0, "default", Default, "commercial (AES) data encryption"),
    ]
}

/// A comms node: a radio, a terminal, an HQ's signal centre.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct NodeId(pub u32);

/// A net: a set of nodes sharing a bearer and a channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct NetId(pub u32);

/// Kinds of bearer (§8.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BearerKind {
    Vhf,
    Uhf,
    Hf,
    SatcomGeo,
    SatcomLeo,
    Tdl,
    Mesh,
    Wire,
    Fibre,
    Courier,
}

impl BearerKind {
    /// Whether the bearer carries analogue/CVSD voice (else voice reports go as text).
    pub fn carries_voice(self) -> bool {
        matches!(self, BearerKind::Vhf | BearerKind::Uhf | BearerKind::Hf | BearerKind::SatcomGeo)
    }
    /// Whether transmissions radiate (wire, fibre and couriers don't).
    pub fn radiates(self) -> bool {
        !matches!(self, BearerKind::Wire | BearerKind::Fibre | BearerKind::Courier)
    }
}

/// SINCGARS-like power settings **[P4]**.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VhfPower {
    Low,
    Medium,
    High,
    VehicleAmplifier,
}

/// How a net shares its channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelAccess {
    /// One talker at a time; wait for a break (voice nets, HF).
    PushToTalk,
    /// Fixed slot assignment per terminal (Link 16-like).
    Tdma,
    /// Random access with collisions (mesh).
    RandomAccess,
    /// A dedicated pipe per sender (wire, fibre, satcom terminal).
    Dedicated,
    /// A person carries it.
    Courier,
}

/// Digital modulation families with closed-form BER.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modulation {
    /// Coherent BPSK: ½·erfc(√(Eb/N₀)).
    Bpsk,
    /// Gray-coded QPSK: same BER per bit as BPSK.
    Qpsk,
    /// Non-coherent binary FSK: ½·e^(−Eb/2N₀).
    NcFsk,
    /// Differential PSK: ½·e^(−Eb/N₀).
    Dpsk,
}

/// A waveform: modulation, occupied bandwidth and coding gain.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Waveform {
    pub modulation: Modulation,
    pub bandwidth_hz: f64,
    pub coding_gain_db: f64,
}

/// Voice intelligibility as a soft threshold on SINR.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct VoiceSpec {
    pub threshold_db: f64,
    pub width_db: f64,
    /// Airtime of one voice contact report.
    pub report_s: f64,
}

/// A bearer's performance record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BearerSpec {
    pub kind: BearerKind,
    pub access: ChannelAccess,
    pub rate_bps: f64,
    /// Fixed one-way latency (propagation, satellite hop, switching).
    pub latency_s: f64,
    pub freq_hz: f64,
    pub power_w: f64,
    pub antenna_gain_dbi: f64,
    pub waveform: Waveform,
    pub voice: Option<VoiceSpec>,
    /// Nominal range, used when no propagation model supplies SINR.
    pub nominal_range_m: f64,
    pub max_retries: u32,
    /// TDMA only: slots per frame a terminal owns, slot length and frame length.
    pub tdma: Option<TdmaSpec>,
    /// Courier only: travel speed and loss hazard per hour.
    pub courier: Option<CourierSpec>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TdmaSpec {
    pub slot_s: f64,
    pub frame_s: f64,
    pub epoch_s: f64,
    pub slots_per_frame: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CourierSpec {
    pub speed_mps: f64,
    pub loss_rate_per_h: f64,
}

/// Bit error rate for a modulation at Eb/N₀ (linear).
pub fn bit_error_rate(m: Modulation, ebn0: f64) -> f64 {
    let ebn0 = ebn0.max(0.0);
    match m {
        Modulation::Bpsk | Modulation::Qpsk => q_function((2.0 * ebn0).sqrt()),
        Modulation::NcFsk => 0.5 * (-ebn0 / 2.0).exp(),
        Modulation::Dpsk => 0.5 * (-ebn0).exp(),
    }
    .clamp(0.0, 0.5)
}

impl BearerSpec {
    /// The bearer record for a kind, from figures. VHF uses high power;
    /// see [`BearerSpec::vhf`] for other settings.
    pub fn from_figures(kind: BearerKind, f: &Figures) -> Self {
        let retries = f.get("comms.max_retries").round() as u32;
        let coding = f.get("comms.coding_gain_db");
        let gain = f.get("comms.antenna_gain_dbi");
        let voice = Some(VoiceSpec {
            threshold_db: f.get("comms.voice.threshold_db"),
            width_db: f.get("comms.voice.width_db"),
            report_s: f.get("comms.voice.report_s"),
        });
        let wf = |m, bw: f64| Waveform { modulation: m, bandwidth_hz: bw, coding_gain_db: coding };
        let base = |access, rate, lat, freq, power, waveform, voice, range| BearerSpec {
            kind,
            access,
            rate_bps: rate,
            latency_s: lat,
            freq_hz: freq,
            power_w: power,
            antenna_gain_dbi: gain,
            waveform,
            voice,
            nominal_range_m: range,
            max_retries: retries,
            tdma: None,
            courier: None,
        };
        match kind {
            BearerKind::Vhf => Self::vhf(VhfPower::High, f),
            BearerKind::Uhf => base(
                ChannelAccess::PushToTalk,
                f.get("comms.uhf.rate_bps"),
                0.0,
                f.get("comms.uhf.freq_hz"),
                f.get("comms.uhf.power_w"),
                wf(Modulation::Bpsk, f.get("comms.uhf.bandwidth_hz")),
                voice,
                f.get("comms.uhf.range_m"),
            ),
            BearerKind::Hf => base(
                ChannelAccess::PushToTalk,
                f.get("comms.hf.rate_bps"),
                0.002,
                f.get("comms.hf.freq_hz"),
                f.get("comms.hf.power_w"),
                wf(Modulation::Dpsk, f.get("comms.hf.bandwidth_hz")),
                voice,
                f.get("comms.hf.range_m"),
            ),
            BearerKind::SatcomGeo | BearerKind::SatcomLeo => {
                let geo = kind == BearerKind::SatcomGeo;
                base(
                    ChannelAccess::Dedicated,
                    f.get(if geo { "comms.satcom_geo.rate_bps" } else { "comms.satcom_leo.rate_bps" }),
                    f.get(if geo { "comms.satcom_geo.latency_s" } else { "comms.satcom_leo.latency_s" }),
                    f.get("comms.satcom.freq_hz"),
                    f.get("comms.satcom.power_w"),
                    wf(Modulation::Qpsk, f.get("comms.satcom.bandwidth_hz")),
                    if geo { voice } else { None },
                    f64::INFINITY,
                )
            }
            BearerKind::Tdl => {
                let mut b = base(
                    ChannelAccess::Tdma,
                    f.get("comms.tdl.rate_bps"),
                    0.0,
                    f.get("comms.tdl.freq_hz"),
                    f.get("comms.tdl.power_w"),
                    wf(Modulation::Bpsk, f.get("comms.tdl.bandwidth_hz")),
                    None,
                    f.get("comms.tdl.range_m"),
                );
                b.tdma = Some(TdmaSpec {
                    slot_s: f.get("comms.tdl.slot_s"),
                    frame_s: f.get("comms.tdl.frame_s"),
                    epoch_s: f.get("comms.tdl.epoch_s"),
                    slots_per_frame: f.get("comms.tdl.slots_per_frame").round().max(1.0) as u32,
                });
                b
            }
            BearerKind::Mesh => base(
                ChannelAccess::RandomAccess,
                f.get("comms.mesh.rate_bps"),
                f.get("comms.mesh.hop_s"),
                f.get("comms.mesh.freq_hz"),
                f.get("comms.mesh.power_w"),
                wf(Modulation::Qpsk, f.get("comms.mesh.bandwidth_hz")),
                None,
                f.get("comms.mesh.range_m"),
            ),
            BearerKind::Wire | BearerKind::Fibre => base(
                ChannelAccess::Dedicated,
                f.get(if kind == BearerKind::Wire { "comms.wire.rate_bps" } else { "comms.fibre.rate_bps" }),
                f.get("comms.line.latency_s"),
                0.0,
                0.0,
                wf(Modulation::Bpsk, 1.0),
                None,
                f64::INFINITY,
            ),
            BearerKind::Courier => {
                let mut b = base(ChannelAccess::Courier, f64::INFINITY, 0.0, 0.0, 0.0, wf(Modulation::Bpsk, 1.0), None, f64::INFINITY);
                b.courier = Some(CourierSpec {
                    speed_mps: f.get("comms.courier.speed_mps"),
                    loss_rate_per_h: f.get("comms.courier.loss_rate_per_h"),
                });
                b
            }
        }
    }

    /// SINCGARS-like VHF at a power setting; nominal range from **[P4]**.
    pub fn vhf(power: VhfPower, f: &Figures) -> Self {
        let range = match power {
            VhfPower::Low => f.get("comms.vhf.range_low_m"),
            VhfPower::Medium => f.get("comms.vhf.range_medium_m"),
            VhfPower::High => f.get("comms.vhf.range_high_m"),
            VhfPower::VehicleAmplifier => f.get("comms.vhf.range_vehicle_m"),
        };
        BearerSpec {
            kind: BearerKind::Vhf,
            access: ChannelAccess::PushToTalk,
            rate_bps: f.get("comms.vhf.rate_bps"),
            latency_s: 0.0,
            freq_hz: f.get("comms.vhf.freq_hz"),
            power_w: f.get("comms.vhf.power_w"),
            antenna_gain_dbi: f.get("comms.antenna_gain_dbi"),
            waveform: Waveform {
                modulation: Modulation::NcFsk,
                bandwidth_hz: f.get("comms.vhf.bandwidth_hz"),
                coding_gain_db: f.get("comms.coding_gain_db"),
            },
            voice: Some(VoiceSpec {
                threshold_db: f.get("comms.voice.threshold_db"),
                width_db: f.get("comms.voice.width_db"),
                report_s: f.get("comms.voice.report_s"),
            }),
            nominal_range_m: range,
            max_retries: f.get("comms.max_retries").round() as u32,
            tdma: None,
            courier: None,
        }
    }

    /// Bits per TDMA slot (rate × slot length).
    pub fn bits_per_slot(&self) -> f64 {
        self.tdma.map(|t| self.rate_bps * t.slot_s).unwrap_or(f64::INFINITY)
    }
}

/// What a link of this bearer achieves for one message at one SINR.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinkQuality {
    pub sinr_db: f64,
    /// Eb/N₀ after processing and coding gain (data), dB.
    pub ebn0_db: f64,
    pub ber: f64,
    /// Per-attempt success: a packet (data) or a whole transmission (voice).
    pub p_attempt: f64,
    pub packets: u32,
    /// Probability the message gets across within the retry budget.
    pub p_success: f64,
    /// Expected transmissions per packet (data ARQ) or per message (voice).
    pub expected_tx: f64,
    pub voice: bool,
}

/// Link success from SINR through the waveform (§8.5): SINR → Eb/N₀ → BER →
/// packet success; ARQ with `max_retries` per packet. Voice uses a soft
/// intelligibility threshold and whole-message "say again".
pub fn link_quality(b: &BearerSpec, sinr_db: f64, size_bits: f64, voice: bool, packet_bits: f64) -> LinkQuality {
    let tries = b.max_retries as f64 + 1.0;
    if voice {
        if let Some(v) = b.voice {
            let p = soft_step(sinr_db, v.threshold_db, v.width_db);
            let p_success = 1.0 - (1.0 - p).powf(tries);
            let expected_tx = if p > 0.0 { p_success / p } else { tries };
            return LinkQuality { sinr_db, ebn0_db: f64::NAN, ber: f64::NAN, p_attempt: p, packets: 1, p_success, expected_tx, voice: true };
        }
    }
    let ebn0_db = sinr_db + ratio_to_db(b.waveform.bandwidth_hz / b.rate_bps) + b.waveform.coding_gain_db;
    let ber = bit_error_rate(b.waveform.modulation, db_to_ratio(ebn0_db));
    let pkt = packet_bits.min(size_bits).max(1.0);
    let packets = (size_bits / pkt).ceil().max(1.0);
    let p_packet = (1.0 - ber).powf(pkt);
    let p_pkt_ok = 1.0 - (1.0 - p_packet).powf(tries);
    let p_success = p_pkt_ok.powf(packets);
    let expected_tx = if p_packet > 0.0 { p_pkt_ok / p_packet } else { tries };
    LinkQuality { sinr_db, ebn0_db, ber, p_attempt: p_packet, packets: packets as u32, p_success, expected_tx, voice: false }
}

/// The SINR at which a reference message gets across half the time.
pub fn required_sinr_db(b: &BearerSpec, size_bits: f64, voice: bool, packet_bits: f64) -> f64 {
    crate::math::bisect(-60.0, 80.0, |s| link_quality(b, s, size_bits, voice, packet_bits).p_success - 0.5).unwrap_or(0.0)
}

/// SINR on a link with no propagation model: the reference message's
/// 50% SINR at the nominal range, rising `slope` dB per decade closer **[P4]**.
pub fn nominal_sinr_db(b: &BearerSpec, distance_m: f64, f: &Figures) -> f64 {
    if !b.nominal_range_m.is_finite() {
        return 60.0;
    }
    let packet = f.get("comms.packet_bits");
    let reference = if b.voice.is_some() { 0.0 } else { f.get("comms.msg.voice_as_data_bits") };
    let req = required_sinr_db(b, reference.max(1.0), b.voice.is_some(), packet);
    req + f.get("comms.range_slope") * (b.nominal_range_m / distance_m.max(1.0)).log10()
}

/// Kinds of message (§8.5), each with a default size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    VoiceContactReport,
    DataLinkTrack,
    Image,
    PlanOrder,
}

/// Crypto classes, each with a delay (§8.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CryptoClass {
    Clear,
    ComsecVoice,
    Type1Data,
    Commercial,
}

/// A message to send.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    pub kind: MessageKind,
    pub from: NodeId,
    pub to: NodeId,
    /// Lower is more urgent.
    pub priority: u8,
    pub crypto: CryptoClass,
    /// Size override; else the kind's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bits: Option<f64>,
}

/// Primary, alternate, contingency and emergency bearers for one message kind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PacePlan {
    pub levels: Vec<BearerKind>,
}

impl PacePlan {
    pub fn new(p: BearerKind, a: Option<BearerKind>, c: Option<BearerKind>, e: Option<BearerKind>) -> Self {
        Self { levels: [Some(p), a, c, e].into_iter().flatten().collect() }
    }
}

/// One hop's record (rule 10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HopRecord {
    pub from: NodeId,
    pub to: NodeId,
    pub net: Option<NetId>,
    pub bearer: BearerKind,
    pub enqueued_at: SimTime,
    pub start: SimTime,
    pub end: SimTime,
    pub queue_wait_s: f64,
    pub airtime_s: f64,
    pub attempts: u32,
    pub sinr_db: f64,
    pub p_success: f64,
    pub success: bool,
}

/// Why a bearer failed for a message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum FailReason {
    NoRoute,
    LinkFailed { net: Option<NetId>, from: NodeId, to: NodeId, sinr_db: f64, p_success: f64, attempts: u32 },
    QueueFull { net: NetId },
    CourierLost { travel_s: f64, p_loss: f64 },
    NodeDestroyed { node: NodeId },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BearerFailure {
    pub bearer: BearerKind,
    pub pace_level: usize,
    pub at: SimTime,
    pub failure: FailReason,
}

/// A message that arrived.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Delivery {
    pub msg: MessageId,
    pub kind: MessageKind,
    pub from: NodeId,
    pub to: NodeId,
    pub sent_at: SimTime,
    pub delivered_at: SimTime,
    pub bearer: BearerKind,
    pub pace_level: usize,
    pub crypto_delay_s: f64,
    pub hops: Vec<HopRecord>,
    pub fallbacks: Vec<BearerFailure>,
    /// Sent under a compromised key: the enemy can read it.
    pub compromised: bool,
}

impl Delivery {
    pub fn latency_s(&self) -> f64 {
        (self.delivered_at - self.sent_at).secs()
    }
}

/// A message that didn't arrive, and why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Loss {
    pub msg: MessageId,
    pub kind: MessageKind,
    pub from: NodeId,
    pub to: NodeId,
    pub sent_at: SimTime,
    pub lost_at: SimTime,
    pub reason: FailReason,
    pub hops: Vec<HopRecord>,
    pub fallbacks: Vec<BearerFailure>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum CommsOutcome {
    Delivered(Delivery),
    Lost(Loss),
}

impl CommsOutcome {
    pub fn msg(&self) -> MessageId {
        match self {
            CommsOutcome::Delivered(d) => d.msg,
            CommsOutcome::Lost(l) => l.msg,
        }
    }
}

/// An RF transmission, for ESM (§8.2 ↔ §8.7).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Emission {
    pub node: NodeId,
    pub net: Option<NetId>,
    pub bearer: BearerKind,
    pub freq_hz: f64,
    pub bandwidth_hz: f64,
    pub power_w: f64,
    pub start: SimTime,
    pub duration_s: f64,
    pub msg: MessageId,
}

/// Propagation supplied by the caller (bgws-prop, or a test model).
pub trait LinkPhysics {
    /// Path loss in dB from `from` to `to` at `freq_hz`.
    fn path_loss_db(&self, from: NodeId, to: NodeId, freq_hz: f64) -> f64;
    /// Interference and jamming at a receiver in this band, dBW, if any.
    fn interference_dbw(&self, _rx: NodeId, _freq_hz: f64, _bandwidth_hz: f64) -> Option<f64> {
        None
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum CommsError {
    #[error("message {0} sent at {1} before the network clock {2}")]
    InThePast(MessageId, SimTime, SimTime),
    #[error("unknown node {0:?}")]
    UnknownNode(NodeId),
    #[error("unknown net {0:?}")]
    UnknownNet(NetId),
    #[error("message {0} already sent")]
    Duplicate(MessageId),
}

/// A node's state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    /// Local position (for couriers and nominal links); optional.
    pub position: Option<Enu>,
    /// Relays traffic between nets (an HQ's signal centre).
    pub relay: bool,
    pub alive: bool,
    pub key_compromised: bool,
}

/// A net's state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Net {
    pub id: NetId,
    pub bearer: BearerSpec,
    pub members: BTreeSet<NodeId>,
    pub up: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Params {
    packet_bits: f64,
    route_min_p: f64,
    ptt_turnaround_s: f64,
    ptt_contention_s: f64,
    queue_capacity: usize,
    relay_s: f64,
    pace_switch_s: f64,
    backoff_s: f64,
    noise_figure_db: f64,
    crypto_voice_s: f64,
    crypto_data_s: f64,
    crypto_commercial_s: f64,
    sizes: BTreeMap<MessageKind, f64>,
    voice_as_data_bits: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RouteHop {
    from: NodeId,
    to: NodeId,
    net: Option<NetId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Flight {
    msg: Message,
    sent_at: SimTime,
    pace_level: usize,
    bearer: Option<BearerKind>,
    route: Vec<RouteHop>,
    hop: usize,
    hops: Vec<HopRecord>,
    fallbacks: Vec<BearerFailure>,
    crypto_delay_s: f64,
    enqueued_at: SimTime,
    done: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct ChannelKey {
    net: NetId,
    /// `Some` for per-sender channels (TDMA, dedicated); `None` for shared.
    node: Option<NodeId>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Channel {
    queue: Vec<(u8, SimTime, u64)>,
    busy_until: SimTime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum Ev {
    /// Pick a bearer at the flight's PACE level and route it.
    Attempt(u64),
    /// The message is ready to go on its next hop.
    Ready(u64),
    /// A channel may start its next transmission.
    Service(ChannelKey),
    /// The current hop's transmission arrived at its far end.
    Arrive(u64),
    /// The current hop failed.
    HopFailed(u64, FailReason),
}

/// The discrete-event communications network.
#[derive(Clone, Debug)]
pub struct CommsNetwork {
    seed: Seed,
    params: Params,
    now: SimTime,
    nodes: BTreeMap<NodeId, Node>,
    nets: BTreeMap<NetId, Net>,
    /// Directional SINR overrides: (net, tx, rx) → dB.
    links: BTreeMap<(NetId, NodeId, NodeId), f64>,
    pace: BTreeMap<MessageKind, PacePlan>,
    courier: BearerSpec,
    flights: BTreeMap<u64, Flight>,
    channels: BTreeMap<ChannelKey, Channel>,
    events: BTreeMap<(SimTime, u64), Ev>,
    heap: BinaryHeap<Reverse<(SimTime, u64)>>,
    seq: u64,
    emissions: Vec<Emission>,
    nominal_sinr: BTreeMap<(NetId, NodeId, NodeId), f64>,
}

impl CommsNetwork {
    /// A network with no nodes or nets. PACE defaults to VHF → satcom GEO →
    /// HF → courier for every kind; set your own with [`CommsNetwork::set_pace`].
    pub fn new(f: &Figures, seed: Seed) -> Self {
        let mut sizes = BTreeMap::new();
        sizes.insert(MessageKind::VoiceContactReport, f.get("comms.msg.voice_as_data_bits"));
        sizes.insert(MessageKind::DataLinkTrack, f.get("comms.msg.track_bits"));
        sizes.insert(MessageKind::Image, f.get("comms.msg.image_bits"));
        sizes.insert(MessageKind::PlanOrder, f.get("comms.msg.order_bits"));
        let params = Params {
            packet_bits: f.get("comms.packet_bits"),
            route_min_p: f.get("comms.route_min_p"),
            ptt_turnaround_s: f.get("comms.ptt.turnaround_s"),
            ptt_contention_s: f.get("comms.ptt.contention_s"),
            queue_capacity: f.get("comms.queue_capacity").round() as usize,
            relay_s: f.get("comms.relay_s"),
            pace_switch_s: f.get("comms.pace_switch_s"),
            backoff_s: f.get("comms.mesh.backoff_window_s"),
            noise_figure_db: f.get("comms.noise_figure_db"),
            crypto_voice_s: f.get("comms.crypto.voice_s"),
            crypto_data_s: f.get("comms.crypto.data_s"),
            crypto_commercial_s: f.get("comms.crypto.commercial_s"),
            voice_as_data_bits: f.get("comms.msg.voice_as_data_bits"),
            sizes,
        };
        let default_pace = PacePlan::new(BearerKind::Vhf, Some(BearerKind::SatcomGeo), Some(BearerKind::Hf), Some(BearerKind::Courier));
        let pace = [MessageKind::VoiceContactReport, MessageKind::DataLinkTrack, MessageKind::Image, MessageKind::PlanOrder]
            .into_iter()
            .map(|k| (k, default_pace.clone()))
            .collect();
        CommsNetwork {
            seed,
            params,
            now: SimTime::ZERO,
            nodes: BTreeMap::new(),
            nets: BTreeMap::new(),
            links: BTreeMap::new(),
            pace,
            courier: BearerSpec::from_figures(BearerKind::Courier, f),
            flights: BTreeMap::new(),
            channels: BTreeMap::new(),
            events: BTreeMap::new(),
            heap: BinaryHeap::new(),
            seq: 0,
            emissions: Vec::new(),
            nominal_sinr: BTreeMap::new(),
        }
    }

    pub fn now(&self) -> SimTime {
        self.now
    }

    pub fn add_node(&mut self, id: NodeId, position: Option<Enu>, relay: bool) {
        self.nodes.insert(id, Node { id, position, relay, alive: true, key_compromised: false });
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id)
    }

    /// Add a net. For point-to-point wire or fibre, add a net of two members.
    pub fn add_net(&mut self, id: NetId, bearer: BearerSpec, members: impl IntoIterator<Item = NodeId>, f: &Figures) -> Result<(), CommsError> {
        let members: BTreeSet<NodeId> = members.into_iter().collect();
        for m in &members {
            if !self.nodes.contains_key(m) {
                return Err(CommsError::UnknownNode(*m));
            }
        }
        // Nominal SINR from positions, for links no physics has set.
        for a in &members {
            for b in &members {
                if a == b {
                    continue;
                }
                if let (Some(pa), Some(pb)) = (self.nodes[a].position, self.nodes[b].position) {
                    let d = pa.sub(pb).norm();
                    self.nominal_sinr.insert((id, *a, *b), nominal_sinr_db(&bearer, d, f));
                } else if !bearer.nominal_range_m.is_finite() {
                    self.nominal_sinr.insert((id, *a, *b), 60.0);
                }
            }
        }
        self.nets.insert(id, Net { id, bearer, members, up: true });
        Ok(())
    }

    pub fn net(&self, id: NetId) -> Option<&Net> {
        self.nets.get(&id)
    }

    pub fn set_pace(&mut self, kind: MessageKind, plan: PacePlan) {
        self.pace.insert(kind, plan);
    }

    /// Set the SINR of a link in both directions.
    pub fn set_link_sinr(&mut self, net: NetId, a: NodeId, b: NodeId, sinr_db: f64) {
        self.links.insert((net, a, b), sinr_db);
        self.links.insert((net, b, a), sinr_db);
    }

    /// Set the SINR of one direction (jamming is at the receiver).
    pub fn set_link_sinr_directed(&mut self, net: NetId, tx: NodeId, rx: NodeId, sinr_db: f64) {
        self.links.insert((net, tx, rx), sinr_db);
    }

    /// Recompute every radio link's SINR from a propagation model:
    /// P_tx + G_tx + G_rx − L − 10·log₁₀(kTBF + I).
    pub fn refresh_links(&mut self, phys: &impl LinkPhysics) {
        let mut updates = Vec::new();
        for net in self.nets.values() {
            if !net.bearer.kind.radiates() {
                continue;
            }
            let b = &net.bearer;
            let noise_w = BOLTZMANN * STANDARD_TEMPERATURE_K * b.waveform.bandwidth_hz * db_to_ratio(self.params.noise_figure_db);
            for &tx in &net.members {
                for &rx in &net.members {
                    if tx == rx {
                        continue;
                    }
                    let pl = phys.path_loss_db(tx, rx, b.freq_hz);
                    let s_dbw = ratio_to_db(b.power_w) + 2.0 * b.antenna_gain_dbi - pl;
                    let i_w = phys.interference_dbw(rx, b.freq_hz, b.waveform.bandwidth_hz).map(db_to_ratio).unwrap_or(0.0);
                    updates.push(((net.id, tx, rx), s_dbw - ratio_to_db(noise_w + i_w)));
                }
            }
        }
        for (k, v) in updates {
            self.links.insert(k, v);
        }
    }

    /// Take a net down (cut wire, jammed-out satcom, switched off) or up.
    pub fn set_net_up(&mut self, net: NetId, up: bool) -> Result<(), CommsError> {
        self.nets.get_mut(&net).ok_or(CommsError::UnknownNet(net))?.up = up;
        Ok(())
    }

    /// A node is destroyed: it stops sending, receiving and relaying.
    pub fn destroy_node(&mut self, node: NodeId) -> Result<(), CommsError> {
        self.nodes.get_mut(&node).ok_or(CommsError::UnknownNode(node))?.alive = false;
        Ok(())
    }

    /// Mark a node's keys compromised (a cyber effect, §8.12).
    pub fn compromise_key(&mut self, node: NodeId, compromised: bool) -> Result<(), CommsError> {
        self.nodes.get_mut(&node).ok_or(CommsError::UnknownNode(node))?.key_compromised = compromised;
        Ok(())
    }

    /// The SINR on a directed link, if the link exists.
    pub fn link_sinr(&self, net: NetId, tx: NodeId, rx: NodeId) -> Option<f64> {
        self.links.get(&(net, tx, rx)).or_else(|| self.nominal_sinr.get(&(net, tx, rx))).copied()
    }

    /// RF transmissions since the last drain.
    pub fn drain_emissions(&mut self) -> Vec<Emission> {
        std::mem::take(&mut self.emissions)
    }

    /// Messages in flight.
    pub fn in_flight(&self) -> usize {
        self.flights.values().filter(|f| !f.done).count()
    }

    fn size_of(&self, m: &Message) -> f64 {
        m.size_bits.unwrap_or_else(|| self.params.sizes[&m.kind])
    }

    fn is_voice(&self, m: &Message, b: &BearerSpec) -> bool {
        m.kind == MessageKind::VoiceContactReport && b.kind.carries_voice() && b.voice.is_some()
    }

    fn crypto_delay(&self, c: CryptoClass) -> f64 {
        match c {
            CryptoClass::Clear => 0.0,
            CryptoClass::ComsecVoice => self.params.crypto_voice_s,
            CryptoClass::Type1Data => self.params.crypto_data_s,
            CryptoClass::Commercial => self.params.crypto_commercial_s,
        }
    }

    fn schedule(&mut self, t: SimTime, ev: Ev) {
        let s = self.seq;
        self.seq += 1;
        self.events.insert((t, s), ev);
        self.heap.push(Reverse((t, s)));
    }

    /// Hand a message to the network at `t` (≥ the network's clock).
    pub fn send(&mut self, msg: Message, t: SimTime) -> Result<(), CommsError> {
        if t < self.now {
            return Err(CommsError::InThePast(msg.id, t, self.now));
        }
        for n in [msg.from, msg.to] {
            if !self.nodes.contains_key(&n) {
                return Err(CommsError::UnknownNode(n));
            }
        }
        let key = msg.id.0 as u64;
        if self.flights.contains_key(&key) {
            return Err(CommsError::Duplicate(msg.id));
        }
        let crypto = self.crypto_delay(msg.crypto);
        let flight = Flight {
            msg,
            sent_at: t,
            pace_level: 0,
            bearer: None,
            route: Vec::new(),
            hop: 0,
            hops: Vec::new(),
            fallbacks: Vec::new(),
            crypto_delay_s: crypto,
            enqueued_at: t,
            done: false,
        };
        self.flights.insert(key, flight);
        self.schedule(t + SimDuration::from_secs(crypto), Ev::Attempt(key));
        Ok(())
    }

    /// Run the network to `until`; return what was delivered or lost.
    pub fn step(&mut self, until: SimTime) -> Vec<CommsOutcome> {
        let mut out = Vec::new();
        while let Some(Reverse((t, s))) = self.heap.peek().copied() {
            if t > until {
                break;
            }
            self.heap.pop();
            let Some(ev) = self.events.remove(&(t, s)) else { continue };
            self.now = self.now.max(t);
            self.handle(t, ev, &mut out);
        }
        self.now = self.now.max(until);
        out
    }

    fn handle(&mut self, t: SimTime, ev: Ev, out: &mut Vec<CommsOutcome>) {
        match ev {
            Ev::Attempt(fid) => self.on_attempt(t, fid, out),
            Ev::Ready(fid) => self.on_ready(t, fid),
            Ev::Service(key) => self.on_service(t, key),
            Ev::Arrive(fid) => self.on_arrive(t, fid, out),
            Ev::HopFailed(fid, reason) => self.on_hop_failed(t, fid, reason),
        }
    }

    fn lose(&mut self, t: SimTime, fid: u64, reason: FailReason, out: &mut Vec<CommsOutcome>) {
        let Some(f) = self.flights.get_mut(&fid) else { return };
        if f.done {
            return;
        }
        f.done = true;
        out.push(CommsOutcome::Lost(Loss {
            msg: f.msg.id,
            kind: f.msg.kind,
            from: f.msg.from,
            to: f.msg.to,
            sent_at: f.sent_at,
            lost_at: t,
            reason,
            hops: f.hops.clone(),
            fallbacks: f.fallbacks.clone(),
        }));
    }

    fn on_attempt(&mut self, t: SimTime, fid: u64, out: &mut Vec<CommsOutcome>) {
        let Some(f) = self.flights.get(&fid) else { return };
        if f.done {
            return;
        }
        let msg = f.msg.clone();
        let level = f.pace_level;
        if !self.nodes[&msg.from].alive {
            return self.lose(t, fid, FailReason::NodeDestroyed { node: msg.from }, out);
        }
        let plan = self.pace.get(&msg.kind).cloned().unwrap_or(PacePlan { levels: vec![] });
        let Some(&bearer) = plan.levels.get(level) else {
            let last = f.fallbacks.last().map(|b| b.failure.clone()).unwrap_or(FailReason::NoRoute);
            return self.lose(t, fid, last, out);
        };
        let route = if bearer == BearerKind::Courier { self.courier_route(&msg) } else { self.route(&msg, bearer) };
        match route {
            Some(route) => {
                let f = self.flights.get_mut(&fid).expect("flight");
                f.bearer = Some(bearer);
                f.route = route;
                f.hop = 0;
                self.schedule(t, Ev::Ready(fid));
            }
            None => {
                let f = self.flights.get_mut(&fid).expect("flight");
                f.fallbacks.push(BearerFailure { bearer, pace_level: level, at: t, failure: FailReason::NoRoute });
                f.pace_level += 1;
                let next = if plan.levels.get(level + 1).is_some() { SimDuration::from_secs(self.params.pace_switch_s) } else { SimDuration::ZERO };
                self.schedule(t + next, Ev::Attempt(fid));
            }
        }
    }

    fn courier_route(&self, m: &Message) -> Option<Vec<RouteHop>> {
        let (a, b) = (&self.nodes[&m.from], &self.nodes[&m.to]);
        (a.position.is_some() && b.position.is_some() && b.alive).then(|| vec![RouteHop { from: m.from, to: m.to, net: None }])
    }

    /// The best route for a message on one bearer kind: Dijkstra over a
    /// petgraph graph whose edge weights are the expected hop time
    /// (airtime × expected transmissions ÷ success + latency + relay delay).
    /// Ties break on node id, so routes never depend on insertion order.
    fn route(&self, m: &Message, kind: BearerKind) -> Option<Vec<RouteHop>> {
        let size = self.size_of(m);
        let mut g: UnGraph<NodeId, (NetId, f64)> = UnGraph::default();
        let mut idx: BTreeMap<NodeId, NodeIndex> = BTreeMap::new();
        for (id, n) in &self.nodes {
            if n.alive {
                idx.insert(*id, g.add_node(*id));
            }
        }
        let (Some(&src), Some(&dst)) = (idx.get(&m.from), idx.get(&m.to)) else { return None };
        for net in self.nets.values().filter(|n| n.up && n.bearer.kind == kind) {
            let b = &net.bearer;
            let voice = self.is_voice(m, b);
            let bits = if voice { 0.0 } else { size };
            let airtime = if voice { b.voice.map(|v| v.report_s).unwrap_or(0.0) } else { bits / b.rate_bps };
            let members: Vec<NodeId> = net.members.iter().copied().filter(|n| idx.contains_key(n)).collect();
            for (i, &a) in members.iter().enumerate() {
                for &c in &members[i + 1..] {
                    // Use the worse direction for an undirected routing edge.
                    let s = match (self.link_sinr(net.id, a, c), self.link_sinr(net.id, c, a)) {
                        (Some(x), Some(y)) => x.min(y),
                        _ => continue,
                    };
                    let q = link_quality(b, s, bits.max(1.0), voice, self.params.packet_bits);
                    if q.p_success < self.params.route_min_p {
                        continue;
                    }
                    let w = airtime * q.expected_tx / q.p_success + b.latency_s;
                    g.add_edge(idx[&a], idx[&c], (net.id, w));
                }
            }
        }
        // Deterministic Dijkstra with relay rules.
        let n = g.node_count();
        let mut dist = vec![f64::INFINITY; n];
        let mut prev: Vec<Option<(NodeIndex, NetId)>> = vec![None; n];
        let mut done = vec![false; n];
        dist[src.index()] = 0.0;
        loop {
            let mut best: Option<NodeIndex> = None;
            for i in 0..n {
                if done[i] || !dist[i].is_finite() {
                    continue;
                }
                let better = match best {
                    None => true,
                    Some(b) => dist[i].total_cmp(&dist[b.index()]).then(g[NodeIndex::new(i)].cmp(&g[b])).is_lt(),
                };
                if better {
                    best = Some(NodeIndex::new(i));
                }
            }
            let Some(u) = best else { break };
            done[u.index()] = true;
            if u == dst {
                break;
            }
            // Only the source, relays, and mesh nodes forward.
            let node = &self.nodes[&g[u]];
            let forwards = u == src || node.relay || kind == BearerKind::Mesh;
            if !forwards {
                continue;
            }
            let mut edges: Vec<(NodeIndex, NetId, f64)> = g
                .edges(u)
                .map(|e| {
                    use petgraph::visit::EdgeRef;
                    let other = if e.source() == u { e.target() } else { e.source() };
                    (other, e.weight().0, e.weight().1)
                })
                .collect();
            edges.sort_by(|a, b| g[a.0].cmp(&g[b.0]).then(a.1.cmp(&b.1)));
            let hop_cost = if u == src { 0.0 } else { self.params.relay_s };
            for (v, net, w) in edges {
                let nd = dist[u.index()] + hop_cost + w;
                if nd < dist[v.index()] {
                    dist[v.index()] = nd;
                    prev[v.index()] = Some((u, net));
                }
            }
        }
        if !dist[dst.index()].is_finite() || src == dst {
            return None;
        }
        let mut hops = Vec::new();
        let mut cur = dst;
        while let Some((p, net)) = prev[cur.index()] {
            hops.push(RouteHop { from: g[p], to: g[cur], net: Some(net) });
            cur = p;
        }
        hops.reverse();
        Some(hops)
    }

    fn on_ready(&mut self, t: SimTime, fid: u64) {
        let Some(f) = self.flights.get_mut(&fid) else { return };
        if f.done {
            return;
        }
        let hop = f.route[f.hop].clone();
        f.enqueued_at = t;
        let prio = f.msg.priority;
        let Some(net_id) = hop.net else {
            // Courier: travel now.
            self.courier_hop(t, fid);
            return;
        };
        let access = self.nets[&net_id].bearer.access;
        let key = match access {
            ChannelAccess::Tdma | ChannelAccess::Dedicated => ChannelKey { net: net_id, node: Some(hop.from) },
            _ => ChannelKey { net: net_id, node: None },
        };
        let cap = self.params.queue_capacity;
        let ch = self.channels.entry(key).or_default();
        if ch.queue.len() >= cap {
            self.schedule(t, Ev::HopFailed(fid, FailReason::QueueFull { net: net_id }));
            return;
        }
        ch.queue.push((prio, t, fid));
        self.schedule(t, Ev::Service(key));
    }

    fn courier_hop(&mut self, t: SimTime, fid: u64) {
        let f = &self.flights[&fid];
        let hop = f.route[f.hop].clone();
        let (pa, pb) = (self.nodes[&hop.from].position.unwrap_or_default(), self.nodes[&hop.to].position.unwrap_or_default());
        let spec = self.courier.courier.expect("courier spec");
        let travel = pa.sub(pb).horizontal() / spec.speed_mps;
        let p_loss = hazard_probability(spec.loss_rate_per_h / 3600.0, travel);
        let u: f64 = self.seed.stream(StreamKey::new("comms.courier", fid, f.hop as u64, f.pace_level as i64)).gen();
        let ok = u >= p_loss;
        let end = t + SimDuration::from_secs(travel);
        let rec = HopRecord {
            from: hop.from,
            to: hop.to,
            net: None,
            bearer: BearerKind::Courier,
            enqueued_at: t,
            start: t,
            end,
            queue_wait_s: 0.0,
            airtime_s: travel,
            attempts: 1,
            sinr_db: f64::NAN,
            p_success: 1.0 - p_loss,
            success: ok,
        };
        self.flights.get_mut(&fid).expect("flight").hops.push(rec);
        if ok {
            self.schedule(end, Ev::Arrive(fid));
        } else {
            self.schedule(end, Ev::HopFailed(fid, FailReason::CourierLost { travel_s: travel, p_loss }));
        }
    }

    fn on_service(&mut self, t: SimTime, key: ChannelKey) {
        let Some(ch) = self.channels.get(&key) else { return };
        if ch.queue.is_empty() {
            return;
        }
        if ch.busy_until > t {
            let at = ch.busy_until;
            self.schedule(at, Ev::Service(key));
            return;
        }
        // Most urgent first, then oldest, then lowest id.
        let ch = self.channels.get_mut(&key).expect("channel");
        let (i, _) = ch.queue.iter().enumerate().min_by(|a, b| a.1.cmp(b.1)).expect("non-empty");
        let (_, _, fid) = ch.queue.remove(i);
        let others_waiting = ch.queue.len();
        let net = self.nets[&key.net].clone();
        let b = &net.bearer;
        let f = self.flights[&fid].clone();
        if f.done {
            self.schedule(t, Ev::Service(key));
            return;
        }
        let hop = f.route[f.hop].clone();
        if !self.nodes[&hop.from].alive {
            self.schedule(t, Ev::HopFailed(fid, FailReason::NodeDestroyed { node: hop.from }));
            self.schedule(t, Ev::Service(key));
            return;
        }
        let voice = self.is_voice(&f.msg, b);
        let size = if voice { 0.0 } else if f.msg.kind == MessageKind::VoiceContactReport { self.params.voice_as_data_bits } else { self.size_of(&f.msg) };
        let sinr = if net.up { self.link_sinr(net.id, hop.from, hop.to).unwrap_or(f64::NEG_INFINITY) } else { f64::NEG_INFINITY };
        let q = link_quality(b, sinr, size.max(1.0), voice, self.params.packet_bits);
        let mut rng = self.seed.stream(StreamKey::new("comms.tx", fid, f.hop as u64, ((f.pace_level as i64) << 8) | f.hops.len() as i64));
        // Channel access: when can it start, and how long is one attempt?
        let one = if voice { b.voice.map(|v| v.report_s).unwrap_or(0.0) } else { size / b.rate_bps };
        let (start, per_attempt_extra) = match b.access {
            ChannelAccess::PushToTalk => {
                let wait = if others_waiting > 0 { rng.gen::<f64>() * self.params.ptt_contention_s } else { 0.0 };
                (t + SimDuration::from_secs(wait + self.params.ptt_turnaround_s), self.params.ptt_turnaround_s)
            }
            ChannelAccess::Tdma => {
                let spec = b.tdma.expect("TDMA spec");
                let interval = spec.frame_s / spec.slots_per_frame as f64;
                let rank = net.members.iter().position(|m| *m == hop.from).unwrap_or(0) as f64;
                let phase = (rank * spec.slot_s) % interval;
                let ts = t.secs();
                let k = ((ts - phase) / interval).ceil().max(0.0);
                (SimTime::from_secs(phase + k * interval).max(t), 0.0)
            }
            ChannelAccess::RandomAccess => {
                // ALOHA-like collisions from the offered load on the net.
                let g = others_waiting as f64 * one / self.params.backoff_s.max(1e-6);
                let p_collide = 1.0 - (-2.0 * g).exp();
                let mut extra = 0.0;
                let mut n = 0;
                while n < 8 && rng.gen::<f64>() < p_collide {
                    extra += one + rng.gen::<f64>() * self.params.backoff_s;
                    n += 1;
                }
                (t + SimDuration::from_secs(extra), 0.0)
            }
            ChannelAccess::Dedicated | ChannelAccess::Courier => (t, 0.0),
        };
        // Airtime and success.
        let (success, attempts, airtime) = if voice {
            let tries = b.max_retries + 1;
            let mut n = 0;
            let mut ok = false;
            while n < tries {
                n += 1;
                if rng.gen::<f64>() < q.p_attempt {
                    ok = true;
                    break;
                }
            }
            (ok, n, n as f64 * (one + per_attempt_extra) - per_attempt_extra)
        } else {
            let ok = rng.gen::<f64>() < q.p_success;
            let tx = if ok { q.expected_tx } else { b.max_retries as f64 + 1.0 };
            let air = match b.tdma {
                Some(spec) => {
                    let slots = (size * tx / b.bits_per_slot()).ceil().max(1.0);
                    let interval = spec.frame_s / spec.slots_per_frame as f64;
                    (slots - 1.0) * interval + spec.slot_s
                }
                None => one * tx,
            };
            (ok, tx.ceil() as u32, air)
        };
        let end = start + SimDuration::from_secs(airtime);
        self.channels.get_mut(&key).expect("channel").busy_until = end;
        if b.kind.radiates() {
            self.emissions.push(Emission {
                node: hop.from,
                net: Some(net.id),
                bearer: b.kind,
                freq_hz: b.freq_hz,
                bandwidth_hz: b.waveform.bandwidth_hz,
                power_w: b.power_w,
                start,
                duration_s: airtime,
                msg: f.msg.id,
            });
        }
        let rec = HopRecord {
            from: hop.from,
            to: hop.to,
            net: Some(net.id),
            bearer: b.kind,
            enqueued_at: f.enqueued_at,
            start,
            end,
            queue_wait_s: (start - f.enqueued_at).secs(),
            airtime_s: airtime,
            attempts,
            sinr_db: sinr,
            p_success: q.p_success,
            success,
        };
        self.flights.get_mut(&fid).expect("flight").hops.push(rec);
        if success {
            self.schedule(end + SimDuration::from_secs(b.latency_s), Ev::Arrive(fid));
        } else {
            let reason = FailReason::LinkFailed { net: Some(net.id), from: hop.from, to: hop.to, sinr_db: sinr, p_success: q.p_success, attempts };
            self.schedule(end, Ev::HopFailed(fid, reason));
        }
        self.schedule(end, Ev::Service(key));
    }

    fn on_arrive(&mut self, t: SimTime, fid: u64, out: &mut Vec<CommsOutcome>) {
        let Some(f) = self.flights.get(&fid) else { return };
        if f.done {
            return;
        }
        let at = f.route[f.hop].to;
        if !self.nodes[&at].alive {
            return self.lose(t, fid, FailReason::NodeDestroyed { node: at }, out);
        }
        let last = f.hop + 1 == f.route.len();
        if last {
            let compromised = f.msg.crypto == CryptoClass::Clear || self.nodes[&f.msg.from].key_compromised;
            let f = self.flights.get_mut(&fid).expect("flight");
            f.done = true;
            out.push(CommsOutcome::Delivered(Delivery {
                msg: f.msg.id,
                kind: f.msg.kind,
                from: f.msg.from,
                to: f.msg.to,
                sent_at: f.sent_at,
                delivered_at: t,
                bearer: f.bearer.expect("bearer chosen"),
                pace_level: f.pace_level,
                crypto_delay_s: f.crypto_delay_s,
                hops: f.hops.clone(),
                fallbacks: f.fallbacks.clone(),
                compromised,
            }));
        } else {
            let relay = if f.bearer == Some(BearerKind::Mesh) { 0.0 } else { self.params.relay_s };
            self.flights.get_mut(&fid).expect("flight").hop += 1;
            self.schedule(t + SimDuration::from_secs(relay), Ev::Ready(fid));
        }
    }

    fn on_hop_failed(&mut self, t: SimTime, fid: u64, reason: FailReason) {
        let Some(f) = self.flights.get_mut(&fid) else { return };
        if f.done {
            return;
        }
        let bearer = f.bearer.unwrap_or(BearerKind::Courier);
        f.fallbacks.push(BearerFailure { bearer, pace_level: f.pace_level, at: t, failure: reason });
        f.pace_level += 1;
        let switch = SimDuration::from_secs(self.params.pace_switch_s);
        self.schedule(t + switch, Ev::Attempt(fid));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;
    use proptest::prelude::*;

    fn figs() -> Figures {
        crate::default_figures()
    }

    fn msg(id: u32, kind: MessageKind, from: u32, to: u32) -> Message {
        Message { id: MessageId(id), kind, from: NodeId(from), to: NodeId(to), priority: 2, crypto: CryptoClass::ComsecVoice, size_bits: None }
    }

    #[test]
    fn ber_matches_closed_forms() {
        // BPSK at Eb/N0 = 9.6 dB → BER ≈ 1e-5.
        let ber = bit_error_rate(Modulation::Bpsk, db_to_ratio(9.6));
        assert!((ber / 1e-5 - 1.0).abs() < 0.1, "{ber}");
        assert_relative_eq!(bit_error_rate(Modulation::NcFsk, 0.0), 0.5);
        assert!(bit_error_rate(Modulation::Dpsk, 10.0) < bit_error_rate(Modulation::NcFsk, 10.0));
    }

    #[test]
    fn link_quality_rises_with_sinr_and_falls_with_size() {
        let f = figs();
        let b = BearerSpec::from_figures(BearerKind::Mesh, &f);
        let small = link_quality(&b, 0.0, 1_000.0, false, 1024.0);
        let large = link_quality(&b, 0.0, 1e6, false, 1024.0);
        let better = link_quality(&b, 6.0, 1e6, false, 1024.0);
        assert!(small.p_success >= large.p_success);
        assert!(better.p_success >= large.p_success);
        assert!(large.expected_tx >= 1.0);
    }

    #[test]
    fn vhf_nominal_range_follows_power_setting() {
        // At each setting's nominal range, a voice report gets through about half the time [P4].
        let f = figs();
        for (p, r) in [(VhfPower::Low, 400.0), (VhfPower::Medium, 5_000.0), (VhfPower::High, 10_000.0), (VhfPower::VehicleAmplifier, 40_000.0)] {
            let b = BearerSpec::vhf(p, &f);
            let s = nominal_sinr_db(&b, r, &f);
            let q = link_quality(&b, s, 1.0, true, 1024.0);
            assert_relative_eq!(q.p_success, 0.5, epsilon = 1e-3);
            let near = link_quality(&b, nominal_sinr_db(&b, r / 3.0, &f), 1.0, true, 1024.0);
            assert!(near.p_success > 0.99);
        }
    }

    #[test]
    fn link16_figures_are_consistent() {
        let f = figs();
        let b = BearerSpec::from_figures(BearerKind::Tdl, &f);
        let t = b.tdma.unwrap();
        // 12.8 min epoch of 98,304 slots [P6].
        assert_relative_eq!(t.epoch_s / t.slot_s, 98_304.0, epsilon = 1e-6);
        assert_relative_eq!(t.frame_s / t.slot_s, 1_536.0, epsilon = 1e-6);
        assert!(b.bits_per_slot() > 200.0);
    }

    fn two_node_vhf(f: &Figures, d: f64) -> CommsNetwork {
        let mut n = CommsNetwork::new(f, Seed(7));
        n.add_node(NodeId(1), Some(Enu::new(0.0, 0.0, 0.0)), false);
        n.add_node(NodeId(2), Some(Enu::new(d, 0.0, 0.0)), false);
        n.add_net(NetId(1), BearerSpec::vhf(VhfPower::High, f), [NodeId(1), NodeId(2)], f).unwrap();
        n.set_pace(MessageKind::VoiceContactReport, PacePlan::new(BearerKind::Vhf, None, None, None));
        n
    }

    #[test]
    fn voice_report_on_a_good_link_takes_its_airtime() {
        let f = figs();
        let mut n = two_node_vhf(&f, 1_000.0);
        n.send(msg(1, MessageKind::VoiceContactReport, 1, 2), SimTime::from_secs_i(10)).unwrap();
        let out = n.step(SimTime::from_secs_i(100));
        let CommsOutcome::Delivered(d) = &out[0] else { panic!("{out:?}") };
        // crypto 0.5 + turnaround 0.5 + 20 s airtime.
        assert_relative_eq!(d.latency_s(), 21.0, epsilon = 1e-3);
        assert_eq!(d.hops.len(), 1);
        assert_eq!(d.hops[0].attempts, 1);
        assert_eq!(n.drain_emissions().len(), 1);
    }

    #[test]
    fn busy_voice_net_queues_by_priority() {
        let f = figs();
        let mut n = two_node_vhf(&f, 1_000.0);
        for i in 0..3 {
            let mut m = msg(i, MessageKind::VoiceContactReport, 1, 2);
            m.priority = if i == 2 { 0 } else { 3 };
            n.send(m, SimTime::from_secs_i(i64::from(i / 2))).unwrap();
        }
        let out = n.step(SimTime::from_secs_i(600));
        let order: Vec<u32> = out.iter().map(|o| o.msg().0).collect();
        // The first goes at once; the urgent one jumps the queue.
        assert_eq!(order, vec![0, 2, 1]);
        let CommsOutcome::Delivered(last) = &out[2] else { panic!() };
        assert!(last.hops[0].queue_wait_s > 35.0, "{:?}", last.hops[0]);
    }

    #[test]
    fn jammed_link_falls_back_through_pace() {
        let f = figs();
        let mut n = two_node_vhf(&f, 1_000.0);
        n.add_net(NetId(2), BearerSpec::from_figures(BearerKind::SatcomGeo, &f), [NodeId(1), NodeId(2)], &f).unwrap();
        n.set_pace(MessageKind::VoiceContactReport, PacePlan::new(BearerKind::Vhf, Some(BearerKind::SatcomGeo), None, None));
        // Jam VHF at the receiver: SINR far below threshold, but still routable.
        n.set_link_sinr(NetId(1), NodeId(1), NodeId(2), 6.5);
        let mut delivered_on = BTreeMap::new();
        for i in 0..40 {
            n.send(msg(i, MessageKind::VoiceContactReport, 1, 2), SimTime::from_secs_i(i as i64 * 300)).unwrap();
        }
        for o in n.step(SimTime::from_secs_i(40 * 300 + 3600)) {
            if let CommsOutcome::Delivered(d) = o {
                *delivered_on.entry(d.bearer).or_insert(0) += 1;
                if d.bearer == BearerKind::SatcomGeo {
                    assert_eq!(d.pace_level, 1);
                    assert!(matches!(d.fallbacks[0].failure, FailReason::LinkFailed { .. }));
                }
            }
        }
        assert!(delivered_on.get(&BearerKind::SatcomGeo).copied().unwrap_or(0) > 0, "{delivered_on:?}");
        assert!(delivered_on.get(&BearerKind::Vhf).copied().unwrap_or(0) > 0, "{delivered_on:?}");
    }

    #[test]
    fn hq_relays_between_nets_and_mesh_routes_around_failures() {
        let f = figs();
        let mut n = CommsNetwork::new(&f, Seed(1));
        for i in 1..=3 {
            n.add_node(NodeId(i), None, i == 2);
        }
        let vhf = BearerSpec::vhf(VhfPower::High, &f);
        n.add_net(NetId(1), vhf.clone(), [NodeId(1), NodeId(2)], &f).unwrap();
        n.add_net(NetId(2), vhf, [NodeId(2), NodeId(3)], &f).unwrap();
        n.set_link_sinr(NetId(1), NodeId(1), NodeId(2), 30.0);
        n.set_link_sinr(NetId(2), NodeId(2), NodeId(3), 30.0);
        n.set_pace(MessageKind::PlanOrder, PacePlan::new(BearerKind::Vhf, None, None, None));
        let mut m = msg(1, MessageKind::PlanOrder, 1, 3);
        m.crypto = CryptoClass::Type1Data;
        n.send(m, SimTime::ZERO).unwrap();
        let out = n.step(SimTime::from_secs_i(600));
        let CommsOutcome::Delivered(d) = &out[0] else { panic!("{out:?}") };
        assert_eq!(d.hops.len(), 2);
        assert!(d.hops[1].enqueued_at >= d.hops[0].end + SimDuration::from_secs(5.0));

        // A mesh square: 1-2-4 and 1-3-4; cut 2 and it goes via 3.
        let mut n = CommsNetwork::new(&f, Seed(2));
        for i in 1..=4 {
            n.add_node(NodeId(i), None, false);
        }
        let mesh = BearerSpec::from_figures(BearerKind::Mesh, &f);
        n.add_net(NetId(1), mesh, (1..=4).map(NodeId), &f).unwrap();
        for (a, b) in [(1, 2), (2, 4), (1, 3), (3, 4)] {
            n.set_link_sinr(NetId(1), NodeId(a), NodeId(b), 25.0);
        }
        n.set_pace(MessageKind::DataLinkTrack, PacePlan::new(BearerKind::Mesh, None, None, None));
        n.destroy_node(NodeId(2)).unwrap();
        n.send(msg(5, MessageKind::DataLinkTrack, 1, 4), SimTime::ZERO).unwrap();
        let out = n.step(SimTime::from_secs_i(60));
        let CommsOutcome::Delivered(d) = &out[0] else { panic!("{out:?}") };
        assert_eq!(d.hops[0].to, NodeId(3));
    }

    #[test]
    fn tdma_waits_for_its_slot_and_courier_walks() {
        let f = figs();
        let mut n = CommsNetwork::new(&f, Seed(3));
        n.add_node(NodeId(1), Some(Enu::new(0.0, 0.0, 0.0)), false);
        n.add_node(NodeId(2), Some(Enu::new(1_200.0, 0.0, 0.0)), false);
        n.add_net(NetId(1), BearerSpec::from_figures(BearerKind::Tdl, &f), [NodeId(1), NodeId(2)], &f).unwrap();
        n.set_pace(MessageKind::DataLinkTrack, PacePlan::new(BearerKind::Tdl, None, None, None));
        n.set_pace(MessageKind::PlanOrder, PacePlan::new(BearerKind::Courier, None, None, None));
        let mut m = msg(1, MessageKind::DataLinkTrack, 2, 1);
        m.crypto = CryptoClass::Clear;
        n.send(m, SimTime::from_secs(0.1)).unwrap();
        n.send(msg(2, MessageKind::PlanOrder, 1, 2), SimTime::ZERO).unwrap();
        let out = n.step(SimTime::from_secs_i(7200));
        for o in &out {
            match o {
                CommsOutcome::Delivered(d) if d.bearer == BearerKind::Tdl => {
                    // Node 2 owns slots at phase 1 slot, every 12/32 s.
                    let start = d.hops[0].start.secs();
                    let interval = 12.0 / 32.0;
                    let k = ((start - 0.007_812_5) / interval).round();
                    assert!((start - 0.007_812_5 - k * interval).abs() < 1e-5, "{start}");
                    assert!(d.compromised, "clear traffic is readable");
                }
                CommsOutcome::Delivered(d) => {
                    assert_eq!(d.bearer, BearerKind::Courier);
                    assert_relative_eq!(d.hops[0].airtime_s, 1000.0, epsilon = 1e-6);
                }
                CommsOutcome::Lost(l) => panic!("{l:?}"),
            }
        }
        assert_eq!(n.drain_emissions().len(), 1, "couriers don't radiate");
    }

    struct FreeSpace(BTreeMap<NodeId, Enu>);
    impl LinkPhysics for FreeSpace {
        fn path_loss_db(&self, a: NodeId, b: NodeId, f: f64) -> f64 {
            crate::math::free_space_loss_db(self.0[&a].sub(self.0[&b]).norm(), f)
        }
        fn interference_dbw(&self, rx: NodeId, _f: f64, _bw: f64) -> Option<f64> {
            (rx == NodeId(2)).then_some(-60.0)
        }
    }

    #[test]
    fn refresh_links_uses_the_supplied_physics() {
        let f = figs();
        let mut n = CommsNetwork::new(&f, Seed(1));
        let pos: BTreeMap<NodeId, Enu> = [(NodeId(1), Enu::new(0.0, 0.0, 0.0)), (NodeId(2), Enu::new(5_000.0, 0.0, 0.0))].into();
        for (id, p) in &pos {
            n.add_node(*id, Some(*p), false);
        }
        n.add_net(NetId(1), BearerSpec::vhf(VhfPower::High, &f), [NodeId(1), NodeId(2)], &f).unwrap();
        n.refresh_links(&FreeSpace(pos));
        let to2 = n.link_sinr(NetId(1), NodeId(1), NodeId(2)).unwrap();
        let to1 = n.link_sinr(NetId(1), NodeId(2), NodeId(1)).unwrap();
        assert!(to1 > to2 + 50.0, "jamming at node 2 only: {to1} {to2}");
    }

    #[test]
    fn destroyed_sender_loses_its_queue() {
        let f = figs();
        let mut n = two_node_vhf(&f, 1_000.0);
        for i in 0..3 {
            n.send(msg(i, MessageKind::VoiceContactReport, 1, 2), SimTime::ZERO).unwrap();
        }
        let first = n.step(SimTime::from_secs_i(5));
        assert!(first.is_empty());
        n.destroy_node(NodeId(1)).unwrap();
        let out = n.step(SimTime::from_secs_i(600));
        let lost = out.iter().filter(|o| matches!(o, CommsOutcome::Lost(l) if matches!(l.reason, FailReason::NodeDestroyed { .. }))).count();
        assert_eq!(lost, 2);
        assert!(n.send(msg(9, MessageKind::DataLinkTrack, 1, 2), SimTime::ZERO).is_err(), "past");
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(24))]
        #[test]
        fn messages_never_arrive_before_sent(
            seed in 0u64..1000,
            sinrs in proptest::collection::vec(-5.0f64..30.0, 6),
            sends in proptest::collection::vec((0u32..4, 0u32..4, 0u8..4, 0.0f64..600.0, 0usize..4), 1..25),
        ) {
            let f = figs();
            let mut n = CommsNetwork::new(&f, Seed(seed));
            for i in 0..4 {
                n.add_node(NodeId(i), Some(Enu::new(i as f64 * 800.0, 0.0, 0.0)), i == 1);
            }
            n.add_net(NetId(1), BearerSpec::vhf(VhfPower::Medium, &f), (0..4).map(NodeId), &f).unwrap();
            n.add_net(NetId(2), BearerSpec::from_figures(BearerKind::Mesh, &f), (0..4).map(NodeId), &f).unwrap();
            let pairs = [(0, 1), (1, 2), (2, 3), (0, 2), (1, 3), (0, 3)];
            for (k, (a, b)) in pairs.iter().enumerate() {
                n.set_link_sinr(NetId(1), NodeId(*a), NodeId(*b), sinrs[k]);
                n.set_link_sinr(NetId(2), NodeId(*a), NodeId(*b), sinrs[k] - 3.0);
            }
            for k in [MessageKind::VoiceContactReport, MessageKind::DataLinkTrack, MessageKind::Image, MessageKind::PlanOrder] {
                n.set_pace(k, PacePlan::new(BearerKind::Vhf, Some(BearerKind::Mesh), Some(BearerKind::Courier), None));
            }
            let kinds = [MessageKind::VoiceContactReport, MessageKind::DataLinkTrack, MessageKind::Image, MessageKind::PlanOrder];
            let mut sends = sends;
            sends.sort_by(|a, b| a.3.total_cmp(&b.3));
            let mut count = 0;
            let mut sent_at = BTreeMap::new();
            for (i, (a, b, p, t, k)) in sends.into_iter().enumerate() {
                if a == b { continue; }
                let m = Message { id: MessageId(i as u32), kind: kinds[k], from: NodeId(a), to: NodeId(b), priority: p, crypto: CryptoClass::Type1Data, size_bits: None };
                let t = SimTime::from_secs(t);
                n.send(m, t).unwrap();
                sent_at.insert(i as u32, t);
                count += 1;
            }
            let out = n.step(SimTime::from_secs_i(1_000_000));
            prop_assert_eq!(out.len(), count);
            for o in &out {
                match o {
                    CommsOutcome::Delivered(d) => {
                        prop_assert!(d.delivered_at >= d.sent_at);
                        prop_assert_eq!(d.sent_at, sent_at[&d.msg.0]);
                        for h in &d.hops { prop_assert!(h.start >= h.enqueued_at && h.end >= h.start && h.enqueued_at >= d.sent_at); }
                    }
                    CommsOutcome::Lost(l) => prop_assert!(l.lost_at >= l.sent_at),
                }
            }
            prop_assert_eq!(n.in_flight(), 0);
        }
    }
}
