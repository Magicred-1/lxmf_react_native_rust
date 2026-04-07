//! BLE and LoRa frame codecs
//!
//! - HDLC: Used for phone-to-phone BLE mesh (0x7E flag, 0x7D escape)
//! - KISS: Used for RNode LoRa devices (0xC0 FEND, 0xDB FESC)

// --- HDLC Codec ---

const HDLC_FLAG: u8 = 0x7E;
const HDLC_ESC: u8 = 0x7D;
const HDLC_ESC_FLAG: u8 = 0x5E; // 0x7E ^ 0x20
const HDLC_ESC_ESC: u8 = 0x5D; // 0x7D ^ 0x20

/// Encode a payload into an HDLC frame
pub fn hdlc_encode(data: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(data.len() + 4);
    frame.push(HDLC_FLAG);
    for &byte in data {
        match byte {
            HDLC_FLAG => {
                frame.push(HDLC_ESC);
                frame.push(HDLC_ESC_FLAG);
            }
            HDLC_ESC => {
                frame.push(HDLC_ESC);
                frame.push(HDLC_ESC_ESC);
            }
            _ => frame.push(byte),
        }
    }
    frame.push(HDLC_FLAG);
    frame
}

/// Stateful HDLC deframer — accumulates bytes and yields complete frames
pub struct HdlcDeframer {
    buf: Vec<u8>,
    in_frame: bool,
    escape_next: bool,
}

impl HdlcDeframer {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(512),
            in_frame: false,
            escape_next: false,
        }
    }

    /// Feed bytes into the deframer. Returns any complete frames.
    pub fn feed(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        let mut frames = Vec::new();

        for &byte in data {
            if byte == HDLC_FLAG {
                if self.in_frame && !self.buf.is_empty() {
                    frames.push(std::mem::take(&mut self.buf));
                }
                self.in_frame = true;
                self.escape_next = false;
                self.buf.clear();
                continue;
            }

            if !self.in_frame {
                continue;
            }

            if self.escape_next {
                self.buf.push(byte ^ 0x20);
                self.escape_next = false;
            } else if byte == HDLC_ESC {
                self.escape_next = true;
            } else {
                self.buf.push(byte);
            }
        }

        frames
    }

    pub fn reset(&mut self) {
        self.buf.clear();
        self.in_frame = false;
        self.escape_next = false;
    }
}

impl Default for HdlcDeframer {
    fn default() -> Self {
        Self::new()
    }
}

// --- KISS Codec ---

const KISS_FEND: u8 = 0xC0;
const KISS_FESC: u8 = 0xDB;
const KISS_TFEND: u8 = 0xDC; // transposed FEND
const KISS_TFESC: u8 = 0xDD; // transposed FESC
const KISS_CMD_DATA: u8 = 0x00;

/// Encode a payload into a KISS frame (for RNode/LoRa)
pub fn kiss_encode(data: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(data.len() + 4);
    frame.push(KISS_FEND);
    frame.push(KISS_CMD_DATA);
    for &byte in data {
        match byte {
            KISS_FEND => {
                frame.push(KISS_FESC);
                frame.push(KISS_TFEND);
            }
            KISS_FESC => {
                frame.push(KISS_FESC);
                frame.push(KISS_TFESC);
            }
            _ => frame.push(byte),
        }
    }
    frame.push(KISS_FEND);
    frame
}

/// Stateful KISS deframer
pub struct KissDeframer {
    buf: Vec<u8>,
    in_frame: bool,
    escape_next: bool,
    command: u8,
}

impl KissDeframer {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(512),
            in_frame: false,
            escape_next: false,
            command: 0,
        }
    }

    /// Feed bytes into the deframer. Returns (command, payload) for complete frames.
    pub fn feed(&mut self, data: &[u8]) -> Vec<(u8, Vec<u8>)> {
        let mut frames = Vec::new();

        for &byte in data {
            if byte == KISS_FEND {
                if self.in_frame && !self.buf.is_empty() {
                    frames.push((self.command, std::mem::take(&mut self.buf)));
                }
                self.in_frame = true;
                self.escape_next = false;
                self.buf.clear();
                self.command = 0xFF; // sentinel until we read the command byte
                continue;
            }

            if !self.in_frame {
                continue;
            }

            if self.command == 0xFF {
                // First byte after FEND is the command
                self.command = byte;
                continue;
            }

            if self.escape_next {
                match byte {
                    KISS_TFEND => self.buf.push(KISS_FEND),
                    KISS_TFESC => self.buf.push(KISS_FESC),
                    _ => self.buf.push(byte),
                }
                self.escape_next = false;
            } else if byte == KISS_FESC {
                self.escape_next = true;
            } else {
                self.buf.push(byte);
            }
        }

        frames
    }

    pub fn reset(&mut self) {
        self.buf.clear();
        self.in_frame = false;
        self.escape_next = false;
    }
}

impl Default for KissDeframer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hdlc_roundtrip() {
        let data = vec![0x01, 0x7E, 0x7D, 0xFF, 0x00];
        let encoded = hdlc_encode(&data);
        let mut deframer = HdlcDeframer::new();
        let frames = deframer.feed(&encoded);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0], data);
    }

    #[test]
    fn hdlc_multiple_frames() {
        let a = vec![0xAA, 0xBB];
        let b = vec![0xCC, 0xDD];
        let mut stream = hdlc_encode(&a);
        stream.extend(hdlc_encode(&b));

        let mut deframer = HdlcDeframer::new();
        let frames = deframer.feed(&stream);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], a);
        assert_eq!(frames[1], b);
    }

    #[test]
    fn kiss_roundtrip() {
        let data = vec![0x01, 0xC0, 0xDB, 0xFF];
        let encoded = kiss_encode(&data);
        let mut deframer = KissDeframer::new();
        let frames = deframer.feed(&encoded);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].0, KISS_CMD_DATA);
        assert_eq!(frames[0].1, data);
    }

    #[test]
    fn kiss_fragmented_delivery() {
        let data = vec![0x01, 0x02, 0x03];
        let encoded = kiss_encode(&data);
        let mid = encoded.len() / 2;

        let mut deframer = KissDeframer::new();
        let frames1 = deframer.feed(&encoded[..mid]);
        assert!(frames1.is_empty());
        let frames2 = deframer.feed(&encoded[mid..]);
        assert_eq!(frames2.len(), 1);
        assert_eq!(frames2[0].1, data);
    }
}
