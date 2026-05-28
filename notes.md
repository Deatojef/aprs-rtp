## Audio Notes

### How the values are calculated

Input audio is normalized to `[-1.0, +1.0]` (full-scale s16 → ±1.0). All three
levels are tracked with a slow-attack/slow-decay IIR (fast attack 0.14, slow
decay 0.000018 — 5× slower than the demodulation AGC) so they stay stable
across an entire packet and are comparable across SSRCs.

- **rec** = `(alevel_rec_peak − alevel_rec_valley) × 100`, clamped to u8 `[0, 255]`.
  - `alevel_rec_peak` and `alevel_rec_valley` are IIR trackers running directly
    on the raw normalized sample stream — one chases the positive extreme, one
    chases the negative.
  - By construction `peak ≥ valley`, so the value is always non-negative.

- **mark** = `alevel_mark_peak × 200`, clamped to u8 `[0, 255]`.
  - The audio is passed through a bandpass pre-filter, then IQ-mixed at
    1200 Hz and lowpassed. `m_amp = √(m_I² + m_Q²)` is the instantaneous
    envelope of the 1200 Hz component. `alevel_mark_peak` is the slow IIR
    tracker over `m_amp`.
  - IQ demodulation halves the envelope (energy split between I and Q legs),
    which is why the scaling constant is `200` vs rec's `100` — so a clean
    full-scale tone lands near 100 on both scales.

- **space** = `alevel_space_peak × 200`, clamped to u8 `[0, 255]`. Same as
  mark but with the IQ mixer at 2200 Hz.

### Ranges and reference points

| Quantity   | Min | Practical max | u8 ceiling |
|------------|-----|---------------|------------|
| **rec**    | 0   | ~200          | 255        |
| **mark**   | 0   | ~100          | 255        |
| **space**  | 0   | ~100          | 255        |

The u8 ceiling (255) is only reachable with pathological input (DC offset
pushing the signal beyond ±1.0). With valid s16 RTP audio you can't exceed
the practical max.

For an unbiased sine of amplitude A in normalized units:

- `rec ≈ 200·A`
- `mark ≈ 100·A` if the tone is at exactly 1200 Hz (0 if it isn't)
- `space ≈ 100·A` if the tone is at exactly 2200 Hz (0 if it isn't)

Examples:

| Input                                  | rec | mark | space |
|----------------------------------------|-----|------|-------|
| silence                                | 0   | 0    | 0     |
| amp 0.25 sine, off-tone                | ~50 | 0    | 0     |
| amp 0.5  sine at 1200 Hz               | ~100| ~50  | ~0    |
| amp 1.0  (full-scale) sine at 1200 Hz  | ~200| ~100 | ~0    |
| amp 1.0  (full-scale) sine at 2200 Hz  | ~200| ~0   | ~100  |

### Typical APRS values

For a well-adjusted real-world APRS signal (modulation deviation in spec,
no clipping, both tones present, no significant hum or DC bias):

- **rec**: 30–70
- **mark / space**: 10–40, with the two within ~2× of each other

Values well outside those ranges hint at the kinds of issues described below.

### What the values indicate


- Spotting **over-deviated** transmitters (rec pushing toward the ceiling, audio likely clipped)
- Spotting **under-deviated** transmitters (rec well below typical, weak modulation)
- Detecting **DC offsets / hum / squelch tails** that inflate rec without contributing to mark/space

**mark** and **space** — energy at exactly 1200 Hz and 2200 Hz. Useful for:
- Confirming both tones are actually present (a stuck-tone transmitter would show one near zero)
- Checking the **mark/space ratio** — a well-adjusted system has them within ~2× of each other. A big imbalance usually means **pre/de-emphasis mismatch**: pre-emphasis on the TX side boosts high frequencies, de-emphasis on the RX side cuts them. If they don't match (e.g., flat audio out of ka9q-radio but TX uses pre-emphasis), the 2200 Hz space tone reads higher than the 1200 Hz mark tone, or vice versa.
- The combination `rec − (mark + space)` gives a rough sense of how much **non-tone energy** is in the audio: noise, hum, distortion, adjacent-channel bleed.

**What they're not good for:** RF signal strength, SNR, distance, or path quality. For those, ka9q-radio publishes per-channel SNR and power figures in its status stream (separate RTCP/status multicast); that's what to use if you want to correlate with RF conditions.

Think of rec/mark/space as a **transmitter audio fingerprint** — useful for spotting misadjusted TXs and pre/de-emphasis mismatches, but completely decoupled from how strong the signal got to your antenna.
