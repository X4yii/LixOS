pub const CURSOR_COLOR: u32 = 0x007DCFFF;

pub struct SoftwareCursor {
    pub col: usize,
    pub row: usize,
    pub visible: bool,
    pub blink_state: bool,
    pub last_drawn_col: usize,
    pub last_drawn_row: usize,
    pub is_drawn: bool,
    pub primary_saved_col: usize,
    pub primary_saved_row: usize,
}

impl SoftwareCursor {
    pub const fn new() -> Self {
        Self {
            col: 0,
            row: 0,
            visible: true,
            blink_state: true,
            last_drawn_col: 0,
            last_drawn_row: 0,
            is_drawn: false,
            primary_saved_col: 0,
            primary_saved_row: 0,
        }
    }

    pub fn save_primary(&mut self) {
        self.primary_saved_col = self.col;
        self.primary_saved_row = self.row;
    }

    pub fn restore_primary(&mut self, max_cols: usize, max_rows: usize) {
        self.col = self.primary_saved_col.min(max_cols.saturating_sub(1));
        self.row = self.primary_saved_row.min(max_rows.saturating_sub(1));
    }
}
