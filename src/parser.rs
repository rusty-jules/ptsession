use crate::{
    content_description::PTCD,
    read_traits::*,
    error::*,
    session::*,
    decrypt,
};

use log::{debug, warn, trace};

use std::io::Cursor;
use std::convert::TryInto;

macro_rules! filter_blocks {
    ($block_iter:expr, $child:expr) => {
        $block_iter
            .flat_map(|block| block.children.iter())
            .filter(|children| children.content_type == $child as u16)
    };

    ($block_iter:expr, $child:expr => $( $children:expr )=>*) => {
        filter_blocks!({ filter_blocks!($block_iter, $child) }, $($children),+)
    };
}

macro_rules! children_of {
    ($block:expr, $child:expr) => {
        $block.children.iter()
            .filter(|children| children.content_type == $child as u16)
    };

    ($block:expr, $child1:expr, $child2:expr) => {
        $block.children.iter()
            .filter(|children| {
                children.content_type == $child1 as u16 ||
                    children.content_type == $child2 as u16
            })
    };
}

#[derive(Default)]
struct BlockMap {
    wav_blocks: Vec<Block>,
    header_blocks: Vec<Block>,
    marker_blocks: Vec<Block>,
    region_to_wav_blocks: Vec<Block>,
    region_to_track_blocks: Vec<Block>,
    track_blocks: Vec<Block>,
}

pub struct PtSessionParser {
    reader: std::io::Cursor<Vec<u8>>,
    is_bigendian: bool,
    block_map: Option<BlockMap>,
    version: Option<u8>,
}

impl Read for PtSessionParser {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.reader.read(buf)
    }
}

impl Endianness for PtSessionParser {
    #[inline(always)]
    fn is_bigendian(&self) -> bool {
        self.is_bigendian
    }
}

impl Seek for PtSessionParser {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.reader.seek(pos)
    }
}

impl PtSessionParser {
    fn set_position(&mut self, pos: usize) {
        self.reader.set_position(pos as u64)
    }

    fn increment_position(&mut self, increment: usize) {
        self.set_position(self.position() + increment)
    }

    fn position(&self) -> usize {
        self.reader.position() as usize
    }

    pub fn unxored(&self) -> &[u8] {
        self.reader.get_ref()
    }

    pub fn decrypt<P: AsRef<std::path::Path>>(path: P) -> Result<Self, PtError> {
        let ptf_unxored = decrypt::unxor(path)
            .map_err(|e| PtError::Decrypt(e))?;

        // Check BitCode
        debug!("BitCode check...");
        if ptf_unxored[0] != 0x03 && decrypt::find_bitcode(&ptf_unxored[..]).is_none() {
            return Err(PtError::BitCode)
        }

        // Parse Endianness
        debug!("Parsing endianness...");
        let is_bigendian = match ptf_unxored[0x11] {
            1 => true,
            0 => false,
            _ => return Err(PtError::Endianness),
        };

        let reader = Cursor::new(ptf_unxored);
        let mut session_parser = PtSessionParser {
            reader,
            is_bigendian,
            block_map: None,
            version: None,
        };

        // Parse Version
        debug!("Parsing version...");
        session_parser.parse_version()?;

        Ok(session_parser)
    }

    fn parse_str_at(&mut self, pos: usize) -> Result<String, io::Error> {
        self.set_position(pos);
        self.parse_str()
    }

    fn parse_str(&mut self) -> Result<String, io::Error> {
        let len = self.read_u32()? as usize;
        let pos = self.position();
        let end = pos + len;
        trace!("Parsing str. Start {} End {} Len {}", pos, end, len);
        let string = unsafe { std::str::from_utf8_unchecked(&self.reader.get_ref()[pos..end]).into() };
        self.set_position(end);
        Ok(string)
    }

