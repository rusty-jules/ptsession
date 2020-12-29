// https://stackoverflow.com/questions/28028854/how-do-i-match-enum-values-with-an-integer
// For deriving try_into from the enum definition without num_traits and num_derive crates
macro_rules! back_to_enum {
    ($(#[$meta:meta])* $vis:vis enum $name:ident {
        $($(#[$vmeta:meta])* $vname:ident $(= $val:expr)?,)*
    }) => {
        $(#[$meta])*
        $vis enum $name {
            $($(#[$vmeta])* $vname $(= $val)?,)*
        }

        impl std::convert::TryFrom<u16> for $name {
            type Error = ();

            fn try_from(v: u16) -> Result<Self, Self::Error> {
                match v {
                    $(x if x == $name::$vname as u16 => Ok($name::$vname),)*
                    _ => Err(()),
                }
            }
        }
    }
}

back_to_enum! {
    #[allow(non_camel_case_types)]
    #[repr(u16)]
    #[derive(Debug)]
    pub enum PTCD {
        DUMMY = 0x0000,
        INFO_Version = 0x0003,
        INFO_Product_and_Version = 0x0030,
        WAV_SampleRate_Size = 0x1001,
        WAV_Metadata = 0x1003,
        WAV_List_Full = 0x1004,
        REGION_Name_Number = 0x1007,
        AUDIO_Region_Name_Number_v5 = 0x1008,
        AUDIO_Region_List_v5 = 0x100b,
        AUDIO_Region_Track_Entry = 0x100f,
        AUDIO_Region_Track_Map_Entries = 0x1011,
        AUDIO_Region_Track_Full_Map = 0x1012,
        AUDIO_Track_Name_Number = 0x1014,
        AUDIO_Tracks = 0x1015,
        PLUGIN_Entry = 0x1017,
        PLUGIN_Full_List = 0x1018,
        IO_Channel_Entry = 0x1021,
        IO_Channel_List = 0x1022,
        INFO_SampleRate = 0x1028,
        WAV_Names = 0x103a,
        AUDIO_Region_Track_SubEntry_v8 = 0x104f,
        AUDIO_Region_Track_Entry_v8 = 0x1050,
        AUDIO_Region_Track_Full_Map_v8 = 0x1054,
        MIDI_Region_Track_Entry = 0x1056,
        MIDI_Region_Track_Map_Entries = 0x1057,
        MIDI_Region_Track_Full_Map = 0x1058,
        MIDI_Events_Block = 0x2000,
        MIDI_Region_Name_Number_v5 = 0x2001,
        MIDI_Regions_Map = 0x2002,
        INFO_Path_of_Session = 0x2067,
        Snaps_Block = 0x2511,
        MIDI_Track_Full_List = 0x2519,
        MIDI_Track_Name_Number = 0x251a,
        COMPOUND_Region_element = 0x2523,
        IO_Route = 0x2602,
        IO_Routing_Table = 0x2603,
        COMPOUND_Region_Group = 0x2628,
        AUDIO_Region_Name_Number_v10 = 0x2629,
        AUDIO_Region_List_v10 = 0x262a,
        COMPOUND_Region_Full_Map = 0x262c,
        MIDI_Region_Name_Number_v10 = 0x2633,
        MIDI_Regions_Map_v10 = 0x2634,
        MARKER_List = 0x271a,
        MARKER_Metadata = 0x2619,
        MARKER_List_Full = 0x2030,
        MARKER_List_Entry = 0x2077,
    }
}
