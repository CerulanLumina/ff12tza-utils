use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Cursor, IoSlice, Read, Seek, SeekFrom, Write};
use std::io::Result as IOResult;
use std::path::PathBuf;

use byteorder::{LE, ReadBytesExt};
use serde::Deserialize;
use serde::export::fmt::Arguments;
use walkdir::WalkDir;

use crate::error::TreasureError;

mod plotter;

#[derive(Deserialize, Debug)]
struct TreasureData {
    groups: HashMap<String, HashSet<String>>,
    zones: HashMap<String, ZoneData>,
}

#[derive(Deserialize, Debug)]
struct ItemData {
    ids: HashMap<u16, String>
}

trait FromJsonPath {
    fn open(input: PathBuf) -> Result<Self, TreasureError>
        where Self: Sized;
}

impl FromJsonPath for TreasureData {
    fn open(input: PathBuf) -> Result<TreasureData, TreasureError> {
        Ok(serde_json::from_reader(File::open(input)?)?)
    }
}

impl FromJsonPath for ItemData {
    fn open(input: PathBuf) -> Result<ItemData, TreasureError> {
        Ok(serde_json::from_reader(File::open(input)?)?)
    }
}

#[derive(Deserialize, Debug)]
struct ZoneData {
    name: String,
    offset: u64,
    quantity: u16,
}

#[derive(Copy, Clone, Debug)]
pub struct ZoneTreasure {
    id: u32,
    pos_x: i16,
    pos_y: i16,
    respawn_slot: u8,
    spawn_chance: u8,
    gil_chance: u8,
    first_item: u16,
    second_item: u16,
    rare_first_item: u16,
    rare_second_item: u16,
    gil_amount: u16,
    rare_gil_amount: u16,
}

fn get_data<T: FromJsonPath>(pb: PathBuf, name: &'static str, env_name: &'static str) -> T {
    if !pb.exists() {
        eprintln!("Missing {} data file!", name);
        eprintln!("Use the --{}-data option or the {}_DATA environment variable.", name, env_name);
        std::process::exit(2);
    }
    match T::open(pb) {
        Ok(data) => data,
        Err(err) => {
            eprintln!("Error occurred while reading the {} data file.", name);
            eprintln!("Error: {}", err);
            std::process::exit(3);
        }
    }

}

fn get_datas(treasure_data: PathBuf, item_data: PathBuf) -> (TreasureData, ItemData) {
    let treasure = get_data(treasure_data, "treasure", "TREASURE");
    let item = get_data(item_data, "item", "ITEM");
    (treasure, item)
}
//
// enum OutputData<'a> {
//     Zone(&'a str),
//     Chest(TreasureOutputData<'a>)
// }
//
// struct TreasureOutputData<'a> {
//     name: Option<&'a str>,
//     first_item: &'a str,
//     second_item: &'a str,
//     rare_first_item: &'a str,
//     rare_second_item: &'a str,
//     gil_amount: u16,
//     rare_gil_amount: u16,
//     gil_chance: u16,
//     spawn_chance: u8,
//     respawn_slot: u8,
// }
//
// fn write_output<W: Write>(output: &mut W, data: OutputData) -> std::io::Result<()> {
//     unimplemented!()
// }

enum OutputWriter {
    Stdout(std::io::Stdout),
    File(File)
}

impl Write for OutputWriter {
    fn write(&mut self, buf: &[u8]) -> IOResult<usize> {
        match self { OutputWriter::Stdout(stdout) => stdout.write(buf), OutputWriter::File(file) => file.write(buf) }
    }

