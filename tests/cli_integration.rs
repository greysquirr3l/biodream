//! Integration tests for the `biopac` CLI binary (T12).
//!
//! Tests spawn the compiled binary as a subprocess and assert on its exit code
//! and stdout.  The binary path is injected by Cargo via `CARGO_BIN_EXE_biopac`.

use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};

// ---------------------------------------------------------------------------
// ACQ file builder (same format as api_integration.rs)
// ---------------------------------------------------------------------------

/// Build a minimal Pre-4 uncompressed `.acq` blob with `channels` channels
/// and `samples_per_channel` i16 samples each.
fn build_pre4_acq(channels: usize, samples_per_channel: usize) -> Vec<u8> {
    let n = samples_per_channel;
    let chan_hdr_len: i32 = 252;
    let chan_hdr_usize: usize = usize::try_from(chan_hdr_len).unwrap_or(252);
    let mut buf: Vec<u8> = Vec::new();

    // Graph header (256 bytes) — BIOPAC format layout
    buf.extend_from_slice(&0i16.to_le_bytes()); // [0..2]  unused i16 = 0
    buf.extend_from_slice(&38i32.to_le_bytes()); // [2..6]  lVersion = 38
    buf.extend_from_slice(&256i32.to_le_bytes()); // [6..10] lExtItemHeaderLen = 256
    buf.extend_from_slice(&i16::try_from(channels).unwrap_or(0).to_le_bytes()); // [10..12] nChannels
    buf.extend_from_slice(&[0u8; 4]); // [12..16] horiz/curr = 0
    buf.extend_from_slice(&1.0f64.to_le_bytes()); // [16..24] dSampleTime = 1 ms
    buf.extend(std::iter::repeat_n(0u8, 228)); // [24..252] zeros
    buf.extend_from_slice(&i16::try_from(chan_hdr_len).unwrap_or(252).to_le_bytes()); // [252..254]
    buf.extend_from_slice(&[0u8; 2]); // [254..256] pad
    assert_eq!(buf.len(), 256);

    // Per-channel headers (252 bytes each, V_20a layout at correct byte offsets)
    for ch in 0..channels {
        let start = buf.len();
        let mut ch_buf = [0u8; 252];

        // offset 0: lChanHeaderLen = 252
        ch_buf[0..4].copy_from_slice(&chan_hdr_len.to_le_bytes());
        // offset 6: szCommentText (channel name)
        let name = format!("CH{ch}");
        let name_src = name.as_bytes();
        let len = name_src.len().min(39);
        if let (Some(dst), Some(src)) = (ch_buf.get_mut(6..6 + len), name_src.get(..len)) {
            dst.copy_from_slice(src);
        }
        // offset 68: szUnitsText (2 bytes "mV", rest zero)
        if let Some(dst) = ch_buf.get_mut(68..70) {
            dst.copy_from_slice(b"mV");
        }
        // offset 88: lBufLength = n
        ch_buf[88..92].copy_from_slice(&i32::try_from(n).unwrap_or(0).to_le_bytes());
        // offset 92: dAmplScale = 1.0
        ch_buf[92..100].copy_from_slice(&1.0f64.to_le_bytes());
        // offset 100: dAmplOffset = 0.0
        ch_buf[100..108].copy_from_slice(&0.0f64.to_le_bytes());
        // nVarSampleDivider: at offset 250 for Pre-4 V_30r (rev >= 44).
        // lVersion = 38 < 44, so biodream defaults divider to 1.
        ch_buf[250..252].copy_from_slice(&1i16.to_le_bytes());

        buf.extend_from_slice(&ch_buf);

        assert_eq!(
            buf.len() - start,
            chan_hdr_usize,
            "channel header must be exactly {chan_hdr_usize} bytes"
        );
    }

    // Foreign data section
    buf.extend_from_slice(&0i32.to_le_bytes());

    // Dtype headers (i16)
    for _ in 0..channels {
        buf.extend_from_slice(&4u16.to_le_bytes());
        buf.extend_from_slice(&2u16.to_le_bytes());
    }

    // Interleaved data
    for s in 0..n {
        for ch in 0..channels {
            let sample = i16::try_from(ch * 10 + s).unwrap_or(0).to_le_bytes();
            buf.extend_from_slice(&sample);
        }
    }

    // Marker section
    buf.extend_from_slice(&8i32.to_le_bytes());
    buf.extend_from_slice(&0i32.to_le_bytes());

    // Journal
    buf.extend_from_slice(&0i32.to_le_bytes());

    buf
}

/// Write bytes to a temp file and return the path.
fn write_tmp(bytes: &[u8], name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = std::env::temp_dir().join(name);
    let mut f = std::fs::File::create(&path)?;
    f.write_all(bytes)?;
    Ok(path)
}

const fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_biopac")
}

fn acq(name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    write_tmp(&build_pre4_acq(2, 10), name)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn cli_info_summary() -> Result<(), Box<dyn std::error::Error>> {
    let path = acq("cli_info_summary.acq")?;
    let out = Command::new(bin())
        .args(["info", path.to_str().unwrap_or("")])
        .output()?;
    assert!(out.status.success(), "exit code {:?}", out.status.code());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("AcqKnowledge"),
        "expected 'AcqKnowledge' in:\n{stdout}"
    );
    Ok(())
}

#[test]
fn cli_info_json() -> Result<(), Box<dyn std::error::Error>> {
    let path = acq("cli_info_json.acq")?;
    let out = Command::new(bin())
        .args(["info", "--json", path.to_str().unwrap_or("")])
        .output()?;
    assert!(out.status.success(), "exit code {:?}", out.status.code());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout)?;
    assert!(
        v.get("revision").is_some_and(serde_json::Value::is_number),
        "expected 'revision' field in JSON"
    );
    assert_eq!(v.get("channel_count"), Some(&serde_json::json!(2)));
    Ok(())
}

#[test]
fn cli_info_stdin() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = build_pre4_acq(1, 5);
    let mut child = Command::new(bin())
        .args(["info", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(&bytes)?;
    }

    let out = child.wait_with_output()?;
    assert!(out.status.success(), "exit code {:?}", out.status.code());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("AcqKnowledge"),
        "expected 'AcqKnowledge' from stdin in:\n{stdout}"
    );
    Ok(())
}

#[test]
fn cli_convert_csv() -> Result<(), Box<dyn std::error::Error>> {
    let path = acq("cli_convert_csv.acq")?;
    let out_path = std::env::temp_dir().join("cli_convert_csv_out.csv");
    let out = Command::new(bin())
        .args([
            "convert",
            "--format",
            "csv",
            path.to_str().unwrap_or(""),
            "-o",
            out_path.to_str().unwrap_or(""),
        ])
        .output()?;
    assert!(
        out.status.success(),
        "exit code {:?}\nstderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out_path.exists(),
        "expected CSV output at {}",
        out_path.display()
    );
    let contents = std::fs::read_to_string(&out_path)?;
    // CSV should have a header row and at least one data row.
    assert!(contents.contains('\n'), "expected multiple lines in CSV");
    Ok(())
}

#[test]
fn cli_markers_table() -> Result<(), Box<dyn std::error::Error>> {
    let path = acq("cli_markers_table.acq")?;
    let out = Command::new(bin())
        .args(["markers", path.to_str().unwrap_or("")])
        .output()?;
    assert!(out.status.success(), "exit code {:?}", out.status.code());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Header row must be present even when there are no markers.
    assert!(
        stdout.starts_with("sample\tlabel"),
        "expected table header in:\n{stdout}"
    );
    Ok(())
}

#[test]
fn cli_markers_json() -> Result<(), Box<dyn std::error::Error>> {
    let path = acq("cli_markers_json.acq")?;
    let out = Command::new(bin())
        .args(["markers", "--json", path.to_str().unwrap_or("")])
        .output()?;
    assert!(out.status.success(), "exit code {:?}", out.status.code());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout)?;
    assert!(v.is_array(), "expected JSON array in:\n{stdout}");
    Ok(())
}

#[test]
fn cli_inspect_hex() -> Result<(), Box<dyn std::error::Error>> {
    let path = acq("cli_inspect_hex.acq")?;
    let out = Command::new(bin())
        .args(["inspect", "--hex", path.to_str().unwrap_or("")])
        .output()?;
    assert!(out.status.success(), "exit code {:?}", out.status.code());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("0x"),
        "expected hex offset (0x...) in:\n{stdout}"
    );
    Ok(())
}

#[test]
fn cli_inspect_shows_channel_info() -> Result<(), Box<dyn std::error::Error>> {
    let path = acq("cli_inspect_channels.acq")?;
    let out = Command::new(bin())
        .args(["inspect", path.to_str().unwrap_or("")])
        .output()?;
    assert!(out.status.success(), "exit code {:?}", out.status.code());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("CH0"), "expected CH0 in:\n{stdout}");
    assert!(stdout.contains("I16"), "expected I16 dtype in:\n{stdout}");
    Ok(())
}

#[test]
fn cli_invalid_subcommand_exits_2() -> Result<(), Box<dyn std::error::Error>> {
    let out = Command::new(bin()).arg("bogus").output()?;
    assert_eq!(
        out.status.code(),
        Some(2),
        "expected exit code 2 for unknown subcommand"
    );
    Ok(())
}
