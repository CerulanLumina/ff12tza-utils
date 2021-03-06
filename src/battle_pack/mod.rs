use std::path::PathBuf;

#[cfg(feature = "battle_fuse")]
mod fuse;

mod io;

use crate::{assert_exists, error_abort};
use crate::utils;
use std::fs::{File, OpenOptions, DirBuilder};
use std::io::{Seek, SeekFrom, Write, Read};
use byteorder::{ReadBytesExt, WriteBytesExt};

use io::BattlePackReader;
use walkdir::WalkDir;
use crate::battle_pack::io::BattlePackWriter;
use std::str::FromStr;

const EQUIPMENT_SIGNATURE: [u8; 3] = [68, 113, 0];
const OFFSET_FROM_SIGNATURE: usize = 8;
const FLYING_FLAG_OFFSET: usize = 7;
const EQUIPMENT_STRUCT_SIZE: usize = 52;

pub fn unpack(battle_pack: PathBuf, output: Option<PathBuf>) {
    assert_exists!(battle_pack, "battle pack");
    let output = output.unwrap_or_else(|| battle_pack.with_extension("unpacked"));

    if let Err(err) = DirBuilder::new().recursive(true).create(output.as_path()) {
        error_abort!(1, "Failed to create output folder. Error: {}", err);
    }

    let bp_file = match File::open(&battle_pack) {
        Ok(file) => file,
        Err(err) => {
            error_abort!(1, "Failed to open battle pack '{:?}' for reading. Error: {}", &battle_pack, err)
        }
    };

    let mut bp_reader = match BattlePackReader::new(bp_file) {
        Ok(reader) => reader,
        Err(err) => {
            error_abort!(2, "Failed to create reader over battle pack. Error: {}", err)
        }
    };

    for i in 0..bp_reader.section_count() {
        let mut output_bin = {
            let out_file_path = output.join(format!("section_{:02}.bin", i));
            let output_path = out_file_path.as_path();
            match File::create(output_path) {
                Ok(file) => file,
                Err(err) => {
                    error_abort!(3, "Failed to create output file '{:?}'. Error: {}", output_path, err);
                }
            }
        };
        let mut buffer = Vec::new();
        // match bp_reader.section_size(i) {
        match bp_reader.section_begin_to_end(i, &mut buffer) {
            Ok(d) => {
                println!("Exporting section {}, {} bytes.", i, d);
                if let Err(err) = output_bin.write_all(&buffer) {
                    error_abort!(4, "Failed to write export for section {}. Error: {}", i, err);
                }
                buffer.clear();
            },
            Err(err) => {
                error_abort!(2, "Failed to read data for section {}. Error: {}", i, err);
            }
        }
    }

}

pub fn repack(input_dir: PathBuf, output: PathBuf) {
    if !input_dir.is_dir() { error_abort!(1, "Input directory is nonexistent or is not a directory."); }
    match File::create(output.as_path()) {
        Ok(file) => {
            let mut all_data = Vec::new();
            let walkdir = WalkDir::new(input_dir.as_path())
                .follow_links(true)
                .contents_first(true)
                .min_depth(1)
                .max_depth(1)
                .contents_first(true);
            let dir = walkdir.into_iter()
                .map(|f| f.unwrap_or_else(|err| error_abort!(1, "Failed to retrieve directory entry. Error: {}", err)))
                .filter(|f| f.file_type().is_file())
                .filter(|a| {
                    let file = a.file_name().to_string_lossy();
                    file.len() == 14 && {
                        let (start, end) = file.split_at(8);
                        start == "section_" && end.ends_with(".bin") && u8::from_str(&end[0..2]).is_ok()
                    }
                })
                .map(|e| e.into_path());
            let mut entries = dir.collect::<Vec<_>>();
            entries.sort_by_key(|a| u8::from_str(&a.as_path().file_name().unwrap().to_string_lossy()[8..10]).unwrap());
            for entry in entries {
                let meta = std::fs::metadata(entry.as_path()).unwrap_or_else(|err| error_abort!(1, "Failed to get input file metadata for {:?}. Error: {}", entry, err));
                let mut data = Vec::with_capacity(meta.len() as usize);
                let mut input = File::open(entry.as_path()).unwrap_or_else(|err| error_abort!(1, "Failed to open input file {:?}. Error: {}", entry, err));
                input.read_to_end(&mut data).unwrap_or_else(|err| error_abort!(1, "Failed to read input file {:?}. Error: {}", entry, err));
                all_data.push(data);
            }
            let mut b_writer = BattlePackWriter::new(all_data.len(), file).unwrap_or_else(|err| error_abort!(2, "Failed to write to output file. Error: {}", err));
            for (i, section) in all_data.into_iter().enumerate() {
                b_writer.write_section(&section).unwrap_or_else(|err| error_abort!(2, "Failed to write section {} to output file. Error: {}", i, err))
            }
        },
        Err(err) => { error_abort!(1, "Failed to create output file. Error: {}", err); }
    }
}

pub fn allow_all_flying(battle_pack: PathBuf) {
    assert_exists!(battle_pack, "battle pack");
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    let mut file = match options.open(&battle_pack) {
        Ok(file) => file,
        Err(err) => {
            eprintln!("Unable to open file: {:?}\nError: {}", &battle_pack, err);
            std::process::exit(-1);
        }
    };
    let equip_array = match utils::locate_signature(&mut file, &EQUIPMENT_SIGNATURE[..]) {
        Some(loc) => loc + OFFSET_FROM_SIGNATURE,
        None => {
            eprintln!("Unable to find the equipment section within the battle pack.");
            std::process::exit(7);
        }
    };
    println!("Located appropriate section.");
    for id in (0usize..=199).map(|a| a * EQUIPMENT_STRUCT_SIZE + equip_array + FLYING_FLAG_OFFSET) {
        file.seek(SeekFrom::Start(id as u64)).expect("Seeking file");
        let byte = file.read_u8().expect("Reading file");
        file.seek(SeekFrom::Start(id as u64)).expect("Seeking file");
        file.write_u8(byte | 0b100).expect("Writing file");
    }

    println!("Made all weapons in battle pack able to hit flying enemies.");

}
