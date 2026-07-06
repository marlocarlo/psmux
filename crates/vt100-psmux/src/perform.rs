const BASE64: &[u8] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=";
const CLIPBOARD_SELECTOR: &[u8] = b"cpqs01234567";

pub struct WrappedScreen<CB: crate::callbacks::Callbacks = ()> {
    pub screen: crate::screen::Screen,
    pub callbacks: CB,
    /// Set by `hook` when a sixel DCS (final `q`, no intermediates) begins, so
    /// `put`/`unhook` know to accumulate and materialise the image.  Any other
    /// DCS leaves this `false`, preserving the pre-existing no-op behaviour.
    dcs_active: bool,
    /// Accumulates the exact re-emit bytes of an in-progress sixel DCS:
    /// `ESC P` + params + intermediates + `q` (from `hook`), the payload (from
    /// `put`), and a trailing `ST` (appended in `unhook`).
    dcs_buf: Vec<u8>,
}

impl WrappedScreen<()> {
    pub fn new(rows: u16, cols: u16, scrollback_len: usize) -> Self {
        Self::new_with_callbacks(rows, cols, scrollback_len, ())
    }
}

impl<CB: crate::callbacks::Callbacks> WrappedScreen<CB> {
    pub fn new_with_callbacks(
        rows: u16,
        cols: u16,
        scrollback_len: usize,
        callbacks: CB,
    ) -> Self {
        Self {
            screen: crate::screen::Screen::new(
                crate::grid::Size { rows, cols },
                scrollback_len,
            ),
            callbacks,
            dcs_active: false,
            dcs_buf: Vec::new(),
        }
    }
}

impl<CB: crate::callbacks::Callbacks> vte::Perform for WrappedScreen<CB> {
    fn print(&mut self, c: char) {
        if c == '\u{fffd}' || ('\u{80}'..'\u{a0}').contains(&c) {
            self.callbacks.unhandled_char(&mut self.screen, c);
        } else {
            self.screen.text(c);
        }
    }

