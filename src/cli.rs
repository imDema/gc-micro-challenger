use flate2::read::GzDecoder;
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader, Read},
    path::Path,
};

use crate::{input_key, output_key};
use crate::{set_meta, types::*};

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

    // let mut input_idx = 0usize;
    // let mut output_idx = 0usize;
    let mut input_seq = Vec::new();
    let mut output_seq: HashMap<u8, Vec<_>> = HashMap::new();

    for e in arc.entries()? {
        let mut e = e?;
        let path = e.path()?.to_path_buf();
        let parent = path.parent().expect("wrong format");
        match parent.to_string_lossy().as_ref() {
            "input" => {
                let mut content = Vec::new();
                e.read_to_end(&mut content)?;
                let des = rmp_serde::from_slice::<Batch>(&content)?;
                input_seq.push(des.batch_id);
                let seq = des.batch_id;
                input_tree.insert(input_key(des.batch_id), content)?;
                // input_idx += 1;
                tracing::debug!("loaded input {:03}: {}", seq, path.display());
            }
            "output" => {
                let mut content = Vec::new();
                e.read_to_end(&mut content)?;
                let des = rmp_serde::from_slice::<QueryResult>(&content)?;
                let (q, seq) = (des.query, des.batch_id);
                output_seq.entry(q).or_default().push(seq);
                output_tree.insert(output_key(des.query, des.batch_id), content)?;
                // output_idx += 1;
                tracing::debug!("loaded output {}.{}: {}", q, seq, path.display());
            }
            _ => {
                eprintln!("invalid path for {:?}", path);
                continue;
            }
        }
    }

    let input_len = input_seq.len();
    input_seq.sort_unstable();
    output_seq.values_mut().for_each(|s| s.sort_unstable());

    assert!(
        input_seq.into_iter().zip(0..).all(|(a, b)| a == b),
        "input sequence is missing intermediate values!"
    );

    for (q, seq) in output_seq {
        if seq.len() != input_len {
            tracing::error!("sequence len mismatch between input and output query {q}!");
        }
        if let Some(bad) = seq.into_iter().zip(0..).find(|(a, b)| a != b) {
            tracing::error!(
                "output sequence {q} is missing intermediate values! got {} at pos {}",
                bad.1,
                bad.0
            );
        }
    }

    set_meta(&db, "count", input_len as u32)?;
    tracing::info!("correctly loaded {input_len} batches");

    Ok(())
}
