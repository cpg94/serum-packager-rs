use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::Command;

const MAGIC: &[u8] = b"XferJson\x00";

#[derive(Serialize, Deserialize)]
struct PresetJson {
    metadata: serde_json::Value,
    data: serde_json::Value,
}

fn unpack(src: &PathBuf, dst: &PathBuf) -> Result<(), String> {
    let mut file = fs::File::open(src).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).map_err(|e| e.to_string())?;

    if !buf.starts_with(MAGIC) {
        return Err("Invalid file format: missing XferJson magic".to_string());
    }

    let mut off = MAGIC.len();
    let jlen = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]) as usize;
    off += 8;

    let meta: serde_json::Value =
        serde_json::from_slice(&buf[off..off + jlen]).map_err(|e| e.to_string())?;
    off += jlen;

    let clen = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]) as usize;
    off += 8;

    let decompressed = zstd::decode_all(&buf[off..]).map_err(|e| e.to_string())?;

    if decompressed.len() != clen {
        return Err(format!(
            "Decompressed size mismatch: expected {}, got {}",
            clen,
            decompressed.len()
        ));
    }

    let cbor_value: ciborium::Value =
        ciborium::from_reader(decompressed.as_slice()).map_err(|e| e.to_string())?;

    let data: serde_json::Value = serde_json::to_value(cbor_value).map_err(|e| e.to_string())?;

    let output = PresetJson {
        metadata: meta,
        data,
    };
    let json_str = serde_json::to_string_pretty(&output).map_err(|e| e.to_string())?;
    fs::write(dst, json_str).map_err(|e| e.to_string())?;

    Ok(())
}

fn pack(src: &PathBuf, dst: &PathBuf) -> Result<(), String> {
    let json_str = fs::read_to_string(src).map_err(|e| e.to_string())?;
    let input: PresetJson = serde_json::from_str(&json_str).map_err(|e| e.to_string())?;

    let meta_bytes = serde_json::to_string(&input.metadata)
        .map_err(|e| e.to_string())?
        .as_bytes()
        .to_vec();

    let mut cbor_value: ciborium::Value =
        serde_json::from_value(input.data).map_err(|e| e.to_string())?;
    let mut cbor_bytes = Vec::new();
    ciborium::into_writer(&mut cbor_value, &mut cbor_bytes).map_err(|e| e.to_string())?;

    let compressed = zstd::encode_all(cbor_bytes.as_slice(), 3).map_err(|e| e.to_string())?;

    let mut output = Vec::new();
    output.extend_from_slice(MAGIC);
    output.extend_from_slice(&(meta_bytes.len() as u32).to_le_bytes());
    output.extend_from_slice(&0u32.to_le_bytes());
    output.extend_from_slice(&meta_bytes);
    output.extend_from_slice(&(cbor_bytes.len() as u32).to_le_bytes());
    output.extend_from_slice(&2u32.to_le_bytes());
    output.extend_from_slice(&compressed);

    fs::write(dst, output).map_err(|e| e.to_string())?;

    Ok(())
}

fn edit(preset: &PathBuf) -> Result<(), String> {
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());

    let default_parent = PathBuf::from(".");
    let tmp_path = preset
        .file_stem()
        .map(|s| {
            let parent = preset.parent().unwrap_or(&default_parent);
            parent.join(format!("{}.tmp.json", s.to_string_lossy()))
        })
        .unwrap_or_else(|| PathBuf::from("preset.tmp.json"));

    unpack(preset, &tmp_path)?;

    Command::new(&editor)
        .arg(&tmp_path)
        .status()
        .map_err(|e| format!("Failed to open editor: {}", e))?;

    pack(&tmp_path, preset)?;

    fs::remove_file(&tmp_path).ok();

    Ok(())
}

#[derive(Parser)]
#[command(name = "serum-packager")]
#[command(about = "CLI tool for converting Serum 2 preset files to JSON and vice versa")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Unpack { input: PathBuf, output: PathBuf },
    Pack { input: PathBuf, output: PathBuf },
    Edit { file: PathBuf },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Unpack { input, output } => unpack(&input, &output),
        Commands::Pack { input, output } => pack(&input, &output),
        Commands::Edit { file } => edit(&file),
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
