pub const MAX_COLS: usize = 160;
pub const MAX_ROWS: usize = 60;

#[derive(Clone, Copy)]
pub struct TextCell {
    pub ch: char,
    pub fg: u32,
    pub bg: u32,
}

impl TextCell {
    pub const fn empty(bg: u32) -> Self {
        Self { ch: ' ', fg: 0x00C0CAF5, bg }
    }
}

static mut PRIMARY_GRID: [TextCell; MAX_COLS * MAX_ROWS] = [TextCell::empty(0x001A1B26); MAX_COLS * MAX_ROWS];
static mut ALT_GRID: [TextCell; MAX_COLS * MAX_ROWS] = [TextCell::empty(0x001A1B26); MAX_COLS * MAX_ROWS];
/// Copia temporal usada durante el reflow.
static mut REFLOW_SCRATCH: [TextCell; MAX_COLS * MAX_ROWS] = [TextCell::empty(0x001A1B26); MAX_COLS * MAX_ROWS];
/// `true` si la fila continúa en la siguiente (soft-wrap), necesario para el reflow.
static mut PRIMARY_WRAP: [bool; MAX_ROWS] = [false; MAX_ROWS];
static mut ALT_WRAP: [bool; MAX_ROWS] = [false; MAX_ROWS];

pub struct TextGridManager {
    pub is_alt_screen: bool,
}

impl TextGridManager {
    pub const fn new() -> Self {
        Self { is_alt_screen: false }
    }

    #[inline(always)]
    pub fn active_grid(&self) -> &[TextCell; MAX_COLS * MAX_ROWS] {
        unsafe {
            if self.is_alt_screen {
                &*core::ptr::addr_of!(ALT_GRID)
            } else {
                &*core::ptr::addr_of!(PRIMARY_GRID)
            }
        }
    }

    #[inline(always)]
    pub fn active_grid_mut(&mut self) -> &mut [TextCell; MAX_COLS * MAX_ROWS] {
        unsafe {
            if self.is_alt_screen {
                &mut *core::ptr::addr_of_mut!(ALT_GRID)
            } else {
                &mut *core::ptr::addr_of_mut!(PRIMARY_GRID)
            }
        }
    }

    #[inline(always)]
    fn active_wrap_mut(&mut self) -> &mut [bool; MAX_ROWS] {
        unsafe {
            if self.is_alt_screen {
                &mut *core::ptr::addr_of_mut!(ALT_WRAP)
            } else {
                &mut *core::ptr::addr_of_mut!(PRIMARY_WRAP)
            }
        }
    }

    #[inline(always)]
    pub fn set_wrap(&mut self, row: usize, wrapped: bool) {
        if row < MAX_ROWS {
            self.active_wrap_mut()[row] = wrapped;
        }
    }

    #[inline(always)]
    pub fn get_cell(&self, col: usize, row: usize) -> TextCell {
        if col < MAX_COLS && row < MAX_ROWS {
            self.active_grid()[row * MAX_COLS + col]
        } else {
            TextCell::empty(0)
        }
    }

    #[inline(always)]
    pub fn set_cell(&mut self, col: usize, row: usize, cell: TextCell) {
        if col < MAX_COLS && row < MAX_ROWS {
            self.active_grid_mut()[row * MAX_COLS + col] = cell;
        }
    }

    pub fn scroll_up(&mut self, rows: usize, fill_bg: u32) {
        let rows = rows.min(MAX_ROWS);
        if rows == 0 {
            return;
        }
        {
            let grid = self.active_grid_mut();
            grid.copy_within(MAX_COLS..rows * MAX_COLS, 0);
            for c in 0..MAX_COLS {
                grid[(rows - 1) * MAX_COLS + c] = TextCell::empty(fill_bg);
            }
        }
        let wrap = self.active_wrap_mut();
        wrap.copy_within(1..rows, 0);
        wrap[rows - 1] = false;
    }

    pub fn clear(&mut self, fill_bg: u32) {
        for cell in self.active_grid_mut().iter_mut() {
            *cell = TextCell::empty(fill_bg);
        }
        self.active_wrap_mut().fill(false);
    }

    pub fn clear_line(&mut self, row: usize, start_col: usize, end_col: usize, fill_bg: u32) {
        for c in start_col..end_col.min(MAX_COLS) {
            self.set_cell(c, row, TextCell::empty(fill_bg));
        }
    }

