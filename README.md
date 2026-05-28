# aprs-rtp

A Rust library for decoding 1200-baud APRS packets from [ka9q-radio]'s RTP
multicast audio streams. Bring your own SDR and `radiod` channel; this crate
listens to the resulting audio multicast, demodulates AFSK with direwolf's
multi-slicer approach, and hands you decoded `AprsPacket`s through a tokio
channel.

[ka9q-radio]: https://github.com/ka9q/ka9q-radio

## Features

- **ka9q-radio native** — speaks the RTP multicast format `radiod` emits
  (S16BE PCM, 24 kHz mono on PT 116 by default; other PCM payload types
  supported automatically).
- **Multi-slicer AFSK demodulator** — direwolf's Profile A pipeline ported
  to Rust: bandpass pre-filter, IQ mixing at 1200/2200 Hz, RRC lowpass,
  per-tone AGC, and 1–16 parallel amplitude-imbalance slicers per stream.
- **HDLC framing + CRC FEC** — bit-stuffed HDLC decode with optional
  single- or double-bit error correction to recover marginal frames.
- **AX.25 UI-frame parser** — surfaces source, destination, full
  digipeater path, H-bit state, and APRS info field as typed fields.
- **APRS-aware "heard from"** — `heard_from` skips routing aliases
  (`WIDE`, `TRACE`, `TCPIP`, etc.) so you get the actual transmitting
  station, not the routing slot.
- **Per-SSRC isolation** — each ka9q-radio channel runs in its own
  blocking DSP thread; one slow SSRC can't stall another.
- **Multi-source** — listen to several multicast groups (e.g. 2 m + 70 cm)
  through one merged stream of packets.

## How it fits in a ka9q-radio setup

```
    SDR ──► radiod ──► RTP multicast (e.g. 239.x or hostname.local:5004)
                              │
                              ▼
                         aprs-rtp ──► mpsc::Receiver<AprsPacket>
                                              │
                                              ▼
                                    your application
                                    (iGate, logger, UI, …)
```

ka9q-radio's `radiod` already does the FM demodulation; this crate consumes
the baseband audio it publishes and turns it into APRS packets. Each
demodulator channel in `radiod` emits a distinct SSRC; by convention the
SSRC equals the tuned frequency in kHz (e.g. SSRC `144390` ⇒ 144.390 MHz),
which is how `AprsPacket.freq_mhz` is derived.

## Library usage

### Minimal example

```rust
use aprs_rtp::{
    config::{DecoderConfig, SourceConfig},
    AprsListener,
};

#[tokio::main]
async fn main() -> aprs_rtp::Result<()> {
    let source = SourceConfig {
        host: "packet.local".into(), // mDNS or 239.x multicast address
        port: 5004,
        interface: None,             // bind on OS default
        jitter_buffer: 2,
        ssrc: vec![],                // empty = accept all SSRCs on the group
    };

    let listener = AprsListener::new(source, DecoderConfig::default());
    let mut packets = listener.run().await?;

    while let Some(pkt) = packets.recv().await {
        println!(
            "{:>8.3} MHz  {}  {}",
            pkt.freq_mhz,
            if pkt.heard_direct { "D" } else { "*" },
            pkt.text,
        );
    }
    Ok(())
}
```

`AprsListener::run` spawns the RTP receive task and per-SSRC DSP threads,
then returns immediately with a `tokio::sync::mpsc::Receiver<AprsPacket>`.
The receiver stays open as long as the multicast socket is alive.

### Loading config from a TOML file

`SourceConfig` and `DecoderConfig` derive `serde::Deserialize`, so you can
deserialize them from any format you like. For TOML, define a top-level
wrapper in your own code and use the `toml` crate directly:

