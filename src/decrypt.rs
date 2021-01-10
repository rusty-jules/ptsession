use log::debug;

use std::path::Path;
use std::io::{self, Read};
use std::fs;

// Decrypt a PT Session File
pub(crate) fn unxor<P: AsRef<Path>>(path: P) -> Result<Vec<u8>, io::Error> {
    let file = fs::File::open(path)?;
    let len = file.metadata()?.len();

    if len < 0x14 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "File is too small",
        ));
    }

    let mut ptf_unxored = vec![0u8; len as usize];

    // First 20 bytes are unencrypted
    let mut reader = io::BufReader::new(file);
    reader.read_exact(&mut ptf_unxored[..0x14])?;
    debug!("Read first 20 bytes");

    let xor_type = ptf_unxored[0x12];
    let xor_value = ptf_unxored[0x13];

    // xor_type 0x01 = ProTools 5, 6, 7, 8 and 9
    // xor_type 0x05 = ProTools 10, 11, 12
    let xor_delta = match xor_type {
        0x01 => gen_xor_delta(xor_value, 53, false),
        0x05 => gen_xor_delta(xor_value, 11, true),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "PT xor type not recognized",
            ))
        }
    };
    debug!(
        "XOR_TYPE: {:#x} XOR_VALUE: {:#x} XOR_DELTA: {:#x}",
        xor_type, xor_value, xor_delta
    );

    let mut xxor = [0u8; 256];
    // Generate the xor_key
    xxor.iter_mut()
        .enumerate()
        .for_each(|(i, xor)| *xor = ((i as isize * xor_delta as isize) & 0xff) as u8);
    debug!("XOR table generated.");

    // Decrypt the rest of the file
    for (i, ct) in reader.bytes().enumerate() {
        let byte = ct?;
        let index = i + 0x14;
        let xor_index = if xor_type == 0x01 {
            index & 0xff
        } else {
            (index >> 12) & 0xff
        };
        ptf_unxored[index] = byte ^ xxor[xor_index];
    }

    debug!("PTF decrypted");

    Ok(ptf_unxored)
}

fn gen_xor_delta(xor_value: u8, mul: u8, negative: bool) -> i8 {
    for i in 0..=255u16 {
        if ((i * mul as u16) & 0xff) as u8 == xor_value {
            return if negative {
                i as i8 * -1
            //(i | 0x80) as i8
            } else {
                i as i8
            };
        }
    }
    // Should not occur
    debug!("gen_xor_delta failed!");
    return 0;
}

pub(crate) fn find_bitcode(ptf_unxored: &[u8]) -> Option<usize> {
    const BITCODE: [u8; 2] = 0x2f2b_u16.to_be_bytes();
    ptf_unxored
        .windows(BITCODE.len())
        .position(|window| window == &BITCODE)
}
