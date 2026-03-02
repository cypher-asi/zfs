/// Shorten an identifier by keeping `prefix_len` leading and `suffix_len`
/// trailing characters, separated by `"..."`.
pub(crate) fn shorten_id(id: &str, prefix_len: usize, suffix_len: usize) -> String {
    let min_len = prefix_len + suffix_len + 3;
    if id.len() > min_len {
        format!("{}...{}", &id[..prefix_len], &id[id.len() - suffix_len..])
    } else {
        id.to_string()
    }
}

/// Shorten a Zode ID by stripping the common prefix.
pub(crate) fn shorten_zid(id: &str, tail_chars: usize) -> String {
    const ZODE_PREFIX: &str = "Zx12D3KooW";
    if let Some(unique) = id.strip_prefix(ZODE_PREFIX) {
        let n = tail_chars.min(unique.len());
        format!("Zx..{}", &unique[unique.len() - n..])
    } else {
        shorten_id(id, 4, tail_chars)
    }
}

/// Format a duration in seconds as HH:MM:SS.
pub(crate) fn format_uptime(secs: u64) -> String {
    let h = (secs / 3600) % 24;
    let m = (secs / 60) % 60;
    let s = secs % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

/// Palette of distinct colors for nodes.
pub(crate) fn node_color(index: usize) -> eframe::egui::Color32 {
    use eframe::egui::Color32;
    const PALETTE: &[Color32] = &[
        Color32::from_rgb(0, 180, 255),
        Color32::from_rgb(46, 230, 176),
        Color32::from_rgb(255, 140, 60),
        Color32::from_rgb(180, 130, 255),
        Color32::from_rgb(255, 100, 100),
        Color32::from_rgb(255, 200, 100),
        Color32::from_rgb(100, 200, 255),
        Color32::from_rgb(100, 255, 150),
        Color32::from_rgb(255, 100, 200),
        Color32::from_rgb(200, 200, 100),
    ];
    PALETTE[index % PALETTE.len()]
}