```rust
use aprs_rtp::{config::{DecoderConfig, SourceConfig}, AprsListener, AprsPacket};
use serde::Deserialize;
use tokio::sync::mpsc;

#[derive(Deserialize)]
struct AppConfig {
    #[serde(default)]
    decoder: DecoderConfig,
    #[serde(rename = "source")]
    sources: Vec<SourceConfig>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string("config.toml")?;
    let cfg: AppConfig = toml::from_str(&text)?;

    // Merge packets from all configured sources into one channel.
    let (tx, mut rx) = mpsc::channel::<AprsPacket>(512);
    for source in cfg.sources {
        let listener = AprsListener::new(source, cfg.decoder.clone());
        let mut pkt_rx = listener.run().await?;
        let fwd = tx.clone();
        tokio::spawn(async move {
            while let Some(pkt) = pkt_rx.recv().await {
                if fwd.send(pkt).await.is_err() { break; }
            }
        });
    }
    drop(tx); // close rx once all source tasks finish

    while let Some(pkt) = rx.recv().await {
        println!("{}", pkt.text);
    }
    Ok(())
}
```

### Filtering to specific channels

If a single ka9q-radio multicast group carries multiple demodulator
channels and you only care about one (or a few), set the `ssrc` allowlist
on the source:

```rust
SourceConfig {
    host: "packet.local".into(),
    port: 5004,
    interface: None,
    jitter_buffer: 2,
    ssrc: vec![144_390, 145_010], // only these two channels
}
```

Frames from other SSRCs on the same multicast group are dropped before
DSP, so this also reduces CPU.

### Working with the decoded packet

`AprsPacket` is a flat struct with structured access to every parsed
field — no need to re-parse `text` or `raw_ax25`:

| Field          | Type            | Notes                                              |
|----------------|-----------------|----------------------------------------------------|
| `ssrc`         | `u32`           | ka9q-radio channel identifier                      |
| `freq_mhz`     | `f64`           | Derived from SSRC (`ssrc / 1000.0`)                |
| `received_at`  | `SystemTime`    | Wall-clock decode time                             |
| `source`       | `String`        | e.g. `"WA0DE-9"`                                   |
| `destination`  | `String`        | AX.25 "to" address (APRS tocall, e.g. `"APDR15"`)  |
| `via`          | `Vec<String>`   | Digipeater path callsigns in order                 |
| `via_heard`    | `Vec<bool>`     | H-bit state for each `via` entry                   |
| `heard_direct` | `bool`          | True iff no H-bits set                             |
| `heard_from`   | `String`        | Last real station with H-bit set, or `source`      |
| `dti`          | `Option<u8>`    | APRS Data Type Identifier (first info byte)        |
| `info`         | `Vec<u8>`       | Raw info field (may contain non-ASCII for Mic-E)   |
| `text`         | `String`        | TNC2 format: `SRC>DST,VIA,...:info`                |
| `raw_ax25`     | `Vec<u8>`       | FCS-stripped frame bytes                           |
| `audio_level`  | `AudioLevel`    | rec/mark/space at decode time (see below)          |
| `first_slice`  | `usize`         | Lowest slicer index that decoded the frame         |
| `slicer_hits`  | `u8`            | How many slicers independently agreed on the frame |

`heard_from` deserves special mention: it walks the path *backward* and
skips entries that are APRS routing aliases (`WIDE1`–`WIDE7`, `TRACE*`,
`RELAY`, `ECHO`, `GATE`) or iGate annotations (`TCPIP`, `TCPXX`, `NOGATE`,
`RFONLY`, `IGATECALL`). For a packet `N7UW-1>APMI06,NCFPD*,SIMLA*,WIDE2*:`
you get `heard_from = "SIMLA"`, not `"WIDE2"`.

`dti` is the raw byte (`b'!'`, `b'='`, `b':'`, `b';'`, `b'\''`, etc.) so
you can match without parsing the rest of the info field — useful for
routing position reports to one queue and messages to another.

### Quick filter examples

Show only directly-heard packets:

```rust
while let Some(pkt) = rx.recv().await {
    if pkt.heard_direct {
        println!("direct: {}", pkt.text);
    }
}
```

Route by APRS data type:

```rust
while let Some(pkt) = rx.recv().await {
    match pkt.dti {
        Some(b'!') | Some(b'=') | Some(b'/') | Some(b'@') => handle_position(&pkt),
        Some(b':') => handle_message(&pkt),
        Some(b';') => handle_object(&pkt),
        Some(b'T') => handle_telemetry(&pkt),
        _ => handle_other(&pkt),
    }
}
```

