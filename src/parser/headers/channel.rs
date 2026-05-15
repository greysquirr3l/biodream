//! Per-channel header structs.
//!
//! Each channel has `lChanHeaderLen` bytes of header data. Key fields:
//! `szCommentText` (name), `szUnitsText`, `lBufLength` (sample count),
//! `dAmplScale`, `dAmplOffset`, `nVarSampleDivider`.

// TODO(T04): implement binrw ChannelHeaderRaw and TryFrom → ChannelMetadata.
