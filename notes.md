## Audio Notes

**rec** — total baseband audio energy. Useful for:
- Spotting **over-deviated** transmitters (rec pushing toward the ceiling, audio likely clipped)
- Spotting **under-deviated** transmitters (rec well below typical, weak modulation)
- Detecting **DC offsets / hum / squelch tails** that inflate rec without contributing to mark/space

**mark** and **space** — energy at exactly 1200 Hz and 2200 Hz. Useful for:
- Confirming both tones are actually present (a stuck-tone transmitter would show one near zero)
- Checking the **mark/space ratio** — a well-adjusted system has them within ~2× of each other. A big imbalance usually means **pre/de-emphasis mismatch**: pre-emphasis on the TX side boosts high frequencies, de-emphasis on the RX side cuts them. If they don't match (e.g., flat audio out of ka9q-radio but TX uses pre-emphasis), the 2200 Hz space tone reads higher than the 1200 Hz mark tone, or vice versa.
- The combination `rec − (mark + space)` gives a rough sense of how much **non-tone energy** is in the audio: noise, hum, distortion, adjacent-channel bleed.

**What they're not good for:** RF signal strength, SNR, distance, or path quality. For those, ka9q-radio publishes per-channel SNR and power figures in its status stream (separate RTCP/status multicast); that's what to use if you want to correlate with RF conditions.

Think of rec/mark/space as a **transmitter audio fingerprint** — useful for spotting misadjusted TXs and pre/de-emphasis mismatches, but completely decoupled from how strong the signal got to your antenna.