Dedupe by content (multiple slicers can independently decode the same
frame within an audio block — `slicer_hits` counts them, and frames
across blocks within 3 s are already suppressed internally, but
cross-receiver dedup is your responsibility):

```rust
use std::collections::HashMap;
use std::time::{Duration, Instant};

let mut seen: HashMap<Vec<u8>, Instant> = HashMap::new();
let dedup_window = Duration::from_secs(30);

while let Some(pkt) = rx.recv().await {
    let now = Instant::now();
    seen.retain(|_, t| now.duration_since(*t) < dedup_window);
    if seen.insert(pkt.raw_ax25.clone(), now).is_some() {
        continue; // duplicate
    }
    process(&pkt);
}
```

## Configuration file format

```toml
[decoder]
slicers = 8          # 1-16 parallel amplitude-imbalance slicers
fix_bits = "single"  # "none" | "single" | "double" — CRC error recovery

# One or more ka9q-radio audio sources.
[[source]]
host = "packet.local"  # mDNS hostname or 239.x multicast address
port = 5004            # RTP UDP port (ka9q-radio default: 5004)
interface = ""         # Bind interface IP; empty = OS default
jitter_buffer = 2      # Reorder window in RTP packets (~80 ms at 24 kHz)
ssrc = []              # SSRC allowlist; empty = all SSRCs on this group

# Add more sources as additional [[source]] sections:
# [[source]]
# host = "packet2.local"
# port = 5004
```

The frequency for each packet is derived from its SSRC at runtime, so
there is no `freq_mhz` config field. Override individual decoder
parameters per process; the same `DecoderConfig` applies to every source
in the file.

## Audio levels (`rec`, `mark`, `space`)

`AudioLevel` is a snapshot taken at the end of the audio block containing
each decoded packet. The three values are calibrated to be stable across
consecutive packets and directly comparable across different SSRCs.

### How the values are calculated

Input audio is normalized to `[-1.0, +1.0]` (full-scale s16 → ±1.0). All
three levels are tracked with a slow-attack/slow-decay IIR (fast attack
0.14, slow decay 0.000018 — 5× slower than the demodulation AGC) so they
stay stable across an entire packet.

- **`rec`** = `(alevel_rec_peak − alevel_rec_valley) × 100`, clamped to
  `u8 [0, 255]`.
  - `alevel_rec_peak` and `alevel_rec_valley` are IIR trackers running
    directly on the raw normalized sample stream — one chases the
    positive extreme, one chases the negative.
  - By construction `peak ≥ valley`, so the value is always non-negative.

- **`mark`** = `alevel_mark_peak × 200`, clamped to `u8 [0, 255]`.
  - The audio passes through a bandpass pre-filter, then IQ-mixed at
    1200 Hz and lowpassed. `m_amp = √(m_I² + m_Q²)` is the instantaneous
    envelope of the 1200 Hz component. `alevel_mark_peak` is the slow IIR
    tracker over `m_amp`.
  - IQ demodulation halves the envelope (energy split between I and Q
    legs), which is why the scaling constant is `200` vs `rec`'s `100` —
    so a clean full-scale tone lands near 100 on both scales.

- **`space`** = `alevel_space_peak × 200`, clamped to `u8 [0, 255]`. Same
  as `mark` but with the IQ mixer at 2200 Hz.

### Ranges and reference points

| Quantity   | Min | Practical max | u8 ceiling |
|------------|-----|---------------|------------|
| **rec**    | 0   | ~200          | 255        |
| **mark**   | 0   | ~100          | 255        |
| **space**  | 0   | ~100          | 255        |

The u8 ceiling (255) is only reachable with pathological input (DC
offset pushing the signal beyond ±1.0). With valid s16 RTP audio you
can't exceed the practical max.

For an unbiased sine of amplitude A in normalized units:

