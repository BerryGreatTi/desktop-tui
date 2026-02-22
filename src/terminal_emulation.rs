use appcui::prelude::{CharFlags, Character, Color, Surface};

#[derive(Clone, Copy)]
struct CellData {
    character: char,
    foreground: Color,
    background: Color,
    flags: CharFlags,
}

impl CellData {
    fn default_with_bg(bg: Color) -> Self {
        Self {
            character: ' ',
            foreground: Color::RGB(255, 255, 255),
            background: bg,
            flags: CharFlags::None,
        }
    }
}

impl Default for CellData {
    fn default() -> Self {
        Self {
            character: ' ',
            foreground: Color::RGB(255, 255, 255),
            background: Color::RGB(0, 0, 0),
            flags: CharFlags::None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct TerminalState {
    default_foreground_color: Color,
    default_background_color: Color,
    foreground: Color,
    background: Color,
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    reverse: bool,
    strikethrough: bool,
    cursor_x: i32,
    cursor_y: i32,
}

impl TerminalState {
    fn reset(&mut self) {
        self.foreground = self.default_foreground_color;
        self.background = self.default_background_color;
        self.bold = false;
        self.dim = false;
        self.italic = false;
        self.underline = false;
        self.reverse = false;
        self.strikethrough = false;
        self.cursor_x = 0;
        self.cursor_y = 0;
    }
}

pub struct TerminalParser {
    width: u32,
    height: u32,
    state: TerminalState,
    cells: Vec<Vec<CellData>>,
    saved_state: Option<TerminalState>,
    main_cells: Option<Vec<Vec<CellData>>>,
    main_state: Option<TerminalState>,
}

impl TerminalParser {
    pub fn new(width: u32, height: u32, default_background_color: Color) -> Self {
        let state = TerminalState {
            default_foreground_color: Color::RGB(255, 255, 255),
            default_background_color,
            foreground: Color::RGB(255, 255, 255),
            background: default_background_color,
            bold: false,
            dim: false,
            italic: false,
            underline: false,
            reverse: false,
            strikethrough: false,
            cursor_x: 0,
            cursor_y: 0,
        };
        let cells = vec![vec![CellData::default_with_bg(default_background_color); width as usize]; height as usize];
        Self {
            width,
            height,
            state,
            cells,
            saved_state: None,
            main_cells: None,
            main_state: None,
        }
    }

    pub fn parse_to_surface(&mut self, data: &[u8], mut surface: Surface) -> Surface {
        let text = String::from_utf8_lossy(data);
        let chars: Vec<char> = text.chars().collect();

        let mut i = 0;

        while i < chars.len() {
            if chars[i] == '\u{1b}' && i + 1 < chars.len() {
                match chars[i + 1] {
                    '[' => {
                        // CSI sequence - re-encode remaining chars into bytes
                        let slice: String = chars[i..].iter().collect();
                        let consumed = self.parse_ansi_sequence(slice.as_bytes(), &mut surface);
                        let consumed_chars = String::from_utf8_lossy(&slice.as_bytes()[..consumed])
                            .chars()
                            .count();
                        i += consumed_chars;
                    }
                    ']' => {
                        // OSC sequence
                        let consumed = self.skip_osc(&chars[i..]);
                        i += consumed;
                    }
                    'P' => {
                        // DCS sequence
                        let consumed = self.skip_dcs(&chars[i..]);
                        i += consumed;
                    }
                    '7' => {
                        // DECSC: save cursor
                        self.saved_state = Some(self.state);
                        i += 2;
                    }
                    '8' => {
                        // DECRC: restore cursor
                        if let Some(saved) = self.saved_state {
                            self.state = saved;
                        }
                        i += 2;
                    }
                    '(' | ')' | '*' | '+' => {
                        // Character set designation: skip ESC + designator + 1 char
                        i += 3;
                    }
                    'M' => {
                        // Reverse index (scroll down one line)
                        if self.state.cursor_y == 0 {
                            self.scroll_down(1);
                        } else {
                            self.state.cursor_y -= 1;
                        }
                        i += 2;
                    }
                    'c' => {
                        // RIS: full reset
                        let bg = self.state.default_background_color;
                        self.state.reset();
                        self.cells = vec![vec![CellData::default_with_bg(bg); self.width as usize]; self.height as usize];
                        i += 2;
                    }
                    _ => {
                        // skip unknown ESC sequences
                        i += 1;
                    }
                }
            } else {
                self.write_character(chars[i]);
                i += 1;
            }
        }

        // Flush shadow buffer to surface
        for row in 0..self.height as usize {
            for col in 0..self.width as usize {
                let cell = &self.cells[row][col];
                surface.write_char(
                    col as i32,
                    row as i32,
                    Character::new(cell.character, cell.foreground, cell.background, cell.flags),
                );
            }
        }

        surface
    }

    fn skip_osc(&self, chars: &[char]) -> usize {
        let mut i = 2; // skip ESC ]
        while i < chars.len() {
            if chars[i] == '\x07' {
                return i + 1; // BEL terminates
            }
            if chars[i] == '\x1b' && i + 1 < chars.len() && chars[i + 1] == '\\' {
                return i + 2; // ST terminates
            }
            i += 1;
        }
        chars.len() // consume all if unterminated
    }

    fn skip_dcs(&self, chars: &[char]) -> usize {
        let mut i = 2; // skip ESC P
        while i < chars.len() {
            if chars[i] == '\x1b' && i + 1 < chars.len() && chars[i + 1] == '\\' {
                return i + 2; // ST terminates
            }
            i += 1;
        }
        chars.len() // consume all if unterminated
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        let old_width = self.width;
        let old_height = self.height;
        self.width = width;
        self.height = height;

        let bg = self.state.default_background_color;

        // Resize cells: truncate or extend rows
        self.cells.resize_with(height as usize, || {
            vec![CellData::default_with_bg(bg); width as usize]
        });

        // Resize each row: truncate or extend columns
        for row in self.cells.iter_mut() {
            row.resize_with(width as usize, || CellData::default_with_bg(bg));
        }

        // Clamp cursor
        if self.state.cursor_x >= width as i32 {
            self.state.cursor_x = width as i32 - 1;
        }
        if self.state.cursor_y >= height as i32 {
            self.state.cursor_y = height as i32 - 1;
        }

        let _ = (old_width, old_height);
    }

    fn scroll_up(&mut self, n: u32) {
        let bg = self.state.default_background_color;
        for _ in 0..n {
            if !self.cells.is_empty() {
                self.cells.remove(0);
                self.cells.push(vec![CellData::default_with_bg(bg); self.width as usize]);
            }
        }
    }

    fn scroll_down(&mut self, n: u32) {
        let bg = self.state.default_background_color;
        for _ in 0..n {
            self.cells.pop();
            self.cells.insert(0, vec![CellData::default_with_bg(bg); self.width as usize]);
        }
    }

    fn insert_lines(&mut self, n: u32) {
        let bg = self.state.default_background_color;
        let y = self.state.cursor_y as usize;
        for _ in 0..n {
            if self.cells.len() > 0 {
                self.cells.pop(); // remove last row to keep height
            }
            self.cells.insert(y, vec![CellData::default_with_bg(bg); self.width as usize]);
        }
    }

    fn delete_lines(&mut self, n: u32) {
        let bg = self.state.default_background_color;
        let y = self.state.cursor_y as usize;
        for _ in 0..n {
            if y < self.cells.len() {
                self.cells.remove(y);
                self.cells.push(vec![CellData::default_with_bg(bg); self.width as usize]);
            }
        }
    }

    fn delete_chars(&mut self, n: u32) {
        let bg = self.state.default_background_color;
        let y = self.state.cursor_y as usize;
        let x = self.state.cursor_x as usize;
        if y < self.cells.len() {
            let row = &mut self.cells[y];
            for _ in 0..n {
                if x < row.len() {
                    row.remove(x);
                    row.push(CellData::default_with_bg(bg));
                }
            }
        }
    }

    fn insert_chars(&mut self, n: u32) {
        let bg = self.state.default_background_color;
        let y = self.state.cursor_y as usize;
        let x = self.state.cursor_x as usize;
        if y < self.cells.len() {
            let row = &mut self.cells[y];
            for _ in 0..n {
                if x <= row.len() {
                    row.insert(x, CellData::default_with_bg(bg));
                    if row.len() > self.width as usize {
                        row.truncate(self.width as usize);
                    }
                }
            }
        }
    }

    fn parse_ansi_sequence(&mut self, data: &[u8], surface: &mut Surface) -> usize {
        if data.len() < 3 {
            return 1; // Skip invalid sequence
        }

        let mut i = 2; // Skip '\x1b['
        let mut params = Vec::new();
        let mut current_param = String::new();
        let mut private_mode = false;

        // Handle private mode prefix '?'
        if i < data.len() && data[i] == b'?' {
            private_mode = true;
            i += 1;
        }

        // Parse parameters
        while i < data.len() {
            let byte = data[i];
            match byte {
                b'0'..=b'9' => current_param.push(byte as char),
                b';' => {
                    params.push(current_param.parse::<u32>().unwrap_or(0));
                    current_param.clear();
                }
                b'A'..=b'Z' | b'a'..=b'z' | b'@' => {
                    // End of sequence
                    if !current_param.is_empty() {
                        params.push(current_param.parse::<u32>().unwrap_or(0));
                    }
                    if private_mode {
                        self.handle_private_ansi_command(byte as char, &params, surface);
                    } else {
                        self.handle_ansi_command(byte as char, &params, surface);
                    }
                    return i + 1;
                }
                _ => break,
            }
            i += 1;
        }

        1 // Skip if we couldn't parse
    }

    fn handle_ansi_command(&mut self, command: char, params: &[u32], surface: &mut Surface) {
        match command {
            'H' | 'f' => {
                // Cursor position
                let row = params.get(0).unwrap_or(&1).saturating_sub(1) as i32;
                let col = params.get(1).unwrap_or(&1).saturating_sub(1) as i32;
                self.state.cursor_x = col.min(self.width as i32 - 1);
                self.state.cursor_y = row.min(self.height as i32 - 1);
            }
            'A' => {
                // Cursor up
                let count = params.get(0).unwrap_or(&1);
                self.state.cursor_y = (self.state.cursor_y - *count as i32).max(0);
            }
            'B' => {
                // Cursor down
                let count = params.get(0).unwrap_or(&1);
                self.state.cursor_y = (self.state.cursor_y + *count as i32).min(self.height as i32 - 1);
            }
            'C' => {
                // Cursor right
                let count = params.get(0).unwrap_or(&1);
                self.state.cursor_x = (self.state.cursor_x + *count as i32).min(self.width as i32 - 1);
            }
            'D' => {
                // Cursor left
                let count = params.get(0).unwrap_or(&1);
                self.state.cursor_x = (self.state.cursor_x - *count as i32).max(0);
            }
            'G' => {
                // Cursor horizontal absolute
                let col = params.get(0).unwrap_or(&1).saturating_sub(1) as i32;
                self.state.cursor_x = col.min(self.width as i32 - 1);
            }
            'd' => {
                // Cursor vertical absolute
                let row = params.get(0).unwrap_or(&1).saturating_sub(1) as i32;
                self.state.cursor_y = row.min(self.height as i32 - 1);
            }
            'E' => {
                // Cursor next line
                let count = params.get(0).unwrap_or(&1);
                self.state.cursor_y = (self.state.cursor_y + *count as i32).min(self.height as i32 - 1);
                self.state.cursor_x = 0;
            }
            'F' => {
                // Cursor previous line
                let count = params.get(0).unwrap_or(&1);
                self.state.cursor_y = (self.state.cursor_y - *count as i32).max(0);
                self.state.cursor_x = 0;
            }
            'm' => {
                // SGR (Select Graphic Rendition) - colors and attributes
                if params.is_empty() {
                    // Reset all attributes
                    self.state.reset();
                } else {
                    self.handle_sgr_params(params);
                }
            }
            'J' => {
                // Clear screen
                let mode = params.get(0).copied().unwrap_or(0);
                self.handle_erase_display(mode);
            }
            'K' => {
                // Clear line
                let mode = params.get(0).copied().unwrap_or(0);
                self.handle_erase_line(mode);
            }
            'S' => {
                // Scroll up
                let count = params.get(0).unwrap_or(&1);
                self.scroll_up(*count);
            }
            'T' => {
                // Scroll down
                let count = params.get(0).unwrap_or(&1);
                self.scroll_down(*count);
            }
            'L' => {
                // Insert lines at cursor
                let count = params.get(0).unwrap_or(&1);
                self.insert_lines(*count);
            }
            'M' => {
                // Delete lines at cursor
                let count = params.get(0).unwrap_or(&1);
                self.delete_lines(*count);
            }
            'X' => {
                // Erase characters (replace with spaces from cursor)
                let count = params.get(0).unwrap_or(&1);
                let bg = self.state.default_background_color;
                let y = self.state.cursor_y as usize;
                if y < self.cells.len() {
                    for dx in 0..*count as i32 {
                        let x = (self.state.cursor_x + dx) as usize;
                        if x < self.width as usize {
                            self.cells[y][x] = CellData::default_with_bg(bg);
                        }
                    }
                }
            }
            'P' => {
                // Delete characters (shift left)
                let count = params.get(0).unwrap_or(&1);
                self.delete_chars(*count);
            }
            '@' => {
                // Insert characters (shift right)
                let count = params.get(0).unwrap_or(&1);
                self.insert_chars(*count);
            }
            's' => {
                // Save cursor position
                self.saved_state = Some(self.state);
            }
            'u' => {
                // Restore cursor position
                if let Some(saved) = self.saved_state {
                    self.state = saved;
                }
            }
            'r' => {
                // DECSTBM: set scrolling region - ignore for now but consume
            }
            _ => {
                // Ignore unknown sequences
                let _ = surface;
            }
        }
    }

    fn handle_private_ansi_command(&mut self, command: char, params: &[u32], surface: &mut Surface) {
        match command {
            'l' => {
                for &p in params {
                    match p {
                        25 => surface.hide_cursor(),
                        1049 => {
                            // Restore main screen
                            if let Some(saved_cells) = self.main_cells.take() {
                                self.cells = saved_cells;
                            }
                            if let Some(saved_state) = self.main_state.take() {
                                self.state = saved_state;
                            }
                        }
                        2004 => {} // bracketed paste - no-op
                        _ => {}
                    }
                }
                // If params is empty, default to hide cursor for backward compat
                if params.is_empty() {
                    surface.hide_cursor();
                }
            }
            'h' => {
                for &p in params {
                    match p {
                        25 => surface.set_cursor(self.state.cursor_x, self.state.cursor_y),
                        1049 => {
                            // Save main screen, switch to alt
                            self.main_cells = Some(self.cells.clone());
                            self.main_state = Some(self.state);
                            let bg = self.state.default_background_color;
                            self.cells = vec![vec![CellData::default_with_bg(bg); self.width as usize]; self.height as usize];
                            self.state.cursor_x = 0;
                            self.state.cursor_y = 0;
                        }
                        2004 => {} // bracketed paste - no-op
                        _ => {}
                    }
                }
                // If params is empty, default to show cursor for backward compat
                if params.is_empty() {
                    surface.set_cursor(self.state.cursor_x, self.state.cursor_y);
                }
            }
            _ => {
                // ignore unknown private sequences
            }
        }
    }

    fn handle_erase_display(&mut self, param: u32) {
        let bg = self.state.default_background_color;
        match param {
            0 => {
                // clear from cursor to end of screen
                let cy = self.state.cursor_y as usize;
                let cx = self.state.cursor_x as usize;
                for y in 0..self.height as usize {
                    let start_x = if y == cy { cx } else if y > cy { 0 } else { continue };
                    for x in start_x..self.width as usize {
                        if y < self.cells.len() && x < self.cells[y].len() {
                            self.cells[y][x] = CellData::default_with_bg(bg);
                        }
                    }
                }
            }
            1 => {
                // clear from beginning of screen to cursor
                let cy = self.state.cursor_y as usize;
                let cx = self.state.cursor_x as usize;
                for y in 0..=cy.min(self.height as usize - 1) {
                    let end_x = if y == cy { cx + 1 } else { self.width as usize };
                    if y < self.cells.len() {
                        for x in 0..end_x.min(self.width as usize) {
                            if x < self.cells[y].len() {
                                self.cells[y][x] = CellData::default_with_bg(bg);
                            }
                        }
                    }
                }
            }
            2 | 3 => {
                // clear entire screen
                for row in self.cells.iter_mut() {
                    for cell in row.iter_mut() {
                        *cell = CellData::default_with_bg(bg);
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_erase_line(&mut self, param: u32) {
        let bg = self.state.default_background_color;
        let y = self.state.cursor_y as usize;
        if y >= self.cells.len() {
            return;
        }
        match param {
            0 => {
                // clear from cursor to end of line
                let cx = self.state.cursor_x as usize;
                for x in cx..self.width as usize {
                    if x < self.cells[y].len() {
                        self.cells[y][x] = CellData::default_with_bg(bg);
                    }
                }
            }
            1 => {
                // clear from beginning of line to cursor
                let cx = self.state.cursor_x as usize;
                for x in 0..=(cx.min(self.width as usize - 1)) {
                    if x < self.cells[y].len() {
                        self.cells[y][x] = CellData::default_with_bg(bg);
                    }
                }
            }
            2 => {
                // clear entire line
                for x in 0..self.width as usize {
                    if x < self.cells[y].len() {
                        self.cells[y][x] = CellData::default_with_bg(bg);
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_sgr_params(&mut self, params: &[u32]) {
        let mut iter = params.iter().copied().peekable();

        while let Some(param) = iter.next() {
            match param {
                0 => self.state.reset(), // Reset
                1 => self.state.bold = true,
                2 => self.state.dim = true,
                3 => self.state.italic = true,
                4 => self.state.underline = true,
                7 => self.state.reverse = true,
                9 => self.state.strikethrough = true,
                22 => {
                    self.state.bold = false;
                    self.state.dim = false;
                }
                23 => self.state.italic = false,
                24 => self.state.underline = false,
                27 => self.state.reverse = false,
                29 => self.state.strikethrough = false,

                39 => self.state.foreground = self.state.default_foreground_color,
                49 => self.state.background = self.state.default_background_color,

                // 16-color standard + bright
                30..=37 => self.state.foreground = ansi_16_color(param - 30, false),
                40..=47 => self.state.background = ansi_16_color(param - 40, false),
                90..=97 => self.state.foreground = ansi_16_color(param - 90, true),
                100..=107 => self.state.background = ansi_16_color(param - 100, true),

                // Extended color sequences
                38 | 48 => {
                    let is_foreground = param == 38;

                    if let Some(mode) = iter.next() {
                        match mode {
                            5 => {
                                // 256-color: 38;5;<idx> or 48;5;<idx>
                                if let Some(idx) = iter.next() {
                                    let color = ansi_256_color(idx);
                                    if is_foreground {
                                        self.state.foreground = color;
                                    } else {
                                        self.state.background = color;
                                    }
                                }
                            }
                            2 => {
                                // Truecolor: 38;2;<r>;<g>;<b> or 48;2;<r>;<g>;<b>
                                if let (Some(r), Some(g), Some(b)) = (iter.next(), iter.next(), iter.next()) {
                                    let color = Color::RGB(r as u8, g as u8, b as u8);

                                    if is_foreground {
                                        self.state.foreground = color;
                                    } else {
                                        self.state.background = color;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }

                _ => {
                    // Ignore unknown
                }
            }
        }
    }

    fn write_character(&mut self, ch: char) {
        match ch {
            '\r' => {
                self.state.cursor_x = 0;
            }
            '\n' => {
                self.state.cursor_x = 0;
                self.state.cursor_y += 1;
                if self.state.cursor_y >= self.height as i32 {
                    self.scroll_up(1);
                    self.state.cursor_y = self.height as i32 - 1;
                }
            }
            '\t' => {
                // Tab to next 8-character boundary
                self.state.cursor_x = ((self.state.cursor_x / 8) + 1) * 8;
                if self.state.cursor_x >= self.width as i32 {
                    self.cursor_forward();
                }
            }
            '\x08' => {
                // Backspace
                if self.state.cursor_x > 0 {
                    self.state.cursor_x -= 1;
                }
            }
            c if c.is_control() => {
                // Ignore other control characters
            }
            c => {
                // Regular printable character
                let mut flags = CharFlags::None;
                if self.state.bold {
                    flags |= CharFlags::Bold;
                }
                if self.state.italic {
                    flags |= CharFlags::Italic;
                }
                if self.state.underline {
                    flags |= CharFlags::Underline;
                }

                let (fg, bg) = if self.state.reverse {
                    (self.state.background, self.state.foreground)
                } else {
                    (self.state.foreground, self.state.background)
                };

                let y = self.state.cursor_y as usize;
                let x = self.state.cursor_x as usize;

                if y < self.cells.len() && x < self.cells[y].len() {
                    self.cells[y][x] = CellData {
                        character: c,
                        foreground: fg,
                        background: bg,
                        flags,
                    };
                }

                self.cursor_forward();
            }
        }
    }

    pub fn cursor_forward(&mut self) {
        // Advance cursor
        self.state.cursor_x += 1;
        if self.state.cursor_x >= self.width as i32 {
            self.state.cursor_x = 0;
            self.state.cursor_y += 1;
            if self.state.cursor_y >= self.height as i32 {
                self.scroll_up(1);
                self.state.cursor_y = self.height as i32 - 1;
            }
        }
    }
}

/// Map 16 ANSI colors to RGB
fn ansi_16_color(code: u32, bright: bool) -> Color {
    let (r, g, b): (u8, u8, u8) = match code {
        0 => (0, 0, 0),       // Black
        1 => (128, 0, 0),     // Red
        2 => (0, 128, 0),     // Green
        3 => (128, 128, 0),   // Yellow
        4 => (0, 0, 128),     // Blue
        5 => (128, 0, 128),   // Magenta
        6 => (0, 128, 128),   // Cyan
        7 => (192, 192, 192), // White (light gray)
        _ => (255, 255, 255),
    };

    if bright {
        Color::RGB(
            r.saturating_mul(2).min(255),
            g.saturating_mul(2).min(255),
            b.saturating_mul(2).min(255)
        )
    } else {
        Color::RGB(r, g, b)
    }
}

/// Map 256-color palette to RGB
fn ansi_256_color(idx: u32) -> Color {
    match idx {
        0..=15 => {
            // Reuse 16 ANSI
            if idx < 8 {
                ansi_16_color(idx, false)
            } else {
                ansi_16_color(idx - 8, true)
            }
        }
        16..=231 => {
            // 6x6x6 color cube
            let n = idx - 16;
            let r = (n / 36) % 6;
            let g = (n / 6) % 6;
            let b = n % 6;
            Color::RGB(
                (r * 51) as u8,
                (g * 51) as u8,
                (b * 51) as u8,
            )
        }
        232..=255 => {
            // Grayscale ramp (24 shades)
            let level = 8 + (idx - 232) * 10;
            Color::RGB(level as u8, level as u8, level as u8)
        }
        _ => Color::RGB(0, 0, 0),
    }
}
