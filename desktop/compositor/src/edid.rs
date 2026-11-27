//! Who a screen is, read from its EDID (lot D-26b): the make (the three letters of its maker),
//! the model (its name, or its product code) and the serial number (written, or its number).
//! `displays.conf` keeps each screen's setting under these three names (ADR 0025).
//!
//! The EDID comes from the screen, through the card: it is read as untrusted bytes, only its
//! first block of 128, and a block whose header or checksum is wrong gives nothing.

use desktop_config::displays::Identity;

/// Length of the base block.
const BLOCK: usize = 128;
const HEADER: [u8; 8] = [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00];

/// The identity of the screen whose EDID is `edid`, if it is a valid one.
pub fn identity(edid: &[u8]) -> Option<Identity> {
    let block = edid.get(..BLOCK)?;
    if block[..8] != HEADER || block.iter().fold(0u8, |sum, b| sum.wrapping_add(*b)) != 0 {
        return None;
    }
    // Three letters of five bits each, 'A' being 1.
    let id = u16::from_be_bytes([block[8], block[9]]);
    let letter = |shift: u16| {
        let code = ((id >> shift) & 0x1f) as u8;
        (1..=26).contains(&code).then(|| char::from(b'@' + code))
    };
    let make: String = [letter(10)?, letter(5)?, letter(0)?].iter().collect();
    let product = u16::from_le_bytes([block[10], block[11]]);
    let number = u32::from_le_bytes([block[12], block[13], block[14], block[15]]);
    let (mut model, mut serial) = (None, None);
    // Four descriptors of 18 bytes; a display descriptor starts with three zeros, then its tag.
    for descriptor in block[54..126].as_chunks::<18>().0 {
        if descriptor[..3] != [0, 0, 0] {
            continue;
        }
        let text = || {
            let bytes = &descriptor[5..18];
            let end = bytes
                .iter()
                .position(|b| *b == b'\n')
                .unwrap_or(bytes.len());
            let text: String = bytes[..end]
                .iter()
                .filter(|b| b.is_ascii_graphic() || **b == b' ')
                .map(|b| char::from(*b))
                .collect();
            Some(text.trim().to_owned()).filter(|t| !t.is_empty())
        };
        match descriptor[3] {
            0xfc => model = text(),
            0xff => serial = text(),
            _ => {}
        }
    }
    let model = model.unwrap_or_else(|| format!("0x{product:04X}"));
    let serial = serial.unwrap_or_else(|| match number {
        0 => String::new(),
        n => n.to_string(),
    });
    Some(Identity::new(&make, &model, &serial))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up EDID: maker "SHE", product 0x1234, serial number 42, a name and a serial
    /// written in descriptors.
    fn edid(name: Option<&str>, serial: Option<&str>) -> Vec<u8> {
        let mut block = vec![0u8; BLOCK];
        block[..8].copy_from_slice(&HEADER);
        let code = |c: u8| u16::from(c - b'@');
        let id = (code(b'S') << 10) | (code(b'H') << 5) | code(b'E');
        block[8..10].copy_from_slice(&id.to_be_bytes());
        block[10..12].copy_from_slice(&0x1234u16.to_le_bytes());
        block[12..16].copy_from_slice(&42u32.to_le_bytes());
        for (i, (tag, text)) in [(0xfc, name), (0xff, serial)].into_iter().enumerate() {
            let Some(text) = text else { continue };
            let at = 54 + 18 * (i + 1);
            block[at + 3] = tag;
            let mut bytes = text.as_bytes().to_vec();
            bytes.push(b'\n');
            bytes.resize(13, b' ');
            block[at + 5..at + 18].copy_from_slice(&bytes);
        }
        let sum = block[..127].iter().fold(0u8, |s, b| s.wrapping_add(*b));
        block[127] = 0u8.wrapping_sub(sum);
        block
    }

    #[test]
    fn names_come_from_the_descriptors() {
        let identity = identity(&edid(Some("Screen 27"), Some("A1B2"))).unwrap();
        assert_eq!(identity, Identity::new("SHE", "Screen 27", "A1B2"));
    }

    #[test]
    fn without_descriptors_the_codes_are_used() {
        let identity = identity(&edid(None, None)).unwrap();
        assert_eq!(identity, Identity::new("SHE", "0x1234", "42"));
    }

    #[test]
    fn a_damaged_edid_gives_nothing() {
        let mut bad = edid(Some("Screen"), None);
        bad[20] ^= 1;
        assert_eq!(identity(&bad), None);
        assert_eq!(identity(&bad[..100]), None);
        assert_eq!(identity(&[0; 256]), None);
        let mut control = edid(Some("Bad\u{1}name"), None);
        assert_eq!(identity(&control).unwrap().model, "Badname");
        control[0] = 1;
        assert_eq!(identity(&control), None);
    }
}
