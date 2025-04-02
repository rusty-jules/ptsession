pub use std::io::{self, Read, Seek, SeekFrom};

pub trait Endianness {
    fn is_bigendian(&self) -> bool;
}

macro_rules! read_endian {
    ($fname:ident, $num:expr, $ret:ty) => {
        fn $fname(&mut self) -> Result<$ret, io::Error> {
            const NUM_BYTES: usize = $num / 8;
            let mut val: $ret = 0;
            let mut limit = if self.is_bigendian() {
                $num - 8
            } else {
                0
            };
            let mut places: [u8; NUM_BYTES] = [0; NUM_BYTES];
            self.read_exact(&mut places)?;
            for i in 0..NUM_BYTES {
                val = (places[i] as $ret << limit) | val;
                if self.is_bigendian() {
                    limit -= 8;
                } else {
                    limit += 8;
                }
            }
            Ok(val)
        }
    };
}

pub trait ReadExt: Read + Seek + Endianness {
    fn read_u8(&mut self) -> Result<u8, io::Error> {
        let mut buf = [0; 1];
        self.read_exact(&mut buf)?;
        Ok(buf[0])
    }

    read_endian!(read_u16, 16, u16);
    read_endian!(read_u24, 24, u32);
    read_endian!(read_u32, 32, u32);
    read_endian!(read_u40, 40, u64);
    read_endian!(read_u64, 64, usize);

    fn parse_bytes(&mut self, num_bytes: u8) -> Result<usize, io::Error> {
        match num_bytes {
            5 => self.read_u40().map(|r| r as usize),
            4 => self.read_u32().map(|r| r as usize),
            3 => self.read_u24().map(|r| r as usize),
            2 => self.read_u16().map(|r| r as usize),
            1 => self.read_u8().map(|r| r as usize),
            _ => Ok(0)
        }
    }

    fn parse_three_point(&mut self) -> Result<(usize, usize, usize), io::Error> {
        let pos = self.seek(SeekFrom::Current(0))?;

        let (offset_bytes, len_bytes, start_bytes) = if self.is_bigendian() {
            self.seek(SeekFrom::Current(2))?;
            let start_bytes = (self.read_u8()? & 0xf0) >> 4;
            let len_bytes = (self.read_u8()? & 0xf0) >> 4;
            let offset_bytes = (self.read_u8()? & 0xf0) >> 4;
            (offset_bytes, len_bytes, start_bytes)
        } else {
            self.seek(SeekFrom::Current(1))?; 
            (
                (self.read_u8()? & 0xf0) >> 4,
                (self.read_u8()? & 0xf0) >> 4,
                (self.read_u8()? & 0xf0) >> 4
            )
        };

        self.seek(SeekFrom::Start(pos + 5))?;
        let offset = self.parse_bytes(offset_bytes)?;
        let len = self.parse_bytes(len_bytes)?;
        let start = self.parse_bytes(start_bytes)?;

        Ok((offset, start, len))
    }
}

impl<T> ReadExt for T where T: Read + Seek + Endianness {}
