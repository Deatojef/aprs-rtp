/// aprs-listen — prints decoded APRS packets from one or more ka9q-radio RTP streams.
///
/// Usage:
///   cargo run --example aprs-listen [CONFIG_PATH]
///
/// CONFIG_PATH defaults to "examples/config.toml" in the current directory.
///
/// Output columns (one row per decoded packet):
///
///   SL     — slicer index (0-based) that first completed the frame
///   HITS   — "N/M": N slicers of M independently decoded this frame (higher = stronger)
///   REC    — overall received audio level 0–200  (200 = full-scale S16 audio)
///   MARK   — 1200 Hz tone IQ envelope level 0–100
///   SPACE  — 2200 Hz tone IQ envelope level 0–100
///   FREQ   — tuned frequency in MHz, derived from SSRC (ka9q-radio convention)
///   D      — `D` if heard directly from the source, `*` if a digipeater touched it
///   PACKET — TNC2-format decoded text; * after a via callsign = H-bit set (that
///            digipeater retransmitted the packet; last * = station we heard over RF)
///
/// REC/MARK/SPACE use direwolf's normalization (5× slower IIR than the demodulation AGC)
/// so values are stable across packets and comparable across different SSRC streams.
///
/// Informational events (new SSRCs, errors) go to stderr.
/// Set RUST_LOG=debug to see per-audio-block tracing from the library.
use aprs_rtp::{
    config::{DecoderConfig, SourceConfig},
    AprsListener, AprsPacket,
};
use serde::Deserialize;
use tokio::sync::mpsc;

/// Top-level config file structure — defined here so the library stays format-agnostic.
#[derive(Debug, Deserialize)]
struct AppConfig {
    #[serde(default)]
    decoder: DecoderConfig,
    #[serde(rename = "source")]
    sources: Vec<SourceConfig>,
}

// Column header and separator — must stay in sync with the println! format below.
// Fields: SL(3) · HITS(5) · REC(3) · MARK(4) · SPACE(5) · FREQ(8) · D(1) · FROM · PACKET
const HEADER: &str = " SL   HITS  REC  MARK  SPACE  FREQ/MHz  D  FROM       PACKET";
const SEP:    &str = "---  -----  ---  ----  -----  --------  -  ---------  ------";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing: default INFO, override with RUST_LOG.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let config_path = std::env::args().nth(1).unwrap_or_else(|| "examples/config.toml".into());
    let text = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("cannot read {config_path}: {e}"))?;
    let cfg: AppConfig = toml::from_str(&text)
        .map_err(|e| format!("parse error in {config_path}: {e}"))?;

    if cfg.sources.is_empty() {
        eprintln!("error: no [[source]] entries in {config_path}");
        std::process::exit(1);
    }

    let num_slicers = cfg.decoder.slicers;

    // Unified output channel — all sources feed into this.
    let (tx, mut rx) = mpsc::channel::<AprsPacket>(512);

    for source in cfg.sources {
        let listener = AprsListener::new(source, cfg.decoder.clone());
        let mut pkt_rx = listener.run().await?;
        let fwd = tx.clone();
        tokio::spawn(async move {
            while let Some(pkt) = pkt_rx.recv().await {
                if fwd.send(pkt).await.is_err() {
                    break; // main task exited
                }
            }
        });
    }
    // Drop our copy so `rx` closes when all listener tasks finish.
    drop(tx);

    println!("{HEADER}");
    println!("{SEP}");

    let mut row = 0usize;
    while let Some(pkt) = rx.recv().await {
        // Re-print the header every 40 rows so it stays visible when scrolling.
        if row > 0 && row % 40 == 0 {
            println!("{SEP}");
            println!("{HEADER}");
            println!("{SEP}");
        }
        row += 1;

        let al = pkt.audio_level;
        let hits = format!("{}/{}", pkt.slicer_hits, num_slicers);
        let direct = if pkt.heard_direct { 'D' } else { '*' };
        println!(
            "{:>3}  {:>5}  {:>3}  {:>4}  {:>5}  {:8.3}  {:>3} {:>9}  {}",
            pkt.first_slice,
            hits,
            al.rec,
            al.mark,
            al.space,
            pkt.freq_mhz,
            direct,
            pkt.heard_from,
            pkt.text,
        );
    }

    Ok(())
}
