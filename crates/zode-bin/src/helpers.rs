pub(crate) use grid_core::format_bytes;

pub(crate) fn format_timestamp_ms(ms: u64) -> String {
    if ms == 0 {
        return "--:--:--".to_string();
    }
    let secs = ms / 1000;
    let h = (secs / 3600) % 24;
    let m = (secs / 60) % 60;
    let s = secs % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

/// Shorten a long identifier by keeping `prefix_len` leading and `suffix_len`
/// trailing characters, separated by `"..."`.  Returns the original string
/// unchanged when it is short enough.
pub(crate) fn shorten_id(id: &str, prefix_len: usize, suffix_len: usize) -> String {
    let min_len = prefix_len + suffix_len + 3;
    if id.len() > min_len {
        format!("{}...{}", &id[..prefix_len], &id[id.len() - suffix_len..])
    } else {
        id.to_string()
    }
}

/// Shorten a ZODE ID or DID string by stripping common prefixes and showing
/// only the last `tail_chars` unique characters.
pub(crate) fn shorten_zid(id: &str, tail_chars: usize) -> String {
    const ZODE_PREFIX: &str = "Zx12D3KooW";
    const DID_PREFIX: &str = "did:key:z6Mk";
    if let Some(unique) = id.strip_prefix(ZODE_PREFIX) {
        let n = tail_chars.min(unique.len());
        format!("Zx..{}", &unique[unique.len() - n..])
    } else if let Some(unique) = id.strip_prefix(DID_PREFIX) {
        let n = tail_chars.min(unique.len());
        format!("did:..{}", &unique[unique.len() - n..])
    } else {
        shorten_id(id, 4, tail_chars)
    }
}
