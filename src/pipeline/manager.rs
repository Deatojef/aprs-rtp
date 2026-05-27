use std::collections::HashMap;
use tokio::sync::mpsc;

use crate::{
    config::{DecoderConfig, SourceConfig},
    pipeline::stream_decoder,
    rtp::{listener, session::AudioBlock},
    AprsPacket,
};

/// Receive `AudioBlock`s from the RTP listener and dispatch each to a per-SSRC
/// `StreamDecoder` blocking task, creating new decoders on first sight of an SSRC.
///
/// Runs as an async tokio task; blocks `audio_rx` are forwarded synchronously to
/// the blocking DSP threads via bounded `std::sync::mpsc::SyncSender` channels.
pub async fn run(
    source: SourceConfig,
    decoder: DecoderConfig,
    aprs_tx: mpsc::Sender<AprsPacket>,
) -> crate::Result<()> {
    let freq_mhz = source.freq_mhz;
    let mut audio_rx = listener::spawn(source).await?;

    // Map SSRC → sender to the per-SSRC blocking DSP thread.
    let mut decoders: HashMap<u32, std::sync::mpsc::SyncSender<AudioBlock>> = HashMap::new();

    while let Some(block) = audio_rx.recv().await {
        let ssrc = block.ssrc;
        let sample_rate = block.sample_rate;

        // Apply SSRC filter if configured (handled in listener; this is belt-and-suspenders).
        let entry = decoders.entry(ssrc).or_insert_with(|| {
            tracing::info!(ssrc, sample_rate, freq_mhz, "new SSRC — spawning stream decoder");
            stream_decoder::spawn(ssrc, decoder.clone(), sample_rate, freq_mhz, aprs_tx.clone())
        });

        // If the blocking thread died (channel disconnected), remove and respawn.
        if entry.try_send(block).is_err() {
            tracing::warn!(ssrc, "stream decoder stalled or closed; respawning");
            decoders.remove(&ssrc);
        }
    }

    tracing::info!("RTP audio source closed; manager exiting");
    Ok(())
}
