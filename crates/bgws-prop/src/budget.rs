//! The receiver budget (§8.4, §7.5): every path loss feeds the same chain —
//! signal, thermal noise `kTBF`, interference and jamming → SNR and SINR →
//! Eb/N0 → bit error rate for the waveform → message success for N bits →
//! throughput. Comms, radar, GNSS and ESM all close their links here.
//!
//! BER formulas are the textbook AWGN ones (jamming treated as
//! noise-like in band): coherent BPSK/QPSK/MSK `½·erfc(√(Eb/N0))`, DBPSK
//! `½·e^(−Eb/N0)`, coherent BFSK `½·erfc(√(Eb/2N0))`, non-coherent BFSK
//! `½·e^(−Eb/2N0)`, Gray 16-QAM `⅜·erfc(√(0.4·Eb/N0))`.

use bgws_core::units::BOLTZMANN;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Waveform {
    Bpsk,
    Qpsk,
    Msk,
    Dbpsk,
    CoherentFsk,
    NoncoherentFsk,
    Qam16,
}

/// Bit error probability at a linear Eb/N0.
pub fn ber(w: Waveform, eb_n0: f64) -> f64 {
    let x = eb_n0.max(0.0);
    let p = match w {
        Waveform::Bpsk | Waveform::Qpsk | Waveform::Msk => 0.5 * libm::erfc(x.sqrt()),
        Waveform::Dbpsk => 0.5 * (-x).exp(),
        Waveform::CoherentFsk => 0.5 * libm::erfc((x / 2.0).sqrt()),
        Waveform::NoncoherentFsk => 0.5 * (-x / 2.0).exp(),
        Waveform::Qam16 => 0.375 * libm::erfc((0.4 * x).sqrt()),
    };
    p.clamp(0.0, 0.5)
}

/// Thermal noise power, dBW: 10·log10(k·T·B) + NF.
pub fn noise_dbw(bandwidth_hz: f64, noise_temp_k: f64, noise_figure_db: f64) -> f64 {
    10.0 * (BOLTZMANN * noise_temp_k * bandwidth_hz).log10() + noise_figure_db
}

/// A jammer as received: power at the receiver input before band overlap.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Jammer {
    pub power_dbw: f64,
    /// Fraction of the jammer's power that falls in the receiver's band.
    pub band_overlap: f64,
}

/// Everything the receiver chain needs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BudgetInput {
    pub tx_power_dbw: f64,
    pub tx_gain_dbi: f64,
    pub rx_gain_dbi: f64,
    pub path_loss_db: f64,
    pub other_losses_db: f64,
    pub bandwidth_hz: f64,
    pub noise_temp_k: f64,
    pub noise_figure_db: f64,
    /// Co-channel interferers already in band, dBW at the receiver.
    pub interference_dbw: Vec<f64>,
    pub jammers: Vec<Jammer>,
    pub waveform: Waveform,
    pub bit_rate_bps: f64,
    pub coding_gain_db: f64,
    pub message_bits: u32,
}

/// The chain's values, for the AAR.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BudgetResult {
    pub signal_dbw: f64,
    pub noise_dbw: f64,
    pub interference_dbw: f64,
    pub jamming_dbw: f64,
    pub snr_db: f64,
    pub sinr_db: f64,
    /// Jam-to-signal ratio, dB (−∞ without jammers).
    pub js_db: f64,
    pub eb_n0_db: f64,
    pub ber: f64,
    pub p_message: f64,
    /// Goodput with retransmission of failed messages, bit/s.
    pub throughput_bps: f64,
}

fn sum_dbw(vals: impl Iterator<Item = f64>) -> f64 {
    let s: f64 = vals.map(|v| 10f64.powf(v / 10.0)).sum();
    if s > 0.0 {
        10.0 * s.log10()
    } else {
        f64::NEG_INFINITY
    }
}