    fn execute(&mut self, b: u8) {
        match b {
            7 => {
                self.screen.audible_bell_count = self.screen.audible_bell_count.wrapping_add(1);
                self.callbacks.audible_bell(&mut self.screen);
            }
            8 => self.screen.bs(),
            9 => self.screen.tab(),
            10 => self.screen.lf(),
            11 => self.screen.vt(),
            12 => self.screen.ff(),
            13 => self.screen.cr(),
            // we don't implement shift in/out alternate character sets, but
            // it shouldn't count as an "error"
            14 | 15 => {}
            _ => self.callbacks.unhandled_control(&mut self.screen, b),
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, b: u8) {
        if let Some(i) = intermediates.first() {
            self.callbacks.unhandled_escape(
                &mut self.screen,
                Some(*i),
                intermediates.get(1).copied(),
                b,
            );
        } else {
            match b {
                b'7' => self.screen.decsc(),
                b'8' => self.screen.decrc(),
                b'=' => self.screen.deckpam(),
                b'>' => self.screen.deckpnm(),
                b'M' => self.screen.ri(),
                b'c' => self.screen.ris(),
                b'g' => self.callbacks.visual_bell(&mut self.screen),
                _ => {
                    self.callbacks.unhandled_escape(
                        &mut self.screen,
                        None,
                        None,
                        b,
                    );
                }
            }
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        _ignore: bool,
        c: char,
    ) {
        let unhandled = |screen: &mut crate::screen::Screen| {
            self.callbacks.unhandled_csi(
                screen,
                intermediates.first().copied(),
                intermediates.get(1).copied(),
                &params.iter().collect::<Vec<_>>(),
                c,
            );
        };
        match intermediates.first() {
            None => match c {
                '@' => self.screen.ich(canonicalize_params_1(params, 1)),
                'A' => self.screen.cuu(canonicalize_params_1(params, 1)),
                'B' => self.screen.cud(canonicalize_params_1(params, 1)),
                'C' => self.screen.cuf(canonicalize_params_1(params, 1)),
                'D' => self.screen.cub(canonicalize_params_1(params, 1)),
                'E' => self.screen.cnl(canonicalize_params_1(params, 1)),
                'F' => self.screen.cpl(canonicalize_params_1(params, 1)),
                'G' => self.screen.cha(canonicalize_params_1(params, 1)),
                'H' | 'f' => self.screen.cup(canonicalize_params_2(params, 1, 1)),
                'J' => self
                    .screen
                    .ed(canonicalize_params_1(params, 0), unhandled),
                'K' => self
                    .screen
                    .el(canonicalize_params_1(params, 0), unhandled),
                'L' => self.screen.il(canonicalize_params_1(params, 1)),
                'M' => self.screen.dl(canonicalize_params_1(params, 1)),
                'P' => self.screen.dch(canonicalize_params_1(params, 1)),
                'S' => self.screen.su(canonicalize_params_1(params, 1)),
                'T' => self.screen.sd(canonicalize_params_1(params, 1)),
                'X' => self.screen.ech(canonicalize_params_1(params, 1)),
                'd' => self.screen.vpa(canonicalize_params_1(params, 1)),
                'm' => self.screen.sgr(params, unhandled),
                'n' => {
                    // DSR (Device Status Report) — in passthrough mode the
                    // child sends this and expects a response.  We ignore it
                    // at the parser level (the host must respond via the PTY
                    // writer if needed), but we must not call unhandled.
                }
                'r' => self.screen.decstbm(canonicalize_params_decstbm(
                    params,
                    self.screen.grid().size(),
                )),
                's' => self.screen.decsc(),
                'u' => self.screen.decrc(),
                't' => {
                    let mut params_iter = params.iter();
                    let op =
                        params_iter.next().and_then(|x| x.first().copied());
                    if op == Some(8) {
                        let (screen_rows, screen_cols) = self.screen.size();
                        let rows =
                            params_iter.next().map_or(screen_rows, |x| {
                                *x.first().unwrap_or(&screen_rows)
                            });
                        let cols =
                            params_iter.next().map_or(screen_cols, |x| {
                                *x.first().unwrap_or(&screen_cols)
                            });
                        self.callbacks.resize(&mut self.screen, (rows, cols));
                    } else {
                        self.callbacks.unhandled_csi(
                            &mut self.screen,
                            None,
                            None,
                            &params.iter().collect::<Vec<_>>(),
                            c,
                        );
                    }
                }
                _ => {
                    self.callbacks.unhandled_csi(
                        &mut self.screen,
                        None,
                        None,
                        &params.iter().collect::<Vec<_>>(),
                        c,
                    );
                }
            },
            Some(b'?') => match c {
                'J' => self
                    .screen
                    .decsed(canonicalize_params_1(params, 0), unhandled),
                'K' => self
                    .screen
                    .decsel(canonicalize_params_1(params, 0), unhandled),
                'h' => self.screen.decset(params, unhandled),
                'l' => self.screen.decrst(params, unhandled),
                _ => {
                    self.callbacks.unhandled_csi(
                        &mut self.screen,
                        Some(b'?'),
                        intermediates.get(1).copied(),
                        &params.iter().collect::<Vec<_>>(),
                        c,
                    );
                }
            },
            Some(i) => {
                self.callbacks.unhandled_csi(
                    &mut self.screen,
                    Some(*i),
                    intermediates.get(1).copied(),
                    &params.iter().collect::<Vec<_>>(),
                    c,
                );
            }
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bel_terminated: bool) {
        match params {
            [b"0", s] => {
                self.callbacks.set_window_icon_name(&mut self.screen, s);
                self.callbacks.set_window_title(&mut self.screen, s);
                self.screen.set_title(s);
            }
            [b"1", s] => {
                self.callbacks.set_window_icon_name(&mut self.screen, s);
            }
            [b"2", s] => {
                self.callbacks.set_window_title(&mut self.screen, s);
                self.screen.set_title(s);
            }
            [b"7", uri] => {
                self.screen.set_path(uri);
            }
            [b"9", b"4", state, progress] => {
                // OSC 9;4 — Windows Terminal progress indicator.
                //   state: 0=hide, 1=default, 2=error, 3=indeterminate, 4=warning
                //   progress: 0..=100
                let s = std::str::from_utf8(state)
                    .ok()
                    .and_then(|s| s.parse::<u8>().ok())
                    .unwrap_or(0);
                let v = std::str::from_utf8(progress)
                    .ok()
                    .and_then(|s| s.parse::<u8>().ok())
                    .unwrap_or(0);
                self.screen.set_progress(s, v);
                self.callbacks.set_progress(&mut self.screen, s, v);
            }
            [b"9999", ..] => {
                self.screen.squelch_cleared = true;
            }
            [b"8", id_params, uri_rest @ ..] => {
                // OSC 8 ; params ; URI  — hyperlink. The URI may itself contain
                // ';' (which the OSC parser splits into extra params), so rejoin
                // the trailing parts. An empty URI closes the current link.
                let mut uri = Vec::new();
                for (i, part) in uri_rest.iter().enumerate() {
                    if i > 0 {
                        uri.push(b';');
                    }
                    uri.extend_from_slice(part);
                }
                self.screen.set_hyperlink(id_params, &uri);
            }
            [b"52", ty, data] => {
                match (
                    ty.iter().all(|c| CLIPBOARD_SELECTOR.contains(c)),
                    *data,
                ) {
                    (true, b"?") => {
                        self.callbacks
                            .paste_from_clipboard(&mut self.screen, ty);
                    }
                    (true, data)
                        if data.iter().all(|c| BASE64.contains(c)) =>
                    {
                        // Stage the payload on Screen so the psmux server
                        // can drain it and forward an OSC 52 to the host
                        // terminal.  Unblocks tools like Claude Code's
                        // `/copy` running inside a pane (OSC 52 was being
                        // swallowed by the default no-op callbacks).
                        self.screen.set_clipboard(ty, data);
                        self.callbacks.copy_to_clipboard(
                            &mut self.screen,
                            ty,
                            data,
                        );
                    }
                    _ => {
                        self.callbacks
                            .unhandled_osc(&mut self.screen, params);
                    }
                }
            }
            _ => {
                self.callbacks.unhandled_osc(&mut self.screen, params);
            }
        }
    }

    // --- DCS (Device Control String) handling ------------------------------
    //
    // vte drives the DCS state machine and, for any DCS, calls hook (once, with
    // the parsed params/intermediates/final byte), then put per payload byte,
    // then unhook at the terminator.  Historically WrappedScreen left all three
    // as trait defaults, so sixel graphics (DCS `q`) were parsed and silently
    // discarded (issue #431).  We now capture ONLY sixel and leave every other
    // DCS (e.g. DECRQSS `ESC P $ q ... ST`) a no-op exactly as before.

    fn hook(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        // Sixel is DCS with final byte 'q' and NO intermediates.  DECRQSS also
        // ends in 'q' but carries the '$' (0x24) intermediate, so gating on an
        // empty intermediates slice keeps DECRQSS (and all other DCS) untouched.
        if action == 'q' && intermediates.is_empty() {
            self.dcs_buf.clear();
            // Reconstruct the exact introducer: ESC P + params + intermediates
            // + 'q'.  (intermediates is empty here but appended for fidelity.)
            self.dcs_buf.push(0x1b); // ESC
            self.dcs_buf.push(b'P');
            for (i, param) in params.iter().enumerate() {
                if i > 0 {
                    self.dcs_buf.push(b';');
                }
                // A single top-level param may carry colon-separated
                // subparams; rejoin them with ':' so the blob round-trips.
                for (j, sub) in param.iter().enumerate() {
                    if j > 0 {
                        self.dcs_buf.push(b':');
                    }
                    self.dcs_buf.extend_from_slice(sub.to_string().as_bytes());
                }
            }
            self.dcs_buf.extend_from_slice(intermediates);
            self.dcs_buf.push(b'q');
            self.dcs_active = true;
        }
        // Any other DCS: no-op (do NOT set active, do NOT route anywhere).
    }

    fn put(&mut self, byte: u8) {
        if self.dcs_active {
            self.dcs_buf.push(byte);
        }
    }

    fn unhook(&mut self) {
        if !self.dcs_active {
            return;
        }
        // Terminate the reconstructed blob with ST (ESC \).
        self.dcs_buf.push(0x1b);
        self.dcs_buf.push(b'\\');

        let (px_width, px_height) = sixel_pixel_dims(&self.dcs_buf);
        let (cell_width, cell_height) =
            crate::image::cell_dims_from_px(px_width, px_height);

        let img = crate::image::SixelImage {
            id: 0,             // assigned by add_image
            raw: self.dcs_buf.clone(),
            px_width,
            px_height,
            cell_width,
            cell_height,
            anchor_line: 0,    // filled by add_image
            anchor_col: 0,     // filled by add_image
            on_alt: false,     // filled by add_image
        };
        self.screen.add_image(img);

        // Cursor advance (tmux sixel-scrolling = xterm default): move to the
        // left margin, then down cell_height rows, reusing cr()/lf() so scroll
        // region, scrollback push and first_line accounting stay uniform.
        self.screen.cr();
        for _ in 0..cell_height {
            self.screen.lf();
        }

        self.dcs_active = false;
        self.dcs_buf.clear();
    }
}

/// Determines a sixel image's pixel dimensions from its reconstructed DCS blob.
///
/// Prefers the raster attributes command `"Pan;Pad;Ph;Pv` (the width `Ph` and
/// height `Pv` follow the introducer's `q`).  If the raster is absent, falls
/// back to scanning the sixel data: width = the widest band's column count
/// (honouring `!Rn` run-length repeats), height = 6 pixels per band.  The
/// fallback is deliberately conservative and only feeds cell-footprint math
/// (cursor advance / clipping), never the rasterised pixels.
fn sixel_pixel_dims(buf: &[u8]) -> (u32, u32) {
    // Locate the payload: everything after the introducer's final 'q'.  The
    // blob always starts with ESC P, so search from index 2 onward for 'q'.
    let payload_start = buf
        .iter()
        .position(|&b| b == b'q')
        .map_or(buf.len(), |i| i + 1);
    let payload = &buf[payload_start..];

    // Raster attributes: `"Pan;Pad;Ph;Pv`.  Scan for the '"' and read the
    // following ';'-separated decimal fields.
    if let Some(q) = payload.iter().position(|&b| b == b'"') {
        let mut fields: [u32; 4] = [0; 4];
        let mut idx = 0usize;
        let mut cur: u32 = 0;
        let mut saw_digit = false;
        for &b in &payload[q + 1..] {
            match b {
                b'0'..=b'9' => {
                    cur = cur.saturating_mul(10).saturating_add(u32::from(b - b'0'));
                    saw_digit = true;
                }
                b';' => {
                    if idx < 4 {
                        fields[idx] = cur;
                    }
                    idx += 1;
                    cur = 0;
                    if idx >= 4 {
                        break;
                    }
                }
                _ => break,
            }
        }
        if idx < 4 && saw_digit {
            fields[idx] = cur;
            idx += 1;
        }
        // Need all four fields (Pan;Pad;Ph;Pv) with a non-zero size to trust it.
        if idx >= 4 && fields[2] > 0 && fields[3] > 0 {
            return (fields[2], fields[3]);
        }
    }

    // Fallback: scan the sixel data to estimate dimensions.
    let mut cur_x: u32 = 0;
    let mut max_x: u32 = 0;
    let mut bands: u32 = 1;
    let mut i = 0usize;
    while i < payload.len() {
        let b = payload[i];
        match b {
            b'!' => {
                // Run-length: !Rn <sixel-char> repeats the next data char n times.
                i += 1;
                let mut n: u32 = 0;
                while i < payload.len() && payload[i].is_ascii_digit() {
                    n = n.saturating_mul(10).saturating_add(u32::from(payload[i] - b'0'));
                    i += 1;
                }
                // Consume the repeated data char (if present); it advances n cols.
                if i < payload.len() && (0x3f..=0x7e).contains(&payload[i]) {
                    i += 1;
                }
                cur_x = cur_x.saturating_add(n.max(1));
                continue;
            }
            b'#' => {
                // Color introducer: skip its numeric/';'-separated params (no x).
                i += 1;
                while i < payload.len()
                    && (payload[i].is_ascii_digit() || payload[i] == b';')
                {
                    i += 1;
                }
                continue;
            }
            b'"' => {
                // Raster (already handled above); skip its numeric params here.
                i += 1;
                while i < payload.len()
                    && (payload[i].is_ascii_digit() || payload[i] == b';')
                {
                    i += 1;
                }
                continue;
            }
            b'$' => {
                // Carriage return within the band: reset x, same band.
                max_x = max_x.max(cur_x);
                cur_x = 0;
            }
            b'-' => {
                // Graphics newline: next band, 6 more pixels tall.
                max_x = max_x.max(cur_x);
                cur_x = 0;
                bands = bands.saturating_add(1);
            }
            0x3f..=0x7e => {
                // A sixel data byte occupies one column.
                cur_x = cur_x.saturating_add(1);
            }
            0x1b => break, // ESC of the ST terminator
            _ => {}
        }
        i += 1;
    }
    max_x = max_x.max(cur_x);
    (max_x.max(1), bands.saturating_mul(6))
}

fn canonicalize_params_1(params: &vte::Params, default: u16) -> u16 {
    let first = params.iter().next().map_or(0, |x| *x.first().unwrap_or(&0));
    if first == 0 {
        default
    } else {
        first
    }
}

fn canonicalize_params_2(
    params: &vte::Params,
    default1: u16,
    default2: u16,
) -> (u16, u16) {
    let mut iter = params.iter();
    let first = iter.next().map_or(0, |x| *x.first().unwrap_or(&0));
    let first = if first == 0 { default1 } else { first };

    let second = iter.next().map_or(0, |x| *x.first().unwrap_or(&0));
    let second = if second == 0 { default2 } else { second };

    (first, second)
}

fn canonicalize_params_decstbm(
    params: &vte::Params,
    size: crate::grid::Size,
) -> (u16, u16) {
    let mut iter = params.iter();
    let top = iter.next().map_or(0, |x| *x.first().unwrap_or(&0));
    let top = if top == 0 { 1 } else { top };

    let bottom = iter.next().map_or(0, |x| *x.first().unwrap_or(&0));
    let bottom = if bottom == 0 { size.rows } else { bottom };

    (top, bottom)
}

#[cfg(test)]
#[path = "../../../tests-rs/test_issue431_sixel_parse.rs"]
mod test_issue431_sixel_parse;
