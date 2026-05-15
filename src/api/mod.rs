//! Ergonomic public API surface.
//!
//! This module provides two complementary entry points:
//!
//! 1. **[`ReadOptions`]** — a builder for filtering and scaling channel data
//!    at read time.  Call [`ReadOptions::read_file`], [`ReadOptions::read_bytes`],
//!    or [`ReadOptions::read_stream`] to obtain a fully-loaded [`Datafile`].
//!
//! 2. **[`LazyDatafile`]** + **[`open_file`]** — open a file and parse its
//!    headers, markers, and journal immediately, but defer loading sample data
//!    until the first call to [`LazyDatafile::load_channel`] or
//!    [`LazyDatafile::load_all`].  Useful when only a subset of channels is
//!    needed from a large recording.
//!
//! Both entry points are available only when the `read` feature is enabled.

extern crate alloc;

#[cfg(feature = "read")]
use std::{
    fs::File,
    io::{BufReader, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::OnceLock,
    vec::Vec,
};

#[cfg(feature = "read")]
use crate::{
    domain::{Channel, ChannelData, ChannelMetadata, Datafile, GraphMetadata, Journal, Marker},
    error::{BiopacError, ParseResult, Warning},
    parser::{
        headers::parse_headers,
        markers::parse_markers_and_journal,
        reader::read_stream,
    },
};

// ---------------------------------------------------------------------------
// ReadOptions
// ---------------------------------------------------------------------------

/// Options for controlling how a `.acq` file is loaded.
///
/// Build via [`ReadOptions::new`] (or [`ReadOptions::default`]), chain option
/// methods, then call one of the reading methods.
///
/// # Example
///
/// ```rust,ignore
/// use biodream::ReadOptions;
///
/// let df = ReadOptions::new()
///     .channels(&[0, 2])
///     .scaled(true)
///     .read_file("recording.acq")?
///     .into_value();
/// ```
#[derive(Debug, Clone, Default)]
pub struct ReadOptions {
    /// Which channel indices to retain (`None` = all).
    channel_indices: Option<alloc::vec::Vec<usize>>,
    /// Convert `Scaled` samples to `Float` after loading.
    scaled: bool,
}

impl ReadOptions {
    /// Create a new `ReadOptions` with defaults (all channels, no scaling).
    pub const fn new() -> Self {
        Self {
            channel_indices: None,
            scaled: false,
        }
    }

    /// Restrict which channels are returned, by zero-based file-order index.
    ///
    /// Channels not listed are silently dropped from the result.  Requesting
    /// an out-of-range index is silently ignored.
    #[must_use]
    pub fn channels(mut self, indices: &[usize]) -> Self {
        self.channel_indices = Some(indices.to_vec());
        self
    }

    /// When `true`, convert all [`ChannelData::Scaled`] variants to
    /// [`ChannelData::Float`] by applying `raw * scale + offset`.
    #[must_use]
    pub const fn scaled(mut self, scaled: bool) -> Self {
        self.scaled = scaled;
        self
    }

    /// Finalise the options (no-op; provided for builder API symmetry).
    #[must_use]
    pub const fn build(self) -> Self {
        self
    }

    // --- reading methods ---------------------------------------------------

    /// Read a `.acq` file from the filesystem.
    ///
    /// Both compressed and uncompressed files are handled transparently.
    #[cfg(feature = "read")]
    pub fn read_file(self, path: impl AsRef<Path>) -> Result<ParseResult<Datafile>, BiopacError> {
        let result = crate::parser::reader::read_file(path)?;
        Ok(self.apply(result))
    }

    /// Parse a `.acq` file from an in-memory byte slice.
    ///
    /// Useful in WASM and embedded contexts where filesystem access is
    /// unavailable.
    #[cfg(feature = "read")]
    pub fn read_bytes(self, bytes: &[u8]) -> Result<ParseResult<Datafile>, BiopacError> {
        let result = read_stream(std::io::Cursor::new(bytes))?;
        Ok(self.apply(result))
    }

    /// Read a `.acq` file from any `Read + Seek` source.
    #[cfg(feature = "read")]
    pub fn read_stream<R: std::io::Read + std::io::Seek>(
        self,
        reader: R,
    ) -> Result<ParseResult<Datafile>, BiopacError> {
        let result = read_stream(reader)?;
        Ok(self.apply(result))
    }

    // --- internal ----------------------------------------------------------

    /// Apply channel filter and optional scaling to a parsed result.
    fn apply(self, mut result: ParseResult<Datafile>) -> ParseResult<Datafile> {
        if let Some(ref keep) = self.channel_indices {
            let all = core::mem::take(&mut result.value.channels);
            result.value.channels = keep.iter().filter_map(|&i| all.get(i).cloned()).collect();
        }
        if self.scaled {
            for ch in &mut result.value.channels {
                if matches!(&ch.data, ChannelData::Scaled { .. }) {
                    let floats = ch.scaled_samples();
                    ch.data = ChannelData::Float(floats);
                }
            }
        }
        result
    }
}

// ---------------------------------------------------------------------------
// LazyDatafile
// ---------------------------------------------------------------------------

/// A `.acq` recording where channel sample data is loaded on demand.
///
/// Headers, metadata, markers, and the journal are parsed immediately when the
/// file is opened via [`open_file`].  Channel samples are read from disk only
/// on the first call to [`load_channel`](LazyDatafile::load_channel) or
/// [`load_all`](LazyDatafile::load_all).
///
/// On first load, **all** channels are read together and cached; subsequent
/// `load_channel` calls are zero-cost.
///
/// # Thread safety
///
/// `LazyDatafile` is `Send + Sync`.  Concurrent calls to `load_channel`
/// are safe; at most one file-read is guaranteed to produce the stored result
/// (a second concurrent read may occur but its result is discarded).
#[cfg(feature = "read")]
pub struct LazyDatafile {
    /// Graph-level metadata (revision, sample rate, byte order, …).
    pub metadata: GraphMetadata,
    /// Per-channel metadata (name, units, scale) without sample data.
    pub channel_metadata: Vec<ChannelMetadata>,
    /// All event markers parsed from the file.
    pub markers: Vec<Marker>,
    /// Optional journal text.
    pub journal: Option<Journal>,
    /// Non-fatal warnings from header/marker parsing.
    pub warnings: Vec<Warning>,
    /// Path used to re-open the file for lazy data loading.
    path: PathBuf,
    /// Lazily populated channel data (all channels loaded together on first
    /// access).  `OnceLock::get_or_try_init` is not yet stabilised on the
    /// MSRV; we use a manual fast/slow path instead.
    data_loaded: OnceLock<Vec<Channel>>,
}

#[cfg(feature = "read")]
impl LazyDatafile {
    /// Number of channels described in the file headers.
    pub const fn channel_count(&self) -> usize {
        self.channel_metadata.len()
    }

    /// Returns `true` if sample data has already been loaded into memory.
    ///
    /// Useful in tests to verify that [`open_file`] does not read sample data.
    pub fn is_data_loaded(&self) -> bool {
        self.data_loaded.get().is_some()
    }

    /// Return a reference to a single channel, loading all channel data on
    /// first call.
    ///
    /// Subsequent calls for any channel index return immediately from cache.
    pub fn load_channel(&self, index: usize) -> Result<&Channel, BiopacError> {
        let channels = self.ensure_loaded()?;
        channels.get(index).ok_or_else(|| {
            BiopacError::InvalidChannel(alloc::format!(
                "index {index} out of bounds (file has {} channels)",
                channels.len()
            ))
        })
    }

    /// Load all channels, returning a reference to the full channel slice.
    pub fn load_all(&self) -> Result<&[Channel], BiopacError> {
        self.ensure_loaded()
    }

    /// Consume the `LazyDatafile` and produce a fully-loaded [`Datafile`].
    ///
    /// If sample data has already been loaded this is allocation-free.
    /// Otherwise the file is opened and read now.
    pub fn into_datafile(self) -> Result<Datafile, BiopacError> {
        let channels = if let Some(ch) = self.data_loaded.into_inner() {
            ch
        } else {
            let file = File::open(&self.path)?;
            let reader = BufReader::new(file);
            read_stream(reader)?.value.channels
        };
        Ok(Datafile {
            metadata: self.metadata,
            channels,
            markers: self.markers,
            journal: self.journal,
        })
    }

    /// Ensure sample data is loaded; return a reference to all channels.
    ///
    /// Manual implementation of `OnceLock::get_or_try_init` (unstable on
    /// MSRV).  In a concurrent scenario, two threads may each load the file
    /// and call `set`; the second `set` loses and its data is dropped, but
    /// correctness is preserved — both threads end up with valid data.
    fn ensure_loaded(&self) -> Result<&[Channel], BiopacError> {
        // Fast path: already populated.
        if let Some(ch) = self.data_loaded.get() {
            return Ok(ch);
        }
        // Slow path: read from disk.
        let file = File::open(&self.path)?;
        let reader = BufReader::new(file);
        let channels = read_stream(reader)?.value.channels;
        // `set` returns Err if another thread already populated the cell;
        // discard the duplicate.
        let _ = self.data_loaded.set(channels);
        // `get` is guaranteed Some here.
        self.data_loaded.get().map(Vec::as_slice).ok_or_else(|| {
            BiopacError::Validation(alloc::string::String::from("OnceLock invariant violated"))
        })
    }
}

#[cfg(feature = "read")]
impl core::fmt::Debug for LazyDatafile {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LazyDatafile")
            .field("metadata", &self.metadata)
            .field("channel_count", &self.channel_metadata.len())
            .field("marker_count", &self.markers.len())
            .field("journal", &self.journal)
            .field("warnings", &self.warnings)
            .field("path", &self.path)
            .field("data_loaded", &self.data_loaded.get().is_some())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// open_file
// ---------------------------------------------------------------------------

/// Open a `.acq` file for lazy channel access.
///
/// Headers, markers, and the journal are parsed immediately.  Channel sample
/// data is deferred until the first call to
/// [`LazyDatafile::load_channel`] or [`LazyDatafile::load_all`].
///
/// For files where all channel data is needed, [`crate::read_file`] is more
/// efficient because it avoids the second file-open during data load.
#[cfg(feature = "read")]
pub fn open_file(path: impl AsRef<Path>) -> Result<LazyDatafile, BiopacError> {
    let path_buf = path.as_ref().to_path_buf();
    let file = File::open(&path_buf)?;
    let mut reader = BufReader::new(file);

    // Parse all header sections.  Reader is now at `data_start_offset`.
    let headers = parse_headers(&mut reader)?;

    let display_orders: Vec<u16> = headers
        .channel_metadata
        .iter()
        .map(|m| m.display_order)
        .collect();
    let file_revision = headers.graph_metadata.file_revision.0;
    let compressed = headers.graph_metadata.compressed;

    // Parse the marker section — for uncompressed files, seek past the data
    // block first.  Collect extra warnings separately so we can move
    // `headers.warnings` only after all borrows on `headers` are released.
    let (markers, journal, extra_warnings) = if compressed {
        // Compressed layout: Markers → Journal → Compressed data.
        // Reader is already at the start of the marker section.
        let (m, j, mw) = parse_markers_lazy(&mut reader, file_revision, &display_orders);
        (m, j, mw)
    } else {
        // Uncompressed layout: Interleaved data → Markers → Journal.
        // Seek past the data section to reach the marker section.
        if let Some(size) = headers.uncompressed_data_byte_count() {
            let target = headers.data_start_offset + size;
            match reader.seek(SeekFrom::Start(target)) {
                Ok(_) => {
                    let (m, j, mw) =
                        parse_markers_lazy(&mut reader, file_revision, &display_orders);
                    (m, j, mw)
                }
                Err(e) => {
                    let w = Warning::new(alloc::format!(
                        "LazyDatafile: could not seek past data section: {e}"
                    ));
                    (Vec::new(), None, alloc::vec![w])
                }
            }
        } else {
            // Unknown sample counts — can't seek past data section.
            let w = Warning::new(alloc::string::String::from(
                "LazyDatafile: sample counts not set in headers; \
                 markers not parsed (use into_datafile() to load all data)",
            ));
            (Vec::new(), None, alloc::vec![w])
        }
    };

    // `headers` is no longer borrowed — move its fields.
    let metadata = headers.graph_metadata;
    let channel_metadata = headers.channel_metadata;
    let mut warnings = headers.warnings;
    warnings.extend(extra_warnings);

    Ok(LazyDatafile {
        metadata,
        channel_metadata,
        markers,
        journal,
        warnings,
        path: path_buf,
        data_loaded: OnceLock::new(),
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Attempt to parse markers + journal; on error emit a warning and return empty.
#[cfg(feature = "read")]
fn parse_markers_lazy<R: std::io::Read + std::io::Seek>(
    reader: &mut R,
    file_revision: i32,
    display_orders: &[u16],
) -> (Vec<Marker>, Option<Journal>, Vec<Warning>) {
    match parse_markers_and_journal(reader, file_revision, display_orders) {
        Ok(mj) => (mj.markers, mj.journal, mj.warnings),
        Err(e) => {
            let w = Warning::new(alloc::format!("marker section unreadable: {e}"));
            (Vec::new(), None, alloc::vec![w])
        }
    }
}