/// Close a link.
pub fn link_budget(i: &BudgetInput) -> BudgetResult {
    let signal = i.tx_power_dbw + i.tx_gain_dbi + i.rx_gain_dbi - i.path_loss_db - i.other_losses_db;
    let noise = noise_dbw(i.bandwidth_hz, i.noise_temp_k, i.noise_figure_db);
    let interference = sum_dbw(i.interference_dbw.iter().copied());
    let jamming = sum_dbw(i.jammers.iter().map(|j| j.power_dbw + 10.0 * j.band_overlap.clamp(1e-12, 1.0).log10()));
    let n_plus_i = sum_dbw([noise, interference, jamming].into_iter().filter(|v| v.is_finite()));
    let snr = signal - noise;
    let sinr = signal - n_plus_i;
    let eb_n0_db = sinr + 10.0 * (i.bandwidth_hz / i.bit_rate_bps.max(1e-9)).log10() + i.coding_gain_db;
    let b = ber(i.waveform, 10f64.powf(eb_n0_db / 10.0));
    let p_message = (1.0 - b).powi(i.message_bits as i32);
    BudgetResult {
        signal_dbw: signal,
        noise_dbw: noise,
        interference_dbw: interference,
        jamming_dbw: jamming,
        snr_db: snr,
        sinr_db: sinr,
        js_db: jamming - signal,
        eb_n0_db,
        ber: b,
        p_message,
        throughput_bps: i.bit_rate_bps * p_message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn db(x: f64) -> f64 {
        10f64.powf(x / 10.0)
    }

    #[test]
    fn ber_curves() {
        // Classic points: BPSK 1e-5 at 9.59 dB; NC-FSK 1e-5 at 13.35 dB; DBPSK 1e-5 at 10.34 dB.
        assert_relative_eq!(ber(Waveform::Bpsk, db(9.59)).log10(), -5.0, epsilon = 0.02);
        assert_relative_eq!(ber(Waveform::NoncoherentFsk, db(13.35)).log10(), -5.0, epsilon = 0.02);
        assert_relative_eq!(ber(Waveform::Dbpsk, db(10.34)).log10(), -5.0, epsilon = 0.02);
        // Coherent FSK needs 3 dB more than BPSK for the same BER.
        assert_relative_eq!(ber(Waveform::CoherentFsk, db(12.6)), ber(Waveform::Bpsk, db(9.59)), epsilon = 1e-7);
        assert_relative_eq!(ber(Waveform::Bpsk, 0.0), 0.5);
        // 16-QAM is worse than QPSK at the same Eb/N0.
        assert!(ber(Waveform::Qam16, db(10.0)) > ber(Waveform::Qpsk, db(10.0)));
    }

    #[test]
    fn budget_noise_jamming_and_messages() {
        // kTB at 290 K in 1 Hz = −204.0 dBW.
        assert_relative_eq!(noise_dbw(1.0, 290.0, 0.0), -203.98, epsilon = 0.01);
        let base = BudgetInput {
            tx_power_dbw: 10.0 * 5f64.log10(), // 5 W
            tx_gain_dbi: 2.0,
            rx_gain_dbi: 2.0,
            path_loss_db: 130.0,
            other_losses_db: 2.0,
            bandwidth_hz: 25e3,
            noise_temp_k: 290.0,
            noise_figure_db: 6.0,
            interference_dbw: vec![],
            jammers: vec![],
            waveform: Waveform::NoncoherentFsk,
            bit_rate_bps: 16e3,
            coding_gain_db: 0.0,
            message_bits: 1000,
        };
        let clear = link_budget(&base);
        // Signal −121 dBW; noise −204 + 44 + 6 = −154 dBW; SNR ≈ 33 dB.
        assert_relative_eq!(clear.snr_db, 33.0, epsilon = 0.1);
        assert!(clear.p_message > 0.999 && clear.js_db == f64::NEG_INFINITY);
        // A jammer 20 dB above the signal, half in band, kills the link.
        let jammed = link_budget(&BudgetInput { jammers: vec![Jammer { power_dbw: clear.signal_dbw + 20.0, band_overlap: 0.5 }], ..base.clone() });
        assert!(jammed.sinr_db < -15.0 && jammed.p_message < 1e-6 && jammed.throughput_bps < 1.0, "{jammed:?}");
        assert_relative_eq!(jammed.js_db, 20.0 - 3.01, epsilon = 0.01);
        // Longer messages fail more often at a marginal Eb/N0.
        let marginal = BudgetInput { path_loss_db: 154.0, ..base.clone() };
        let short = link_budget(&BudgetInput { message_bits: 100, ..marginal.clone() });
        let long = link_budget(&BudgetInput { message_bits: 5000, ..marginal });
        assert!(short.p_message > long.p_message && long.p_message < 0.9);
    }
}
