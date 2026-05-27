/// A parsed AX.25 UI frame.
///
/// Addresses are decoded from the 7-byte-per-callsign wire format:
///   bytes 0..5: ASCII characters each shifted left 1 bit (>> 1 to recover)
///   byte 6: SSID in bits [4:1]; H-bit in bit 7; end-of-address marker in bit 0
///
/// Only UI frames are accepted (control = 0x03, PID = 0xF0).
#[derive(Debug, Clone)]
pub struct Ax25Frame {
    pub source: String,
    pub destination: String,
    /// Digipeater path entries; H-bit state is encoded in `via_heard`.
    pub via: Vec<String>,
    /// For each via entry: true if the H-bit (bit 7 of the 7th address byte) is set.
    /// H-bit = 1 means this digipeater has already repeated the packet.
    pub via_heard: Vec<bool>,
    /// Information field (everything after control + PID bytes).
    pub info: Vec<u8>,
}

impl Ax25Frame {
    /// Decode a raw AX.25 frame (FCS already stripped).
    ///
    /// Returns `None` if the frame is malformed, too short, or not a UI frame.
    pub fn parse(data: &[u8]) -> Option<Self> {
        // Minimum: destination(7) + source(7) + control(1) + pid(1) = 16 bytes.
        if data.len() < 16 {
            return None;
        }

        let mut pos = 0;

        // Destination address.
        if pos + 7 > data.len() {
            return None;
        }
        let (dest, dest_end) = decode_address(&data[pos..pos + 7]);
        pos += 7;
        // End-of-address bit should NOT be set on destination in normal frames,
        // but we don't enforce it — source or via entries end the address field.
        let _ = dest_end;

        // Source address.
        if pos + 7 > data.len() {
            return None;
        }
        let (src, src_end) = decode_address(&data[pos..pos + 7]);
        pos += 7;

        // Optional digipeater addresses: continue while end-of-address bit not set.
        let mut via: Vec<String> = Vec::new();
        let mut via_heard: Vec<bool> = Vec::new();
        if !src_end {
            loop {
                if pos + 7 > data.len() {
                    return None; // truncated
                }
                let (call, end, h_bit) = decode_via(&data[pos..pos + 7]);
                pos += 7;
                via.push(call);
                via_heard.push(h_bit);
                if end {
                    break;
                }
            }
        }

        // Control byte: must be 0x03 for UI frame.
        if pos >= data.len() {
            return None;
        }
        let control = data[pos];
        pos += 1;
        if control != 0x03 {
            return None;
        }

        // PID byte: must be 0xF0 for No Layer 3.
        if pos >= data.len() {
            return None;
        }
        let pid = data[pos];
        pos += 1;
        if pid != 0xF0 {
            return None;
        }

        // Information field: remainder of frame.
        let info = data[pos..].to_vec();

        Some(Ax25Frame {
            source: src,
            destination: dest,
            via,
            via_heard,
            info,
        })
    }

    /// True when the packet was received directly (no H-bits set in the via path).
    ///
    /// If every digipeater entry in the via path has its H-bit clear, the packet
    /// has not been repeated — we heard it directly from the originating station.
    pub fn heard_direct(&self) -> bool {
        self.via_heard.iter().all(|&h| !h)
    }

    /// The callsign most likely responsible for the RF signal we received.
    ///
    /// If any H-bits are set, this is the last digipeater that set its H-bit
    /// (i.e. the last station to actually transmit the packet over the air).
    /// If no H-bits are set, it's the source callsign.
    pub fn heard_from(&self) -> &str {
        // Find the last via entry with H-bit set.
        for (i, &h) in self.via_heard.iter().enumerate().rev() {
            if h {
                return &self.via[i];
            }
        }
        &self.source
    }
}

/// Decode a 7-byte AX.25 address field into a callsign string.
///
/// Returns (callsign, end_of_address_bit).
fn decode_address(bytes: &[u8]) -> (String, bool) {
    debug_assert_eq!(bytes.len(), 7);
    let mut call = String::with_capacity(9);
    for &b in &bytes[0..6] {
        let ch = (b >> 1) as char;
        if ch != ' ' {
            call.push(ch);
        }
    }
    let ssid_byte = bytes[6];
    let ssid = (ssid_byte >> 1) & 0x0F;
    if ssid != 0 {
        call.push('-');
        call.push_str(&ssid.to_string());
    }
    let end_bit = (ssid_byte & 0x01) != 0;
    (call, end_bit)
}