    fn write_vectored(&mut self, bufs: &[IoSlice<'_>]) -> IOResult<usize> {
        match self { OutputWriter::Stdout(stdout) => stdout.write_vectored(bufs), OutputWriter::File(file) => file.write_vectored(bufs) }
    }

    fn flush(&mut self) -> IOResult<()> {
        match self { OutputWriter::Stdout(stdout) => stdout.flush(), OutputWriter::File(file) => file.flush() }
    }

    fn write_all(&mut self, buf: &[u8]) -> IOResult<()> {
        match self { OutputWriter::Stdout(stdout) => stdout.write_all(buf), OutputWriter::File(file) => file.write_all(buf) }
    }

    fn write_fmt(&mut self, fmt: Arguments<'_>) -> IOResult<()> {
        match self { OutputWriter::Stdout(stdout) => stdout.write_fmt(fmt), OutputWriter::File(file) => file.write_fmt(fmt) }
    }
}

pub fn dump_treasure(input: PathBuf, output: Option<PathBuf>, treasure_data: PathBuf, item_data: PathBuf, create_maps: bool) {
    assert!(!(output.is_some() ^ create_maps));
    let (treasure_data, item_data) = get_datas(treasure_data, item_data);

    if !input.exists() {
        eprintln!("Non-existent input directory: {:?}", input);
        std::process::exit(4);
    }
    let output = if !output.as_ref().map(|dir| dir.exists()).unwrap_or(true) {
        let dir = output.unwrap();
        println!("Non-existent output directory: {:?}. Creating...", &dir);
        if let Err(err) = std::fs::create_dir(&dir) {
            eprintln!("Unable to create output directory. Error: {}", err);
            std::process::exit(4);
        }
        Some(dir)
    } else { output };

    let iter = WalkDir::new(input)
        .follow_links(true)
        .into_iter()
        .filter_map(|a| a.ok())
        .filter(|a| a.file_type().is_file())
        .filter(|a| a.path().extension().map(|a| a == "ebp").unwrap_or(false))
        .map(|it| if it.path_is_symlink() { std::fs::read_link(it.path()) } else { Ok(it.into_path()) })
        .filter_map(|it| it.ok());

    let mut slot_binds = Vec::with_capacity(255);
    for _ in 0..255 { slot_binds.push(Vec::new()); }

    for path in iter {
        let file_stem = path.file_stem().unwrap().to_str().unwrap().to_owned();

        let group = if let Some(item) = treasure_data.groups.iter().find(|a| a.1.contains(&file_stem)) {
            item.0.as_str()
        } else { "Unknown" };

        if !treasure_data.zones.contains_key(&file_stem) {
            continue;
        }
        let zone = &treasure_data.zones[&file_stem];

        if output.is_some() {
            if let Err(err) = std::fs::DirBuilder::new()
                .recursive(true)
                .create(output.as_ref().unwrap().join(group)) {
                eprintln!("Unable to create file directory. Error: {}", err);
            }
        }

        let writer_path = output.as_ref().map(|dir| dir.join(group).join(&zone.name).with_extension("txt"));
        let mut writer = match writer_path.as_ref() {
            Some(file_path) => {
                match File::create(&file_path) { Ok(file) => OutputWriter::File(file), Err(err) => { eprintln!("Error creating file {:?}. Error: {}", file_path, err); continue; }}
            },
            None => OutputWriter::Stdout(std::io::stdout())
        };

        let write_res = writeln!(writer, "{}", &zone.name)
            .and_then(|_| writeln!(writer, "\t{:3}{:6}{:6}{:6}{:6}{:20}{:20}{:20}{:20}{:5}{:>6}{:>6}", "ID", "Slot", "Spn%", "Gil%", "Gil", "Item 1 (%50%)", "Item 2 (50%)", "DA 1 (95%)", "DA 2 (5%)", "DGil", "X", "Y"))
            .and_then(|_| writeln!(writer, "\t{:=<124}", "="));
        if let Err(e) = write_res { eprintln!("Error writing to file. {}", e); continue; }
        let res = File::open(path.as_path()).map_err(|e| TreasureError::from(e))
            .and_then(|file| read_treasure_files(file, &zone));

        match res {
            Ok(zone_treasures) => {
                if create_maps {
                    let svg_path = writer_path.as_ref().unwrap().with_extension("svg");
                    if let Err(err) = plotter::plot(&svg_path, &zone.name, &zone_treasures) {
                        eprintln!("Failed to create SVG map for {}. Error: {}", &zone.name, err);
                    }
                }
                // plotter::plot()
                // plotter::plot(&zone.name, &zone_treasures).expect("creating chart");
                for treasure in zone_treasures {
                    let first_item = item_data.ids[&treasure.first_item].as_str();
                    let second_item = item_data.ids[&treasure.second_item].as_str();
                    let rare_first_item = item_data.ids[&treasure.rare_first_item].as_str();
                    let rare_second_item = item_data.ids[&treasure.rare_second_item].as_str();
                    if treasure.respawn_slot != 255 {
                        (&mut slot_binds[treasure.respawn_slot as usize]).push((zone.name.clone(), group.to_string(), treasure.id, String::from(first_item)));
                    }
                    if let Err(e) = writeln!(writer, "\t{:<3}{:<6x}{:<6}{:<6}{:<6}{:20}{:20}{:20}{:20}{:5}{:6}{:6}", treasure.id, treasure.respawn_slot, treasure.spawn_chance, treasure.gil_chance, treasure.gil_amount, first_item, second_item, rare_first_item, rare_second_item, treasure.rare_gil_amount, treasure.pos_x, treasure.pos_y) {
                        eprintln!("Error writing to file. {}", e); continue;
                    }
                }
            },
            Err(err) => {
                eprintln!("An error occurred while processing file {:?}. Error: {}", path.as_path(), err);
            }
        }

    }
    let mut slot_out = if let Some(out_dir) = output.as_ref() {
        let slots = out_dir.join("respawn-slots.txt");
        match File::create(slots) {
            Ok(file) => OutputWriter::File(file),
            Err(err) => { eprintln!("Unable to create respawn-slots.txt.\nError: {}", err); return; }
        }
    } else { OutputWriter::Stdout(std::io::stdout()) };

    writeln!(slot_out, "Slot => [(Zone: Area :: ID = Item), (...), ...]").expect("Writing respawn-slots.txt");
    for (i, mut slot) in slot_binds.drain(..).enumerate() {

        write!(slot_out, "{:02x} => [", i).expect("Writing respawn-slots.txt");
        let num_in_slot = slot.len();
        if num_in_slot > 0 {
            for (k, data) in slot.drain(..).enumerate() {
                write!(slot_out, "({}: {} :: {} = {}){}", data.1, data.0, data.2, data.3, if k == num_in_slot - 1 { "" } else { ", " }).expect("Writing respawn-slots.txt");
            }
        }
        writeln!(slot_out, "]").expect("Writing respawn-slots.txt");
    }

}

fn read_treasure_files<R: Read + Seek>(reader: R, data: &ZoneData) -> Result<Vec<ZoneTreasure>, TreasureError> {
    let mut reader = reader;
    reader.seek(SeekFrom::Start(data.offset))?;
    let mut buffer = [0u8; 24];

    let mut treasures = Vec::with_capacity(data.quantity as usize);

    for _ in 0..data.quantity {
        reader.read_exact(&mut buffer)?;

        let mut cursor = Cursor::new(&mut buffer);

        treasures.push(ZoneTreasure {
            id: cursor.read_u32::<LE>()?,
            pos_x: cursor.read_i16::<LE>()?,
            pos_y: cursor.read_i16::<LE>()?,
            respawn_slot: {cursor.read_u8()?; cursor.read_u8()?},
            spawn_chance: cursor.read_u8()?,
            gil_chance: cursor.read_u8()?,
            first_item: cursor.read_u16::<LE>()?,
            second_item: cursor.read_u16::<LE>()?,
            rare_first_item: cursor.read_u16::<LE>()?,
            rare_second_item: cursor.read_u16::<LE>()?,
            gil_amount: cursor.read_u16::<LE>()?,
            rare_gil_amount: cursor.read_u16::<LE>()?,
        });
    }

    Ok(treasures)
}
