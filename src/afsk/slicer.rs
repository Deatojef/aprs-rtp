use crate::afsk::dpll::{DemodBit, Dpll};

/// Precomputed space-gain factors for the multi-slicer array.
///
/// Mirrors direwolf's initialization:
///   space_gain[0] = MIN_G (0.5)
///   step = (MAX_G / MIN_G)^(1 / (n-1))   [geometric progression]
///   space_gain[i] = space_gain[i-1] * step
///
/// The gains span [0.5, 4.0] in `n` logarithmically-spaced steps.
/// Each slicer uses a different gain to compensate for varying degrees of
/// transmitter pre-emphasis / receiver de-emphasis imbalance.
pub fn space_gains(n: usize) -> Vec<f32> {
    assert!(n >= 1);
    const MIN_G: f32 = 0.5;
    const MAX_G: f32 = 4.0;
    if n == 1 {
        return vec![1.0]; // single slicer: unity gain
    }
    let step = (MAX_G / MIN_G).powf(1.0 / (n - 1) as f32);
    let mut gains = vec![MIN_G];
    for i in 1..n {
        gains.push(gains[i - 1] * step);
    }
    gains
}

/// Per-slicer state: one DPLL per slicer, driven by a unique space_gain.
///
/// In the multi-slicer path, all slicers receive the same mark/space amplitude
/// measurements each sample but apply a different `space_gain` before comparing:
///   demod_out = m_amp - s_amp * space_gain[i]
///   amplitude = 0.5 * (m_peak - m_valley + (s_peak - s_valley) * space_gain[i])
///
/// This makes the decoder robust to amplitude imbalance between the tones
/// without needing to know a priori which direction the imbalance goes.
pub struct SlicerBank {
    pub dplls: Vec<Dpll>,
    pub gains: Vec<f32>,
}

impl SlicerBank {
    pub fn new(num_slicers: usize, baud: u32, sample_rate: u32) -> Self {
        let gains = space_gains(num_slicers);
        let dplls = (0..num_slicers)
            .map(|i| Dpll::new(baud, sample_rate, i))
            .collect();
        Self { dplls, gains }
    }

    /// Drive all slicers for one audio sample.
    ///
    /// `m_amp`, `s_amp`: mark and space envelope magnitudes from IQ detection.
    /// `m_peak`, `m_valley`, `s_peak`, `s_valley`: running AGC bounds used for
    /// the per-slicer amplitude (quality) estimate.
    ///
    /// Returns bits from any slicers whose DPLLs overflowed this sample.
    pub fn process(
        &mut self,
        m_amp: f32,
        s_amp: f32,
        m_peak: f32,
        m_valley: f32,
        s_peak: f32,
        s_valley: f32,
    ) -> Vec<DemodBit> {
        let mut bits = Vec::new();
        for (dpll, &gain) in self.dplls.iter_mut().zip(&self.gains) {
            let demod_out = m_amp - s_amp * gain;
            let amp = 0.5 * (m_peak - m_valley + (s_peak - s_valley) * gain);
            let amp = if amp < 1e-7 { 1.0 } else { amp };
            if let Some(bit) = dpll.step(demod_out, amp) {
                bits.push(bit);
            }
        }
        bits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gains_span_min_to_max() {
        let g = space_gains(9);
        assert_eq!(g.len(), 9);
        assert!((g[0] - 0.5).abs() < 1e-6, "first gain = {}", g[0]);
        assert!((g[8] - 4.0).abs() < 1e-4, "last gain = {}", g[8]);
    }

    #[test]
    fn gains_are_geometric() {
        let g = space_gains(9);
        let ratios: Vec<f32> = g.windows(2).map(|w| w[1] / w[0]).collect();
        // All ratios should be equal (geometric progression).
        let r0 = ratios[0];
        for &r in &ratios {
            assert!((r - r0).abs() < 1e-5, "non-geometric: {r} vs {r0}");
        }
    }

    #[test]
    fn single_slicer_unity_gain() {
        let g = space_gains(1);
        assert_eq!(g.len(), 1);
        assert!((g[0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn slicer_bank_produces_bits() {
        // Drive the bank with a strong mark signal; after 20 samples (one baud
        // period at 24kHz/1200baud) each slicer should have produced one bit.
        let mut bank = SlicerBank::new(8, 1200, 24000);
        let mut total_bits = 0usize;
        for _ in 0..20 {
            // m_amp >> s_amp → strong mark
            let bits = bank.process(1.0, 0.0, 1.0, 0.0, 0.0, 0.0);
            total_bits += bits.len();
        }
        // Each of 8 slicers overflows once in 20 samples.
        assert_eq!(total_bits, 8, "expected 8 bits (one per slicer)");
    }
}