/// Decode a 7-byte AX.25 via (digipeater) address field.
///
/// Returns (callsign, end_of_address_bit, h_bit).
fn decode_via(bytes: &[u8]) -> (String, bool, bool) {
    debug_assert_eq!(bytes.len(), 7);
    let (call, end) = decode_address(bytes);
    let h_bit = (bytes[6] & 0x80) != 0;
    (call, end, h_bit)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode a callsign into 7-byte AX.25 address format.
    fn encode_address(call: &str, ssid: u8, end_bit: bool, h_bit: bool) -> [u8; 7] {
        let mut out = [b' ' << 1; 7]; // pad with space<<1
        let base: &str = if let Some(pos) = call.find('-') {
            &call[..pos]
        } else {
            call
        };
        for (i, ch) in base.chars().enumerate().take(6) {
            out[i] = (ch as u8) << 1;
        }
        let ssid_byte = (ssid << 1)
            | if end_bit { 0x01 } else { 0x00 }
            | if h_bit { 0x80 } else { 0x00 }
            | 0x60; // reserved bits set per AX.25 spec
        out[6] = ssid_byte;
        out
    }

    fn build_ui_frame(
        dest: &str,
        src: &str,
        via: &[(&str, bool)], // (callsign, h_bit)
        info: &[u8],
    ) -> Vec<u8> {
        let mut frame = Vec::new();
        // Destination: end_bit set only if no source/via follow? No — end_bit is on last addr.
        let is_last_dest = via.is_empty() && true; // source always follows destination
        let _ = is_last_dest;
        frame.extend_from_slice(&encode_address(dest, 0, false, false));

        let src_end = via.is_empty();
        // Extract SSID from src if present.
        let (src_base, src_ssid) = parse_ssid(src);
        let mut src_bytes = encode_address(src_base, src_ssid, src_end, false);
        // Remove reserved bits override — just use what encode_address gave us, but fix end.
        if src_end {
            src_bytes[6] |= 0x01;
        } else {
            src_bytes[6] &= !0x01;
        }
        frame.extend_from_slice(&src_bytes);

        for (i, &(call, h)) in via.iter().enumerate() {
            let end = i == via.len() - 1;
            let (base, ssid) = parse_ssid(call);
            let mut via_bytes = encode_address(base, ssid, end, h);
            if end { via_bytes[6] |= 0x01; } else { via_bytes[6] &= !0x01; }
            frame.extend_from_slice(&via_bytes);
        }

        frame.push(0x03); // control: UI
        frame.push(0xF0); // PID: No Layer 3
        frame.extend_from_slice(info);
        frame
    }

    fn parse_ssid(call: &str) -> (&str, u8) {
        if let Some(pos) = call.find('-') {
            let base = &call[..pos];
            let ssid: u8 = call[pos + 1..].parse().unwrap_or(0);
            (base, ssid)
        } else {
            (call, 0)
        }
    }

    #[test]
    fn parse_basic_ui_frame() {
        let info = b"/position data";
        let frame = build_ui_frame("APDR15", "KA9Q-1", &[], info);
        let parsed = Ax25Frame::parse(&frame).expect("should parse");
        assert_eq!(parsed.destination, "APDR15");
        assert_eq!(parsed.source, "KA9Q-1");
        assert!(parsed.via.is_empty());
        assert_eq!(parsed.info, info);
    }

    #[test]
    fn parse_via_with_h_bit() {
        let info = b"test info";
        // Via: WIDE1-1 not heard (H=0), KD9PDP-3 heard (H=1).
        let frame = build_ui_frame(
            "APDR15",
            "KA9Q-1",
            &[("WIDE1-1", false), ("KD9PDP-3", true)],
            info,
        );
        let parsed = Ax25Frame::parse(&frame).expect("should parse");
        assert_eq!(parsed.via.len(), 2);
        assert_eq!(parsed.via_heard, vec![false, true]);
        assert_eq!(parsed.heard_from(), "KD9PDP-3");
        assert!(!parsed.heard_direct());
    }

    #[test]
    fn heard_direct_no_via() {
        let frame = build_ui_frame("APDR15", "KA9Q-1", &[], b"direct");
        let parsed = Ax25Frame::parse(&frame).unwrap();
        assert!(parsed.heard_direct());
        assert_eq!(parsed.heard_from(), "KA9Q-1");
    }

    #[test]
    fn heard_direct_with_unheared_via() {
        // Via exists but H-bit not set → still heard directly.
        let frame = build_ui_frame("APDR15", "KA9Q-1", &[("WIDE1-1", false)], b"direct");
        let parsed = Ax25Frame::parse(&frame).unwrap();
        assert!(parsed.heard_direct());
        assert_eq!(parsed.heard_from(), "KA9Q-1");
    }

    #[test]
    fn rejects_non_ui_frame() {
        let mut frame = build_ui_frame("APDR15", "KA9Q-1", &[], b"test");
        // Change control byte from 0x03 to 0x05 (not UI).
        let ctrl_pos = 14; // 7 (dest) + 7 (src) = 14
        frame[ctrl_pos] = 0x05;
        assert!(Ax25Frame::parse(&frame).is_none());
    }

    #[test]
    fn rejects_wrong_pid() {
        let mut frame = build_ui_frame("APDR15", "KA9Q-1", &[], b"test");
        let pid_pos = 15;
        frame[pid_pos] = 0xCF; // not 0xF0
        assert!(Ax25Frame::parse(&frame).is_none());
    }

    #[test]
    fn rejects_too_short() {
        assert!(Ax25Frame::parse(&[]).is_none());
        assert!(Ax25Frame::parse(&[0u8; 10]).is_none());
    }
}
