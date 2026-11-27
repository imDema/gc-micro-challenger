//! # New format
//!
//! ## Input
//! event_time,request_id,request_type,params,tx_id,spends,creates
//!
//! request_id is empty for input-type events
//!
//! ## Output
//!
//! event_time,request_id,request_type,result

use bstr::ByteSlice;
use eyre::Context;
use flate2::read::GzDecoder;
use std::{
    fs::File,
    io::{BufRead, BufReader, Read},
    path::Path,
};

use crate::{COL_EVENT_ID, REQ_MAPPING_KEY, get_meta, input_key, set_meta};

/// Load input/output events from CSV files.
///
/// Groups consecutive type-0 (transaction) events into batches of at most
/// `max_batch_size`.  Type-1 (query) events always start and end their own
/// batch, even if it means a batch of size 1.
pub fn load_csv(
    sled: &Path,
    input_csv: &Path,
    output_csv: &Path,
    max_batch_size: u32,
) -> eyre::Result<()> {
    let db = sled::open(sled)?;
    let input_tree = db.open_tree(super::INPUT)?;
    let output_tree = db.open_tree(super::OUTPUT)?;

    // --- load input ---
    let input_file = BufReader::new(File::open(input_csv)?);
    let input_lines = input_file.lines().skip_while(|l| {
        l.as_ref()
            .is_ok_and(|l| l.starts_with("event_time,request_id,"))
    });

    let mut idx: u32 = 0;
    let mut batch = Vec::new();
    for l in input_lines {
        let l = l?;
        let is_query = l
            .split(',')
            .nth(COL_EVENT_ID)
            .and_then(|p| p.parse::<usize>().ok())
            .is_some();
        batch.push(l.clone());

        let flush = is_query || batch.len() as u32 >= max_batch_size;
        if flush {
            // for line in batch.drain(..) {
            input_tree.insert(input_key(idx), batch.join("\n").as_bytes())?;
            batch.clear();
            tracing::debug!("loaded input event {}", idx);
            idx += 1;
            // }
        }
    }

    input_tree.insert(input_key(idx), batch.join("\n").as_bytes())?;
    batch.clear();
    tracing::debug!("loaded input event {}", idx);
    idx += 1;

    // flush any remaining
    // for line in batch.drain(..) {
    //     input_tree.insert(input_key(idx), line.as_bytes())?;
    //     tracing::debug!("loaded input event {}: {}", idx, line);
    //     idx += 1;
    // }

    let input_len = idx;

    // --- load output ---
    let output_file = BufReader::new(File::open(output_csv)?);
    let mut output_reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_reader(output_file);
    let hdr = output_reader.headers()?;
    let req_col = hdr
        .into_iter()
        .enumerate()
        .find(|&(_, h)| h == "request_id")
        .map(|(i, _)| i)
        .expect("missing `request_id` header");

    for result in output_reader.records() {
        let record = result?;

        let req_id_str = record
            .get(req_col)
            .expect("missing request_id column on row");
        let req_id: u32 = req_id_str.parse().expect("request_id must be an integer");

        let mut output_line = Vec::<u8>::new();
        {
            let mut writer = csv::WriterBuilder::new().from_writer(&mut output_line);
            record
                .iter()
                .map(|f| writer.write_field(f))
                .collect::<Result<(), _>>()?;
        }

        output_tree.insert(req_id.to_be_bytes(), &output_line[..])?;
        tracing::debug!("loaded output event {}: {}", req_id, output_line.as_bstr());
    }

    crate::set_meta(&db, "count", input_len)?;
    tracing::info!("correctly loaded {input_len} input events from CSV");

    Ok(())
}

pub fn load_request_mapping(sled: &Path, json: &Path) -> eyre::Result<()> {
    let db = sled::open(sled)?;
    let mut input_file = BufReader::new(File::open(json)?);
    let map: std::collections::HashMap<String, String> =
        serde_json::from_reader(&mut input_file)
            .wrap_err("Invalid format for request mapping, expected string->string map.")?;

    set_meta(&db, REQ_MAPPING_KEY, &map)?;

    let mappings = get_meta::<std::collections::HashMap<String, String>>(&db, REQ_MAPPING_KEY);

    tracing::info!("loaded mappings: {mappings:?}");

    Ok(())
}

pub fn load_data(sled: &Path, data: &Path) -> eyre::Result<()> {
    let f = BufReader::new(File::open(data)?);

    let db = sled::open(sled)?;
    let tree = db.open_tree(super::INPUT)?;

    for (index, l) in f.lines().enumerate() {
        let l = l?;
        tree.insert(index.to_be_bytes(), l.as_bytes())?;
    }
    Ok(())
}

pub fn load_tar(sled: &Path, path: &Path) -> eyre::Result<()> {
    let file = GzDecoder::new(BufReader::new(File::open(path)?));
    let mut arc = tar::Archive::new(file);

    let db = sled::open(sled)?;
    let input_tree = db.open_tree(super::INPUT)?;
    let output_tree = db.open_tree(super::OUTPUT)?;

    let mut input_seq = Vec::new();
    let mut output_seq = Vec::new();

    for e in arc.entries()? {
        let mut e = e?;
        let path = e.path()?.to_path_buf();
        let parent = path.parent().expect("wrong format");
        match parent.to_string_lossy().as_ref() {
            "input" => {
                let mut content = Vec::new();
                e.read_to_end(&mut content)?;
                let line = String::from_utf8_lossy(&content).to_string();
                // Extract event type from the first CSV field (0 or 1).
                let event_type = line
                    .find(',')
                    .map(|p| line[..p].trim())
                    .and_then(|f| f.parse::<u8>().ok())
                    .unwrap_or(0);
                let idx = input_seq.len() as u32;
                input_seq.push(idx);
                input_tree.insert(input_key(idx), line.as_bytes())?;
                tracing::debug!("loaded input event {}: type={}", idx, event_type);
            }
            "output" => {
                let mut content = Vec::new();
                e.read_to_end(&mut content)?;
                let line = String::from_utf8_lossy(&content).to_string();
                let idx = output_seq.len() as u32;
                output_seq.push(idx);
                output_tree.insert(idx.to_be_bytes(), line.as_bytes())?;
                tracing::debug!("loaded output event {}", idx);
            }
            _ => {
                eprintln!("invalid path for {:?}", path);
                continue;
            }
        }
    }

    let input_len = input_seq.len();
    crate::set_meta(&db, "count", input_len as u32)?;
    tracing::info!("correctly loaded {input_len} events");

    Ok(())
}
