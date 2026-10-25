//! Time-continuous hazards (rule 2): P = 1 − e^(−λΔt), so results don't depend
//! on the step size.

/// Probability that an event with rate `lambda` (per second) happens within `dt` seconds.
pub fn hazard_probability(lambda: f64, dt: f64) -> f64 {
    if lambda <= 0.0 || dt <= 0.0 {
        return 0.0;
    }
    -(-lambda * dt).exp_m1()
}

/// The constant rate that gives probability `p` over `tau` seconds.
pub fn rate_from_probability(p: f64, tau: f64) -> f64 {
    if p <= 0.0 || tau <= 0.0 {
        return 0.0;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }
    -(-p).ln_1p() / tau
}

/// Combine independent per-step probabilities.
pub fn any_of(ps: impl IntoIterator<Item = f64>) -> f64 {
    1.0 - ps.into_iter().fold(1.0, |acc, p| acc * (1.0 - p.clamp(0.0, 1.0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn step_size_independent() {
        let l = 0.05;
        let one = hazard_probability(l, 10.0);
        let ten = any_of((0..10).map(|_| hazard_probability(l, 1.0)));
        assert!((one - ten).abs() < 1e-12);
        assert!((rate_from_probability(one, 10.0) - l).abs() < 1e-12);
    }
}