- `rec ≈ 200·A`
- `mark ≈ 100·A` if the tone is at exactly 1200 Hz (0 if it isn't)
- `space ≈ 100·A` if the tone is at exactly 2200 Hz (0 if it isn't)

Examples:

| Input                                  | rec  | mark | space |
|----------------------------------------|------|------|-------|
| silence                                | 0    | 0    | 0     |
| amp 0.25 sine, off-tone                | ~50  | 0    | 0     |
| amp 0.5 sine at 1200 Hz                | ~100 | ~50  | ~0    |
| amp 1.0 (full-scale) sine at 1200 Hz   | ~200 | ~100 | ~0    |
| amp 1.0 (full-scale) sine at 2200 Hz   | ~200 | ~0   | ~100  |

### Typical APRS values

For a well-adjusted real-world APRS signal (modulation deviation in spec,
no clipping, both tones present, no significant hum or DC bias):

- **rec**: 30–70
- **mark / space**: 10–40, with the two within ~2× of each other

Values well outside those ranges hint at the kinds of issues described
below.

### What the values indicate

**`rec`** — total baseband audio energy. Useful for:
- Spotting **over-deviated** transmitters (rec pushing toward the
  ceiling, audio likely clipped).
- Spotting **under-deviated** transmitters (rec well below typical,
  weak modulation).
- Detecting **DC offsets / hum / squelch tails** that inflate rec
  without contributing to mark/space.

**`mark`** and **`space`** — energy at exactly 1200 Hz and 2200 Hz.
Useful for:
- Confirming both tones are actually present (a stuck-tone transmitter
  would show one near zero).
- Checking the **mark/space ratio** — a well-adjusted system has them
  within ~2× of each other. A big imbalance usually means
  **pre/de-emphasis mismatch**: pre-emphasis on the TX side boosts high
  frequencies, de-emphasis on the RX side cuts them. If they don't
  match (e.g. flat audio out of ka9q-radio but TX uses pre-emphasis),
  the 2200 Hz space tone reads higher than the 1200 Hz mark tone, or
  vice versa.
- The combination `rec − (mark + space)` gives a rough sense of how
  much **non-tone energy** is in the audio: noise, hum, distortion,
  adjacent-channel bleed.

**What they're not good for:** RF signal strength, SNR, distance, or
path quality. For those, ka9q-radio publishes per-channel SNR and power
figures in its status multicast — consume that directly if you want to
correlate with RF conditions.

Think of `rec`/`mark`/`space` as a **transmitter audio fingerprint** —
useful for spotting misadjusted TXs and pre/de-emphasis mismatches, but
completely decoupled from how strong the signal got to your antenna.

## Running the example

A reference consumer is included at `examples/aprs-listen.rs`:

```sh
# uses ./examples/config.toml by default
cargo run --release --example aprs-listen

# or point at any other config file
cargo run --release --example aprs-listen -- /path/to/my.toml

# verbose tracing
RUST_LOG=debug cargo run --release --example aprs-listen
```

Output is a tabular dump of every decoded packet with audio levels,
slicer agreement, frequency, direct-vs-digipeated marker, the
`heard_from` station, and the TNC2 text.

## Architecture overview

```
RTP listener (async tokio task)
    │  parses UDP packets, dejitters per SSRC
    ▼
per-SSRC AudioBlock
    │  one bounded sync channel per SSRC
    ▼
StreamDecoder (blocking thread)
    │  AFSK demod → multi-slicer → HDLC framer → CRC validate → AX.25 parse
    ▼
AprsPacket
    │  one mpsc::Sender shared across all SSRCs
    ▼
your code (AprsListener.run() return value)
```

- `src/rtp/` — listener, RTP header parsing, jitter buffer, payload decode
- `src/afsk/` — Profile A AFSK demodulator, AGC, DPLL slicers
- `src/dsp/` — FIR filter design utilities (kernels, windows)
- `src/hdlc/` — bit-stuffed HDLC framer + CRC FEC
- `src/ax25/` — AX.25 UI frame parser
- `src/aprs/` — TNC2 text formatter
- `src/pipeline/` — per-SSRC stream-decoder spawning and routing
- `src/config.rs` — config types (`SourceConfig`, `DecoderConfig`, `FixBits`)

## Error handling

`AprsListener::run` returns `Result<mpsc::Receiver<AprsPacket>>` with
`aprs_rtp::Error` covering address resolution, multicast join, and RTP parse
errors. Once the receiver is in hand, internal task
errors are surfaced through tracing rather than the channel — packet
delivery continues across transient socket/decoder hiccups.

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option.