    /// Reorganiza (reflow) la pantalla primaria de `old_cols` a `new_cols`/`new_rows`
    /// uniendo las líneas con soft-wrap y volviéndolas a partir. Devuelve la nueva
    /// posición del cursor. Si el contenido excede `new_rows`, las líneas superiores
    /// se descartan (como un scroll).
    pub fn reflow_primary(
        &mut self,
        old_cols: usize,
        old_rows: usize,
        new_cols: usize,
        new_rows: usize,
        cur_col: usize,
        cur_row: usize,
        bg: u32,
    ) -> (usize, usize) {
        let old_cols = old_cols.min(MAX_COLS);
        let old_rows = old_rows.min(MAX_ROWS);
        let new_cols = new_cols.clamp(1, MAX_COLS);
        let new_rows = new_rows.clamp(1, MAX_ROWS);
        if old_rows == 0 {
            return (0, 0);
        }

        let (grid, wrap, src) = unsafe {
            (
                &mut *core::ptr::addr_of_mut!(PRIMARY_GRID),
                &mut *core::ptr::addr_of_mut!(PRIMARY_WRAP),
                &mut *core::ptr::addr_of_mut!(REFLOW_SCRATCH),
            )
        };
        src.copy_from_slice(&grid[..]);
        let mut src_wrap = [false; MAX_ROWS];
        src_wrap.copy_from_slice(&wrap[..]);

        for cell in grid.iter_mut() {
            *cell = TextCell::empty(bg);
        }
        wrap.fill(false);

        let is_blank = |cell: &TextCell| cell.ch == ' ' && cell.bg == bg;

        let cur_row = cur_row.min(old_rows - 1);
        let mut last = cur_row;
        for r in (cur_row + 1)..old_rows {
            if (0..old_cols).any(|c| !is_blank(&src[r * MAX_COLS + c])) {
                last = r;
            }
        }

        let mut dc = 0usize;
        let mut dr = 0usize;
        let mut new_cur = (0usize, 0usize);
        let mut cur_set = false;

        for r in 0..=last {
            let wrapped = src_wrap[r];
            let mut len = old_cols;
            if !wrapped {
                while len > 0 && is_blank(&src[r * MAX_COLS + len - 1]) {
                    len -= 1;
                }
            }

            for c in 0..len {
                if dc >= new_cols {
                    wrap[dr] = true;
                    dc = 0;
                    dr += 1;
                }
                if dr >= new_rows {
                    Self::shift_dest(grid, wrap, new_rows, bg);
                    dr = new_rows - 1;
                    if cur_set {
                        new_cur.1 = new_cur.1.saturating_sub(1);
                    }
                }
                if r == cur_row && c == cur_col {
                    new_cur = (dc, dr);
                    cur_set = true;
                }
                grid[dr * MAX_COLS + dc] = src[r * MAX_COLS + c];
                dc += 1;
            }

            if r == cur_row && cur_col >= len {
                let extra = cur_col - len;
                new_cur = ((dc + extra).min(new_cols.saturating_sub(1)), dr);
                cur_set = true;
            }

            if r < last && !wrapped {
                dc = 0;
                dr += 1;
                if dr >= new_rows {
                    Self::shift_dest(grid, wrap, new_rows, bg);
                    dr = new_rows - 1;
                    if cur_set {
                        new_cur.1 = new_cur.1.saturating_sub(1);
                    }
                }
            }
        }

        if !cur_set {
            new_cur = (dc.min(new_cols.saturating_sub(1)), dr);
        }
        (new_cur.0, new_cur.1.min(new_rows.saturating_sub(1)))
    }

    fn shift_dest(
        grid: &mut [TextCell; MAX_COLS * MAX_ROWS],
        wrap: &mut [bool; MAX_ROWS],
        rows: usize,
        bg: u32,
    ) {
        grid.copy_within(MAX_COLS..rows * MAX_COLS, 0);
        for c in 0..MAX_COLS {
            grid[(rows - 1) * MAX_COLS + c] = TextCell::empty(bg);
        }
        wrap.copy_within(1..rows, 0);
        wrap[rows - 1] = false;
    }
}
