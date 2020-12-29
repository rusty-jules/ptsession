use serde::{Serialize, Deserialize};

use std::path::Path;
use std::fmt;

#[derive(Default, Debug, PartialEq)]
#[derive(Serialize, Deserialize)]
pub struct PtSession {
    pub session_sample_rate: u64,
    pub version: u8,
    pub num_blocks: usize,
    pub audio_files: Vec<Wav>,
    pub audio_regions: Vec<Region>,
    pub audio_tracks: Vec<Track>,
    pub markers: Vec<Marker>,
}

impl<P: AsRef<Path>> From<P> for PtSession {
    fn from(path: P) -> Self {
        crate::parser::PtSessionParser::decrypt(path)
            .expect("Successful decrypt")
            .parse_session()
            .expect("Successful parse")
    }
}

impl fmt::Display for PtSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "Pro Tools {} Session: Samplerate = {}",
            self.version, self.session_sample_rate
        )?;
        //writeln!(f, "Target samplerate = {}\n", self.target_sample_rate)?;
        writeln!(
            f,
            "{} wavs, {} regions\n",
            self.audio_files.len(),
            self.audio_regions.len()
        )?;

        if !self.audio_files.is_empty() {
            writeln!(f, "Audio file (WAV#) @ offset, length:")?;
            for wav in &self.audio_files {
                writeln!(
                    f,
                    "`{}`, w({}) @ {}, {}",
                    wav.file_name, wav.index, wav.pos_absolute, wav.len
                )?;
            }
            writeln!(f)?;
        }

        if !self.audio_regions.is_empty() {
            writeln!(f, "Region (Region#) (WAV#) @ into-sample, length:")?;
            for r in &self.audio_regions {
                writeln!(
                    f,
                    "`{}`, r({}), w({}), @ {}, {}",
                    r.name,
                    r.index,
                    r.wav.as_ref().unwrap().index,
                    r.sample_offset,
                    r.len
                )?;
            }
            writeln!(f)?;
        }
        
        if !self.audio_tracks.is_empty() {
            writeln!(f, "Track name (Track#) (Region#) @ Absolute:")?;
            for t in &self.audio_tracks {
                if !t.regions.is_empty() {
                    write!(
                        f, 
                        "`{}` t({})",
                        t.name, t.index
                    )?;
                    for region in &t.regions {
                        write!(f, " r({}) @ {}", region.index, region.sample_offset)?;
                    }
                    writeln!(f)?;
                }
            }
            writeln!(f)?;
        }

        Ok(())
    }
}

#[derive(Debug, PartialEq)]
#[derive(Serialize, Deserialize)]
pub struct Block {
    pub z_mark: u8,
    pub block_type: u16,
    pub size: usize,
    pub content_type: u16,
    pub offset: usize,
    pub children: Vec<Block>,
}

#[derive(Default, Debug, Clone, PartialEq)]
#[derive(Serialize, Deserialize)]
pub struct Wav {
    pub file_name: String,
    pub index: u16,
    pub pos_absolute: usize,
    pub len: usize,
}

#[derive(Default, Debug, Clone, PartialEq)]
#[derive(Serialize, Deserialize)]
pub struct Region {
    pub name: String,
    pub index: u16,
    pub start_pos: u64,
    pub sample_offset: u64,
    pub len: usize,
    pub wav: Option<Wav>,
}

#[derive(Default, Debug, Clone, PartialEq)]
#[derive(Serialize, Deserialize)]
pub struct Track {
    pub name: String,
    pub index: u16,
    pub playlist: u8,
    pub regions: Vec<Region>,
}

#[derive(Default, Debug, Clone, PartialEq)]
#[derive(Serialize, Deserialize)]
pub struct Marker {
    pub name: String,
    pub index: u16,
    pub comment: String,
    pub sample_offset: usize,
}

