#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Visual,
    VisualLine,
}

impl Mode {
    pub fn status_label(self) -> &'static str {
        match self {
            Self::Normal => "-- NORMAL --",
            Self::Insert => "-- INSERT --",
            Self::Visual => "-- VISUAL --",
            Self::VisualLine => "-- V-LINE --",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VimKey {
    Char(char),
    Esc,
    Enter,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    CtrlD,
    CtrlU,
    CtrlR,
}

#[derive(Debug, Clone)]
pub struct VimState {
    pub mode: Mode,
    pub count: Option<usize>,
    pub pending_op: Option<char>,
    pub register: String,
    pub register_linewise: bool,
    pub visual_anchor: Option<usize>,
    pub preferred_col: Option<usize>,
    pub undo_stack: Vec<(String, usize)>,
    pub redo_stack: Vec<(String, usize)>,
}

impl Default for VimState {
    fn default() -> Self {
        Self {
            mode: Mode::Normal,
            count: None,
            pending_op: None,
            register: String::new(),
            register_linewise: false,
            visual_anchor: None,
            preferred_col: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }
}

impl VimState {
    pub fn reset_transient(&mut self) {
        self.count = None;
        self.pending_op = None;
    }

    pub fn push_undo(&mut self, text: &str, cursor: usize) {
        if self
            .undo_stack
            .last()
            .is_some_and(|(t, c)| t == text && *c == cursor)
        {
            return;
        }
        self.undo_stack.push((text.to_string(), cursor));
        if self.undo_stack.len() > 200 {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    /// Returns `true` if the key was consumed by the Vim state machine.
    pub fn handle_key(&mut self, key: VimKey, text: &mut String, cursor: &mut usize) -> bool {
        let len = text.chars().count();
        *cursor = (*cursor).min(len);

        match self.mode {
            Mode::Insert => {
                if key == VimKey::Esc {
                    self.mode = Mode::Normal;
                    self.reset_transient();
                    self.preferred_col = None;
                    let chars: Vec<char> = text.chars().collect();
                    if *cursor > 0 && (*cursor >= chars.len() || chars.get(*cursor) == Some(&'\n'))
                    {
                        let line_start = line_start_char(&chars, *cursor);
                        if *cursor > line_start {
                            *cursor -= 1;
                        }
                    }
                    return true;
                }
                false
            }
            Mode::Visual | Mode::VisualLine => self.handle_visual(key, text, cursor),
            Mode::Normal => self.handle_normal(key, text, cursor),
        }
    }

    pub fn visual_selection_range(&self, text: &str, cursor: usize) -> Option<(usize, usize)> {
        let anchor = self.visual_anchor?;
        let chars: Vec<char> = text.chars().collect();
        let cur = cursor.min(chars.len());
        let anc = anchor.min(chars.len());
        match self.mode {
            Mode::Visual => {
                let (lo, hi) = if anc <= cur { (anc, cur) } else { (cur, anc) };
                Some((lo, (hi + 1).min(chars.len())))
            }
            Mode::VisualLine => {
                let (lo, hi) = if anc <= cur { (anc, cur) } else { (cur, anc) };
                let start = line_start_char(&chars, lo);
                let mut end = line_end_char(&chars, hi);
                if end < chars.len() && chars[end] == '\n' {
                    end += 1;
                }
                Some((start, end))
            }
            _ => None,
        }
    }

    fn handle_visual(&mut self, key: VimKey, text: &mut String, cursor: &mut usize) -> bool {
        match key {
            VimKey::Esc => {
                self.mode = Mode::Normal;
                self.visual_anchor = None;
                self.reset_transient();
                true
            }
            VimKey::Char('v') if self.mode == Mode::Visual => {
                self.mode = Mode::Normal;
                self.visual_anchor = None;
                self.reset_transient();
                true
            }
            VimKey::Char('V') if self.mode == Mode::VisualLine => {
                self.mode = Mode::Normal;
                self.visual_anchor = None;
                self.reset_transient();
                true
            }
            VimKey::Char('v') => {
                self.mode = Mode::Visual;
                true
            }
            VimKey::Char('V') => {
                self.mode = Mode::VisualLine;
                true
            }
            VimKey::Char('d') | VimKey::Char('x') | VimKey::Delete => {
                if let Some((start, end)) = self.visual_selection_range(text, *cursor) {
                    self.push_undo(text, *cursor);
                    let linewise = self.mode == Mode::VisualLine;
                    self.register = slice_chars(text, start, end);
                    self.register_linewise = linewise;
                    remove_char_range(text, start, end);
                    *cursor = start.min(text.chars().count());
                }
                self.mode = Mode::Normal;
                self.visual_anchor = None;
                self.reset_transient();
                true
            }
            VimKey::Char('c') => {
                if let Some((start, end)) = self.visual_selection_range(text, *cursor) {
                    self.push_undo(text, *cursor);
                    self.register = slice_chars(text, start, end);
                    self.register_linewise = self.mode == Mode::VisualLine;
                    remove_char_range(text, start, end);
                    *cursor = start.min(text.chars().count());
                }
                self.mode = Mode::Insert;
                self.visual_anchor = None;
                self.reset_transient();
                true
            }
            VimKey::Char('y') => {
                if let Some((start, end)) = self.visual_selection_range(text, *cursor) {
                    self.register = slice_chars(text, start, end);
                    self.register_linewise = self.mode == Mode::VisualLine;
                    *cursor = start;
                }
                self.mode = Mode::Normal;
                self.visual_anchor = None;
                self.reset_transient();
                true
            }
            _ => {
                let count = self.take_count();
                let chars: Vec<char> = text.chars().collect();
                if let Some(new_cur) = self.apply_motion(key, &chars, *cursor, count) {
                    *cursor = new_cur;
                    return true;
                }
                false
            }
        }
    }

    fn take_count(&mut self) -> usize {
        self.count.take().unwrap_or(1).max(1)
    }

    fn handle_normal(&mut self, key: VimKey, text: &mut String, cursor: &mut usize) -> bool {
        // Handle pending `r` (replace single character)
        if self.pending_op == Some('r') {
            self.pending_op = None;
            let count = self.take_count();
            if let VimKey::Char(ch) = key {
                let mut chars: Vec<char> = text.chars().collect();
                if *cursor + count <= chars.len() {
                    self.push_undo(text, *cursor);
                    for i in 0..count {
                        if chars[*cursor + i] != '\n' {
                            chars[*cursor + i] = ch;
                        }
                    }
                    *text = chars.into_iter().collect();
                    *cursor = (*cursor + count - 1).min(text.chars().count());
                }
            }
            return true;
        }

        // Handle pending `g` (`gg`)
        if self.pending_op == Some('g') {
            self.pending_op = None;
            let count = self.take_count();
            if key == VimKey::Char('g') {
                let chars: Vec<char> = text.chars().collect();
                *cursor = goto_line(&chars, count.saturating_sub(1));
                self.preferred_col = None;
            }
            return true;
        }

        // Handle pending operator (`d`, `c`, `y`)
        if let Some(op) = self.pending_op {
            if let VimKey::Char(c) = key
                && c.is_ascii_digit()
                && (c != '0' || self.count.is_some())
            {
                let d = c.to_digit(10).unwrap_or(0) as usize;
                self.count = Some(self.count.unwrap_or(0).saturating_mul(10).saturating_add(d));
                return true;
            }
            self.pending_op = None;
            let count = self.take_count();
            return self.apply_operator_motion(op, key, text, cursor, count);
        }

        // Numeric count prefix
        if let VimKey::Char(c) = key
            && c.is_ascii_digit()
            && (c != '0' || self.count.is_some())
        {
            let d = c.to_digit(10).unwrap_or(0) as usize;
            self.count = Some(self.count.unwrap_or(0).saturating_mul(10).saturating_add(d));
            return true;
        }

        let chars: Vec<char> = text.chars().collect();
        match key {
            VimKey::Esc => {
                self.reset_transient();
                true
            }
            VimKey::Char('i') => {
                self.push_undo(text, *cursor);
                self.reset_transient();
                self.mode = Mode::Insert;
                true
            }
            VimKey::Char('I') => {
                self.push_undo(text, *cursor);
                self.reset_transient();
                *cursor = first_non_blank_on_line(&chars, *cursor);
                self.mode = Mode::Insert;
                true
            }
            VimKey::Char('a') => {
                self.push_undo(text, *cursor);
                self.reset_transient();
                if *cursor < chars.len() && chars[*cursor] != '\n' {
                    *cursor += 1;
                }
                self.mode = Mode::Insert;
                true
            }
            VimKey::Char('A') => {
                self.push_undo(text, *cursor);
                self.reset_transient();
                *cursor = line_end_char(&chars, *cursor);
                self.mode = Mode::Insert;
                true
            }
            VimKey::Char('o') => {
                self.push_undo(text, *cursor);
                self.reset_transient();
                let end = line_end_char(&chars, *cursor);
                insert_char_at(text, end, '\n');
                *cursor = end + 1;
                self.mode = Mode::Insert;
                true
            }
            VimKey::Char('O') => {
                self.push_undo(text, *cursor);
                self.reset_transient();
                let start = line_start_char(&chars, *cursor);
                insert_char_at(text, start, '\n');
                *cursor = start;
                self.mode = Mode::Insert;
                true
            }
            VimKey::Char('v') => {
                self.reset_transient();
                self.visual_anchor = Some(*cursor);
                self.mode = Mode::Visual;
                true
            }
            VimKey::Char('V') => {
                self.reset_transient();
                self.visual_anchor = Some(*cursor);
                self.mode = Mode::VisualLine;
                true
            }
            VimKey::Char('d') | VimKey::Char('c') | VimKey::Char('y') => {
                let VimKey::Char(op) = key else {
                    unreachable!()
                };
                self.pending_op = Some(op);
                true
            }
            VimKey::Char('g') | VimKey::Char('r') => {
                let VimKey::Char(op) = key else {
                    unreachable!()
                };
                self.pending_op = Some(op);
                true
            }
            VimKey::Char('D') => {
                self.reset_transient();
                self.apply_operator_motion('d', VimKey::Char('$'), text, cursor, 1)
            }
            VimKey::Char('C') => {
                self.reset_transient();
                self.apply_operator_motion('c', VimKey::Char('$'), text, cursor, 1)
            }
            VimKey::Char('x') | VimKey::Delete => {
                let count = self.take_count();
                if *cursor < chars.len() && chars[*cursor] != '\n' {
                    self.push_undo(text, *cursor);
                    let end = (*cursor + count).min(line_end_char(&chars, *cursor));
                    self.register = slice_chars(text, *cursor, end);
                    self.register_linewise = false;
                    remove_char_range(text, *cursor, end);
                    *cursor = (*cursor).min(text.chars().count());
                }
                true
            }
            VimKey::Char('X') => {
                let count = self.take_count();
                let line_start = line_start_char(&chars, *cursor);
                if *cursor > line_start {
                    self.push_undo(text, *cursor);
                    let start = (*cursor).saturating_sub(count).max(line_start);
                    self.register = slice_chars(text, start, *cursor);
                    self.register_linewise = false;
                    remove_char_range(text, start, *cursor);
                    *cursor = start;
                }
                true
            }
            VimKey::Char('p') => {
                let count = self.take_count();
                if !self.register.is_empty() {
                    self.push_undo(text, *cursor);
                    for _ in 0..count {
                        let chars_now: Vec<char> = text.chars().collect();
                        if self.register_linewise {
                            let end = line_end_char(&chars_now, *cursor);
                            let mut to_insert = String::new();
                            if end == chars_now.len() {
                                to_insert.push('\n');
                                to_insert.push_str(self.register.trim_end_matches('\n'));
                                insert_str_at(text, end, &to_insert);
                                *cursor = end + 1;
                            } else {
                                to_insert.push_str(self.register.trim_end_matches('\n'));
                                to_insert.push('\n');
                                insert_str_at(text, end + 1, &to_insert);
                                *cursor = end + 1;
                            }
                        } else {
                            let pos = if *cursor < chars_now.len() && chars_now[*cursor] != '\n' {
                                *cursor + 1
                            } else {
                                *cursor
                            };
                            insert_str_at(text, pos, &self.register);
                            *cursor = pos + self.register.chars().count().saturating_sub(1);
                        }
                    }
                }
                true
            }
            VimKey::Char('P') => {
                let count = self.take_count();
                if !self.register.is_empty() {
                    self.push_undo(text, *cursor);
                    for _ in 0..count {
                        let chars_now: Vec<char> = text.chars().collect();
                        if self.register_linewise {
                            let start = line_start_char(&chars_now, *cursor);
                            let mut to_insert = self.register.trim_end_matches('\n').to_string();
                            to_insert.push('\n');
                            insert_str_at(text, start, &to_insert);
                            *cursor = start;
                        } else {
                            insert_str_at(text, *cursor, &self.register);
                            *cursor += self.register.chars().count().saturating_sub(1);
                        }
                    }
                }
                true
            }
            VimKey::Char('J') => {
                self.reset_transient();
                let end = line_end_char(&chars, *cursor);
                if end < chars.len() && chars[end] == '\n' {
                    self.push_undo(text, *cursor);
                    let mut next_non_blank = end + 1;
                    while next_non_blank < chars.len()
                        && (chars[next_non_blank] == ' ' || chars[next_non_blank] == '\t')
                    {
                        next_non_blank += 1;
                    }
                    remove_char_range(text, end, next_non_blank);
                    insert_char_at(text, end, ' ');
                    *cursor = end;
                }
                true
            }
            VimKey::Char('u') => {
                self.reset_transient();
                if let Some((prev_text, prev_cursor)) = self.undo_stack.pop() {
                    self.redo_stack.push((text.clone(), *cursor));
                    *text = prev_text;
                    *cursor = prev_cursor.min(text.chars().count());
                }
                true
            }
            VimKey::CtrlR => {
                self.reset_transient();
                if let Some((next_text, next_cursor)) = self.redo_stack.pop() {
                    self.undo_stack.push((text.clone(), *cursor));
                    *text = next_text;
                    *cursor = next_cursor.min(text.chars().count());
                }
                true
            }
            VimKey::Char('G') => {
                let count_opt = self.count.take();
                *cursor = match count_opt {
                    Some(line_1based) => goto_line(&chars, line_1based.saturating_sub(1)),
                    None => {
                        let last = chars.len().saturating_sub(1);
                        line_start_char(&chars, last)
                    }
                };
                self.preferred_col = None;
                true
            }
            _ => {
                let count = self.take_count();
                if let Some(new_cur) = self.apply_motion(key, &chars, *cursor, count) {
                    *cursor = new_cur;
                    return true;
                }
                true
            }
        }
    }

    fn apply_operator_motion(
        &mut self,
        op: char,
        key: VimKey,
        text: &mut String,
        cursor: &mut usize,
        count: usize,
    ) -> bool {
        let chars: Vec<char> = text.chars().collect();
        let linewise = key == VimKey::Char(op);
        let range = if linewise {
            let start = line_start_char(&chars, *cursor);
            let mut end = start;
            for i in 0..count {
                end = line_end_char(&chars, end);
                if i + 1 < count && end < chars.len() && chars[end] == '\n' {
                    end += 1;
                }
            }
            if end < chars.len() && chars[end] == '\n' {
                end += 1;
            }
            Some((start, end))
        } else if let Some(target) = self.apply_motion(key, &chars, *cursor, count) {
            let (lo, mut hi) = if target >= *cursor {
                (*cursor, target)
            } else {
                (target, *cursor)
            };
            if matches!(key, VimKey::Char('$') | VimKey::Char('e')) && hi < chars.len() {
                hi += 1;
            }
            Some((lo, hi.min(chars.len())))
        } else {
            None
        };

        if let Some((start, end)) = range
            && start < end
        {
            self.register = slice_chars(text, start, end);
            self.register_linewise = linewise;
            if op == 'y' {
                *cursor = start;
            } else {
                self.push_undo(text, *cursor);
                remove_char_range(text, start, end);
                *cursor = start.min(text.chars().count());
                if op == 'c' {
                    if linewise {
                        insert_char_at(text, *cursor, '\n');
                    }
                    self.mode = Mode::Insert;
                }
            }
        } else if op == 'c' && linewise {
            self.push_undo(text, *cursor);
            self.mode = Mode::Insert;
        }
        true
    }

    fn apply_motion(
        &mut self,
        key: VimKey,
        chars: &[char],
        cursor: usize,
        count: usize,
    ) -> Option<usize> {
        let cur = cursor.min(chars.len());
        match key {
            VimKey::Char('h') | VimKey::Left | VimKey::Backspace => {
                self.preferred_col = None;
                let line_start = line_start_char(chars, cur);
                Some(cur.saturating_sub(count).max(line_start))
            }
            VimKey::Char('l') | VimKey::Right => {
                self.preferred_col = None;
                let line_end = line_end_char(chars, cur);
                Some((cur + count).min(line_end))
            }
            VimKey::Char('0') => {
                self.preferred_col = None;
                Some(line_start_char(chars, cur))
            }
            VimKey::Char('^') => {
                self.preferred_col = None;
                Some(first_non_blank_on_line(chars, cur))
            }
            VimKey::Char('$') => {
                self.preferred_col = None;
                let end = line_end_char(chars, cur);
                let start = line_start_char(chars, cur);
                if end > start {
                    Some(end - 1)
                } else {
                    Some(end)
                }
            }
            VimKey::Char('j') | VimKey::Down | VimKey::Enter => Some(move_vertical(
                chars,
                cur,
                count as isize,
                &mut self.preferred_col,
            )),
            VimKey::Char('k') | VimKey::Up => Some(move_vertical(
                chars,
                cur,
                -(count as isize),
                &mut self.preferred_col,
            )),
            VimKey::CtrlD => Some(move_vertical(
                chars,
                cur,
                10 * count as isize,
                &mut self.preferred_col,
            )),
            VimKey::CtrlU => Some(move_vertical(
                chars,
                cur,
                -(10 * count as isize),
                &mut self.preferred_col,
            )),
            VimKey::Char('w') => {
                self.preferred_col = None;
                let mut pos = cur;
                for _ in 0..count {
                    pos = next_word_start(chars, pos);
                }
                Some(pos)
            }
            VimKey::Char('b') => {
                self.preferred_col = None;
                let mut pos = cur;
                for _ in 0..count {
                    pos = prev_word_start(chars, pos);
                }
                Some(pos)
            }
            VimKey::Char('e') => {
                self.preferred_col = None;
                let mut pos = cur;
                for _ in 0..count {
                    pos = next_word_end(chars, pos);
                }
                Some(pos)
            }
            _ => None,
        }
    }
}

fn line_start_char(chars: &[char], pos: usize) -> usize {
    let mut i = pos.min(chars.len());
    while i > 0 && chars[i - 1] != '\n' {
        i -= 1;
    }
    i
}

fn line_end_char(chars: &[char], pos: usize) -> usize {
    let mut i = pos.min(chars.len());
    while i < chars.len() && chars[i] != '\n' {
        i += 1;
    }
    i
}

fn first_non_blank_on_line(chars: &[char], pos: usize) -> usize {
    let start = line_start_char(chars, pos);
    let end = line_end_char(chars, pos);
    let mut i = start;
    while i < end && (chars[i] == ' ' || chars[i] == '\t') {
        i += 1;
    }
    i
}

fn goto_line(chars: &[char], target_line: usize) -> usize {
    let mut line = 0usize;
    let mut i = 0usize;
    while i < chars.len() && line < target_line {
        if chars[i] == '\n' {
            line += 1;
        }
        i += 1;
    }
    i
}

fn move_vertical(
    chars: &[char],
    pos: usize,
    delta: isize,
    preferred_col: &mut Option<usize>,
) -> usize {
    let start = line_start_char(chars, pos);
    let col = *preferred_col.get_or_insert(pos - start);

    let mut cur_line_start = start;
    if delta > 0 {
        for _ in 0..delta {
            let end = line_end_char(chars, cur_line_start);
            if end >= chars.len() {
                break;
            }
            cur_line_start = end + 1;
        }
    } else if delta < 0 {
        for _ in 0..(-delta) {
            if cur_line_start == 0 {
                break;
            }
            cur_line_start = line_start_char(chars, cur_line_start - 1);
        }
    }

    let target_end = line_end_char(chars, cur_line_start);
    let line_len = target_end - cur_line_start;
    if line_len == 0 {
        cur_line_start
    } else {
        cur_line_start + col.min(line_len - 1)
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn next_word_start(chars: &[char], pos: usize) -> usize {
    let len = chars.len();
    let mut i = pos.min(len);
    if i >= len {
        return len;
    }
    if is_word_char(chars[i]) {
        while i < len && is_word_char(chars[i]) {
            i += 1;
        }
    } else if !chars[i].is_whitespace() {
        while i < len && !is_word_char(chars[i]) && !chars[i].is_whitespace() {
            i += 1;
        }
    }
    while i < len && chars[i].is_whitespace() {
        i += 1;
    }
    i
}

fn prev_word_start(chars: &[char], pos: usize) -> usize {
    let mut i = pos.min(chars.len());
    if i == 0 {
        return 0;
    }
    i -= 1;
    while i > 0 && chars[i].is_whitespace() {
        i -= 1;
    }
    if is_word_char(chars[i]) {
        while i > 0 && is_word_char(chars[i - 1]) {
            i -= 1;
        }
    } else if !chars[i].is_whitespace() {
        while i > 0 && !is_word_char(chars[i - 1]) && !chars[i - 1].is_whitespace() {
            i -= 1;
        }
    }
    i
}

fn next_word_end(chars: &[char], pos: usize) -> usize {
    let len = chars.len();
    if len == 0 {
        return 0;
    }
    let mut i = (pos + 1).min(len - 1);
    while i < len && chars[i].is_whitespace() {
        i += 1;
    }
    if i >= len {
        return len - 1;
    }
    if is_word_char(chars[i]) {
        while i + 1 < len && is_word_char(chars[i + 1]) {
            i += 1;
        }
    } else {
        while i + 1 < len && !is_word_char(chars[i + 1]) && !chars[i + 1].is_whitespace() {
            i += 1;
        }
    }
    i
}

fn char_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

fn slice_chars(s: &str, start: usize, end: usize) -> String {
    let b0 = char_to_byte(s, start);
    let b1 = char_to_byte(s, end);
    s[b0..b1].to_string()
}

fn remove_char_range(s: &mut String, start: usize, end: usize) {
    let b0 = char_to_byte(s, start);
    let b1 = char_to_byte(s, end);
    if b0 < b1 {
        s.replace_range(b0..b1, "");
    }
}

fn insert_char_at(s: &mut String, char_idx: usize, ch: char) {
    let b = char_to_byte(s, char_idx);
    s.insert(b, ch);
}

fn insert_str_at(s: &mut String, char_idx: usize, ins: &str) {
    let b = char_to_byte(s, char_idx);
    s.insert_str(b, ins);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_keys(keys: &[VimKey], initial: &str, start_cur: usize) -> (VimState, String, usize) {
        let mut st = VimState::default();
        let mut text = initial.to_string();
        let mut cur = start_cur;
        for &k in keys {
            st.handle_key(k, &mut text, &mut cur);
        }
        (st, text, cur)
    }

    #[test]
    fn test_basic_hl_motions() {
        let (_, _, cur) = run_keys(
            &[VimKey::Char('l'), VimKey::Char('l'), VimKey::Char('h')],
            "SELECT",
            0,
        );
        assert_eq!(cur, 1);
    }

    #[test]
    fn test_count_prefix_motion() {
        let (_, _, cur) = run_keys(&[VimKey::Char('3'), VimKey::Char('l')], "SELECT *", 0);
        assert_eq!(cur, 3);
    }

    #[test]
    fn test_zero_caret_dollar() {
        let (_, _, c0) = run_keys(&[VimKey::Char('$')], "  SELECT 1", 0);
        assert_eq!(c0, 9);
        let (_, _, c1) = run_keys(&[VimKey::Char('$'), VimKey::Char('^')], "  SELECT 1", 0);
        assert_eq!(c1, 2);
        let (_, _, c2) = run_keys(&[VimKey::Char('$'), VimKey::Char('0')], "  SELECT 1", 0);
        assert_eq!(c2, 0);
    }

    #[test]
    fn test_jk_vertical_motions() {
        let (_, _, c1) = run_keys(&[VimKey::Char('j')], "SELECT\nFROM users\nWHERE id = 1", 2);
        assert_eq!(c1, 9); // 'O' in FROM
        let (_, _, c2) = run_keys(
            &[VimKey::Char('2'), VimKey::Char('j'), VimKey::Char('k')],
            "SELECT\nFROM users\nWHERE id = 1",
            2,
        );
        assert_eq!(c2, 9);
    }

    #[test]
    fn test_word_motions_w_b_e() {
        let (_, _, c_w) = run_keys(&[VimKey::Char('w')], "SELECT id FROM users", 0);
        assert_eq!(c_w, 7);
        let (_, _, c_b) = run_keys(
            &[VimKey::Char('w'), VimKey::Char('w'), VimKey::Char('b')],
            "SELECT id FROM users",
            0,
        );
        assert_eq!(c_b, 7);
        let (_, _, c_e) = run_keys(&[VimKey::Char('e')], "SELECT id", 0);
        assert_eq!(c_e, 5);
    }

    #[test]
    fn test_gg_and_capital_g() {
        let (_, _, c_g) = run_keys(&[VimKey::Char('G')], "a\nb\nc", 0);
        assert_eq!(c_g, 4);
        let (_, _, c_gg) = run_keys(
            &[VimKey::Char('G'), VimKey::Char('g'), VimKey::Char('g')],
            "a\nb\nc",
            0,
        );
        assert_eq!(c_gg, 0);
        let (_, _, c_2g) = run_keys(&[VimKey::Char('2'), VimKey::Char('G')], "a\nb\nc", 0);
        assert_eq!(c_2g, 2);
    }

    #[test]
    fn test_insert_transitions_i_a_cap_i_cap_a_o_cap_o_esc() {
        let (st, _, cur) = run_keys(&[VimKey::Char('a')], "abc", 0);
        assert_eq!(st.mode, Mode::Insert);
        assert_eq!(cur, 1);

        let (st2, _, cur2) = run_keys(&[VimKey::Char('A')], "abc\ndef", 0);
        assert_eq!(st2.mode, Mode::Insert);
        assert_eq!(cur2, 3);

        let (st3, txt3, cur3) = run_keys(&[VimKey::Char('o')], "abc", 0);
        assert_eq!(st3.mode, Mode::Insert);
        assert_eq!(txt3, "abc\n");
        assert_eq!(cur3, 4);

        let (st4, txt4, cur4) = run_keys(&[VimKey::Char('O')], "abc", 1);
        assert_eq!(st4.mode, Mode::Insert);
        assert_eq!(txt4, "\nabc");
        assert_eq!(cur4, 0);

        let (st5, _, _) = run_keys(&[VimKey::Char('i'), VimKey::Esc], "abc", 0);
        assert_eq!(st5.mode, Mode::Normal);
    }

    #[test]
    fn test_x_and_r_replace() {
        let (_, txt, cur) = run_keys(&[VimKey::Char('x')], "abc", 1);
        assert_eq!(txt, "ac");
        assert_eq!(cur, 1);

        let (_, txt2, _) = run_keys(&[VimKey::Char('r'), VimKey::Char('Z')], "abc", 1);
        assert_eq!(txt2, "aZc");
    }

    #[test]
    fn test_dd_yy_p_and_undo_redo() {
        let (st, txt, _) = run_keys(&[VimKey::Char('d'), VimKey::Char('d')], "line1\nline2\n", 0);
        assert_eq!(txt, "line2\n");
        assert_eq!(st.register, "line1\n");

        let (_, txt_u, _) = run_keys(
            &[VimKey::Char('d'), VimKey::Char('d'), VimKey::Char('u')],
            "line1\nline2\n",
            0,
        );
        assert_eq!(txt_u, "line1\nline2\n");

        let (_, txt_r, _) = run_keys(
            &[
                VimKey::Char('d'),
                VimKey::Char('d'),
                VimKey::Char('u'),
                VimKey::CtrlR,
            ],
            "line1\nline2\n",
            0,
        );
        assert_eq!(txt_r, "line2\n");

        let (_, txt_yp, _) = run_keys(
            &[VimKey::Char('y'), VimKey::Char('y'), VimKey::Char('p')],
            "line1\nline2",
            0,
        );
        assert_eq!(txt_yp, "line1\nline1\nline2");
    }

    #[test]
    fn test_dw_and_cw_and_d_dollar() {
        let (_, txt, _) = run_keys(&[VimKey::Char('d'), VimKey::Char('w')], "SELECT id", 0);
        assert_eq!(txt, "id");

        let (st, txt2, _) = run_keys(&[VimKey::Char('c'), VimKey::Char('w')], "SELECT id", 0);
        assert_eq!(txt2, "id");
        assert_eq!(st.mode, Mode::Insert);

        let (_, txt3, _) = run_keys(&[VimKey::Char('D')], "SELECT id", 6);
        assert_eq!(txt3, "SELECT");
    }

    #[test]
    fn test_visual_and_visual_line_delete() {
        let (st, txt, _) = run_keys(
            &[
                VimKey::Char('v'),
                VimKey::Char('l'),
                VimKey::Char('l'),
                VimKey::Char('d'),
            ],
            "abcdef",
            1,
        );
        assert_eq!(st.mode, Mode::Normal);
        assert_eq!(txt, "aef");

        let (_, txt2, _) = run_keys(&[VimKey::Char('V'), VimKey::Char('d')], "abc\ndef", 0);
        assert_eq!(txt2, "def");
    }

    #[test]
    fn test_utf8_safety_and_join_lines() {
        let (_, txt, _) = run_keys(&[VimKey::Char('x')], "áéí", 1);
        assert_eq!(txt, "áí");

        let (_, txt_j, _) = run_keys(&[VimKey::Char('J')], "SELECT *\n  FROM users", 0);
        assert_eq!(txt_j, "SELECT * FROM users");
    }
}
