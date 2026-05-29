/// Parsed RTP header fields used by the pipeline.
#[derive(Debug, Clone)]
pub struct RtpHeader {
    pub payload_type: u8,
    pub seq: u16,
    pub ssrc: u32,
}

/// Parse an RTP header from a raw UDP datagram.
///
/// Returns the parsed header and the byte offset where the payload begins,
/// or an error if the packet is malformed.
pub fn parse(buf: &[u8]) -> crate::Result<(RtpHeader, usize)> {
    if buf.len() < 12 {
        return Err(crate::Error::PacketTooShort(buf.len()));
    }

    // Word 0: V(2) P(1) X(1) CC(4) M(1) PT(7) SEQ(16)
    let word0 = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let version = ((word0 >> 30) & 0x3) as u8;
    if version != 2 {
        return Err(crate::Error::RtpVersion(version));
    }
    let has_extension = ((word0 >> 28) & 1) != 0;
    let cc = ((word0 >> 24) & 0xf) as usize;
    let payload_type = ((word0 >> 16) & 0x7f) as u8;
    let seq = (word0 & 0xffff) as u16;

    let ssrc = u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]);

    // Fixed header (12) + CSRC list (cc * 4)
    let mut offset = 12 + cc * 4;

    if has_extension {
        // Extension header: first 2 bytes are profile, next 2 bytes are length in 32-bit words
        if buf.len() < offset + 4 {
            return Err(crate::Error::PacketTooShort(buf.len()));
        }
        let ext_len = u16::from_be_bytes([buf[offset + 2], buf[offset + 3]]) as usize;
        offset += 4 + ext_len * 4;
    }

    if buf.len() < offset {
        return Err(crate::Error::PacketTooShort(buf.len()));
    }

    Ok((
        RtpHeader {
            payload_type,
            seq,
            ssrc,
        },
        offset,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_rtp() {
        // Version=2, no padding, no extension, CC=0, no marker, PT=116, seq=1, ts=0, ssrc=0xDEAD
        let mut buf = [0u8; 12];
        buf[0] = 0x80; // V=2, P=0, X=0, CC=0
        buf[1] = 116; // M=0, PT=116
        buf[2] = 0x00;
        buf[3] = 0x01; // seq=1
        buf[4..8].copy_from_slice(&0u32.to_be_bytes()); // timestamp=0
        buf[8..12].copy_from_slice(&0xDEAD_u32.to_be_bytes()); // ssrc
        let (hdr, offset) = parse(&buf).unwrap();
        assert_eq!(hdr.payload_type, 116);
        assert_eq!(hdr.seq, 1);
        assert_eq!(hdr.ssrc, 0xDEAD);
        assert_eq!(offset, 12);
    }

    #[test]
    fn rejects_wrong_version() {
        let buf = [0x40u8; 12]; // V=1
        assert!(matches!(parse(&buf), Err(crate::Error::RtpVersion(1))));
    }

    #[test]
    fn rejects_short_packet() {
        let buf = [0x80u8; 11];
        assert!(matches!(parse(&buf), Err(crate::Error::PacketTooShort(11))));
    }
}