    fn parse_block_at(&mut self, pos: usize, parent: Option<&Block>, level: i32) -> Result<Block, io::Error> {
        const Z_MARK: u8 = 0x5a;

        let len = self.reader.get_ref().len();
        let max = match parent {
            Some(p) => p.size + p.offset,
            None => len,
        };
        self.set_position(pos);

        let z_mark = self.read_u8()?;

        if z_mark != Z_MARK {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "First byte of block must be ZMARK",
            ));
        }

        let mut block = Block {
            z_mark,
            block_type: self.read_u16()?,
            size: self.read_u32()? as usize,
            content_type: self.read_u16()?,
            offset: pos + 7,
            children: vec![],
        };

        if block.size + block.offset > max {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Block is too large",
            ));
        }

        if (block.block_type & 0xff00) != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid Block Type",
            ));
        }

        // Parse the children
        let mut child_jump = 0;
        let mut i = 1;

        while i < block.size && pos + i + child_jump < max {
            let p = pos + i;
            child_jump = 0;
            
            if let Ok(child) = self.parse_block_at(p, Some(&block), level + 1) {
                child_jump = child.size + 7;
                block.children.push(child);
            }

            i += if child_jump > 0 { child_jump } else { 1 };
        }

        Ok(block)
    }

    fn parse_version(&mut self) -> Result<(), PtError> {
        match self.parse_block_at(0x1f, None, 0) {
            Ok(block) => match block.content_type.try_into() {
                Ok(PTCD::INFO_Version) => {
                    // old PT
                    let skip = self.parse_str_at(block.offset + 3)
                        .map_err(PtError::Io)?
                        .len() + 8;
                    self.set_position(block.offset + 3 + skip);
                    self.version = Some(self.read_u32().map_err(PtError::Io)? as u8);
                }
                Ok(PTCD::INFO_Path_of_Session) => {
                    // new PT
                    self.set_position(block.offset + 20);
                    let version = 2 + self.read_u32().map_err(PtError::Io)? as u8;
                    self.version = Some(version);
                }
                _ => {
                    return Err(
                        PtError::Version(
                            format!("Could not parse version block type: {:#04x}",
                            block.content_type)
                    ));
                }
            }
            Err(e) => {
                warn!("Could not parse version block: {}", e);
                let ptf_unxored = self.unxored();
                let mut version = ptf_unxored[0x40];
                if version == 0 {
                    version = ptf_unxored[0x3d];
                }
                if version == 0 {
                    version = ptf_unxored[0x3a] + 2;
                }
                if version != 0 {
                    self.version = Some(version);
                } else {
                    return Err(
                        PtError::Version("Failed to parse version block".into())
                    )
                }
            }
        }
        Ok(())
    }

    pub fn parse_session(&mut self) -> Result<PtSession, PtError> {
        debug!("Parsing blocks...");
        self.parse_blocks()?;

        debug!("Parsing header...");
        let session_sample_rate = self.parse_header()?;

        debug!("Parsing audio files...");
        let audio_files = self.parse_audio_files()
            .map_err(PtError::Io)?;

        debug!("Parsing audio tracks...");
        let (audio_tracks, audio_regions) = self.parse_audio_tracks(&audio_files)
            .map_err(PtError::Io)?;

        debug!("Parsing markers...");
        let markers = self.parse_markers()
            .map_err(PtError::Io)?;

        let session = PtSession {
            version: self.version.unwrap(),
            session_sample_rate,
            audio_files,
            audio_tracks,
            audio_regions,
            markers,
            ..Default::default()
        };

        Ok(session)
    }

    fn parse_blocks(&mut self) -> Result<(), PtError> {
        let mut block_map = BlockMap::default();
        let mut i = 20;
        let mut count = 0;

        while i < self.unxored().len() {
            match self.parse_block_at(i, None, 0) {
                Ok(block) => {
                    count += 1;
                    i += if block.size > 0 {
                        block.size + 7
                    } else {
                        1
                    };
                    if let Ok(ptcd) = block.content_type.try_into() {
                        use PTCD::*;
                        match ptcd {
                            INFO_SampleRate => block_map.header_blocks.push(block),
                            WAV_List_Full => block_map.wav_blocks.push(block),
                            AUDIO_Region_List_v5 | AUDIO_Region_List_v10  => block_map.region_to_wav_blocks.push(block),
                            AUDIO_Tracks => block_map.track_blocks.push(block),
                            AUDIO_Region_Track_Full_Map | AUDIO_Region_Track_Full_Map_v8 => block_map.region_to_track_blocks.push(block),
                            MARKER_List => block_map.marker_blocks.push(block),
                            _ => {}
                        }
                    }
                }
                Err(e) => {
                    debug!("Parsed {} parent blocks", count);
                    self.block_map = Some(block_map);
                    return Err(PtError::Io(e));
                }
            }
        }

        Ok(self.block_map = Some(block_map))
    }

    fn parse_header(&mut self) -> Result<u64, PtError> {
        let block_map = self.block_map.as_ref().unwrap();

        debug!("Header blocks: {}", block_map.header_blocks.len());
        if block_map.header_blocks.is_empty() {
            return Err(PtError::Parse)
        }

        let offset = block_map.header_blocks[0].offset;
        self.set_position(offset + 4);
        Ok(self.read_u32().map_err(PtError::Io)? as u64)
    }

    fn parse_audio_files(&mut self) -> Result<Vec<Wav>, io::Error> {
        let mut audio_files = vec![];
        let block_map = self.block_map.take();
        let wav_blocks = &block_map.as_ref().unwrap().wav_blocks;

        for wav_list in wav_blocks {
            self.set_position(wav_list.offset + 2);
            let num_waves = self.read_u32()?;
            debug!("Num Wavs: {}", num_waves);

            for child in children_of!(wav_list, PTCD::WAV_Names) {
                self.set_position(child.offset + 11);
                let mut n = 0;

                debug!("Found WAV @pos {} offset {} size {}", self.position(), child.offset, child.size);

                while self.position() < child.offset + child.size && n < num_waves {
                    let wav_name = self.parse_str()?;
                    let wav_type =
                        unsafe { std::str::from_utf8_unchecked(&self.reader.get_ref()[self.position()..(self.position() + 4)]).to_string() };
                    self.increment_position(9);

                    if wav_name.contains(".grp")
                        || wav_name.contains("Audio Files")
                        || wav_name.contains("Fade Files")
                    {
                        continue;
                    }

                    // Cull container types
                    if self.version.unwrap() < 10 {
                        if !(wav_type.contains("WAVE")
                            || wav_type.contains("EVAW")
                            || wav_type.contains("AIFF")
                            || wav_type.contains("FFIA"))
                        {
                            continue;
                        }
                    } else {
                        if wav_type.len() != 0 {
                            if !(wav_type.contains("WAVE")
                                || wav_type.contains("EVAW")
                                || wav_type.contains("AIFF")
                                || wav_type.contains("FFIA"))
                            {
                                continue;
                            }
                        } else if !(wav_name.contains(".wav") || wav_name.contains(".aif")) {
                            continue;
                        }
                    }

                    let wav = Wav {
                        index: n as u16,
                        file_name: wav_name,
                        ..Default::default()
                    };

                    audio_files.push(wav);
                    n += 1;
                }
            }
        }

        let mut wav_iter = audio_files.iter_mut();
        for block in filter_blocks!(wav_blocks.iter(), PTCD::WAV_Metadata => PTCD::WAV_SampleRate_Size) {
            if let Some(wav) = wav_iter.next() {
                self.set_position(block.offset + 8);
                wav.len = self.read_u64()?;
            }
        }

        self.block_map = block_map;
        Ok(audio_files)
    }

    fn parse_audio_tracks(&mut self, audio_files: &[Wav]) -> Result<(Vec<Track>, Vec<Region>), io::Error> {
        const MAX_CHANNELS_PER_TRACK: usize = 8;
        let mut audio_tracks: Vec<Track> = vec![];
        let mut regions = vec![];
        let block_map = self.block_map.take();
        let BlockMap { track_blocks, region_to_track_blocks, region_to_wav_blocks, .. }
            = &block_map.as_ref().unwrap();


        let mut channel_map = [0u16; MAX_CHANNELS_PER_TRACK];
        let mut region_index = 0;

        // Wav source -> Regions
        for block in region_to_wav_blocks {
            for b in children_of!(block, PTCD::AUDIO_Region_Name_Number_v5, PTCD::AUDIO_Region_Name_Number_v10) {
                self.set_position(b.offset + 11);
                let mut region = self.parse_region_info(b.offset + b.size)?;
                if let Some(wav) = audio_files
                    .iter()
                    .find(|wav| region.wav.as_ref().unwrap().index == wav.index)
                {
                    region.wav.as_mut().unwrap().file_name = wav.file_name.clone();
                }
                region.index = region_index;
                regions.push(region);
                region_index += 1;
            }
        }

        // Audio Tracks
        for b in filter_blocks!(track_blocks.iter(), PTCD::AUDIO_Track_Name_Number) {
            self.set_position(b.offset + 2);
            let name = self.parse_str()?;

            self.increment_position(1);
            let num_channels = self.read_u32()? as usize;

            for i in 0..num_channels {
                channel_map[i] = self.read_u16()?;
                if audio_tracks
                    .iter()
                    .find(|&t| t.index == channel_map[i])
                    .is_none()
                {
                    let track = Track {
                        index: channel_map[i],
                        name: name.clone(),
                        ..Default::default()
                    };
                    audio_tracks.push(track);
                }
            }
        }

        // Regions -> Tracks
        for block in region_to_track_blocks {
            match block.content_type.try_into() {
                // Old PT
                Ok(PTCD::AUDIO_Region_Track_Full_Map) => {

                }
                // New PT
                Ok(PTCD::AUDIO_Region_Track_Full_Map_v8) => {
                    let mut count = 0;

                    for a in children_of!(block, 0x1052) {
                        let track_name = self.parse_str_at(a.offset + 2)?;
                        trace!("Mapping regions for track {}", track_name);

                        for b in children_of!(a, 0x1050) {
                            // Check if region is fade
                            if self.unxored()[b.offset + 46] == 0x01 {
                                continue;
                            }

                            for c in children_of!(b, 0x104f) {
                                self.set_position(c.offset + 4);
                                let raw_index = self.read_u32()? as u16;
                                self.increment_position(5);
                                let start = self.read_u32()?;

                                let track_index = count;
                                if let Some(ref mut track) = audio_tracks.iter_mut().find(|t| t.index == track_index) {
                                    if let Some(region) = regions.iter_mut().find(|r| r.index == raw_index) {
                                        // start as f32 * rate_factor
                                        region.start_pos = start as u64;
                                        track.regions.push(region.clone());
                                    }
                                }
                            }
                        }
                        count += 1;
                    }
                }
                _ => {}
            }
        }

        self.block_map = block_map;
        Ok((audio_tracks, regions))
    }

    fn parse_region_info(&mut self, offset: usize) -> Result<Region, io::Error> {
        let name = self.parse_str()?;
        let (sample_offset, start, length) = self.parse_three_point()?;
        self.set_position(offset);
        let index = self.read_u32()? as u16;
        // let pos_absolute = start as f32 * rate_factor;
        // let len = length as f32 * rate_factor

        let wav = Wav {
            index,
            pos_absolute: start,
            len: length,
            ..Default::default()
        };

        let region = Region {
            name,
            start_pos: start as u64,
            sample_offset: sample_offset as u64,
            len: length,
            wav: Some(wav),
            ..Default::default()
        };

        Ok(region)
    }

    fn parse_markers(&mut self) -> Result<Vec<Marker>, io::Error> {
        let block_map = self.block_map.take();
        let marker_blocks = &block_map.as_ref().unwrap().marker_blocks;
        let mut markers = vec![];

        for block in filter_blocks!(marker_blocks.iter(), PTCD::MARKER_List_Full => PTCD::MARKER_List_Entry) {
            trace!("In Marker Entry");
            self.set_position(block.offset + 2);
            let index = self.read_u16()?;
            self.increment_position(4);
            let name = self.parse_str()?;
            let sample_offset = self.read_u32()? as usize;

            // Jump to marker comments
            // NOTE: This method could break very easily!
            // We are in full assumption mode that there are no further 0x01 bytes between the
            // sample_offset and the comments
            let mut pos = self.position();
            let mut i = 0;
            loop {
                if self.read_u8()? == 0x01 {
                    pos += i + 5;
                    break;
                }
                i += 1;
            }

            let comment = self.parse_str_at(pos)?;

            markers.push(Marker {
                index,
                name,
                sample_offset,
                comment,
            });
        }
        
        self.block_map = block_map;
        Ok(markers)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::fs::read_to_string;
    use serde_json as serde;

    #[test]
    fn regions() {
        env_logger::init();
        let session = PtSession::from("tests/RegionTest.ptx");
        assert_eq!(session.version, 12);
        assert_eq!(session.session_sample_rate, 44100);
        assert!(session.audio_files.len() > 0);
        assert_eq!(session.audio_files[0].file_name, "region_name_WAV.wav");
        assert_eq!(format!("{}", session), read_to_string("tests/RegionTestOutput.txt").unwrap());
    }

    #[test]
    fn markers() {
        let session = PtSession::from("tests/MarkerTest.ptx");
        assert_eq!(session.version, 12);
        assert_eq!(session.session_sample_rate, 48000);
        assert_eq!(session.markers, serde::from_str::<Vec<Marker>>(&read_to_string("tests/MarkerTestOutput.json").unwrap()).unwrap())
    }
}
