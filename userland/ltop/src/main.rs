#![no_std]
#![no_main]

extern crate alloc;

pub mod tui_backend;

use sys_libc::syscalls::{sys_write, sys_sleep, sys_exit, sys_process_list, ProcessInfo, sys_read, sys_kill};
use core::panic::PanicInfo;
use linked_list_allocator::LockedHeap;
use core::str;

use litui::Terminal;
use litui::widgets::{Block, Borders, Paragraph, Table, Row, Cell as TuiCell, TableState};
use litui::layout::{Layout, Constraint, Direction};
use litui::style::{Style, Color, Modifier};

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

static mut HEAP: [u8; 1024 * 512] = [0; 1024 * 512]; // 512KB heap

#[no_mangle]
pub extern "C" fn _start() -> ! {
    unsafe {
        ALLOCATOR.lock().init(HEAP.as_mut_ptr(), HEAP.len());
    }

    let backend = tui_backend::LixosBackend;
    let mut terminal = match Terminal::new(backend) {
        Ok(t) => t,
        Err(_) => sys_exit(2),
    };

    let _ = terminal.clear();

    let mut buf = [ProcessInfo {
        pid: 0,
        name: [0; 16],
        state: 0,
        privilege_level: 0,
        cpu_ticks: 0,
        ram_bytes: 0,
    }; 64];

    let mut selected_idx: usize = 0;
    let mut table_state = TableState::default();

    loop {
        let count = sys_process_list(buf.as_mut_ptr(), 64);
        
        if selected_idx >= count {
            selected_idx = if count > 0 { count - 1 } else { 0 };
        }
        table_state.select(Some(selected_idx));

        let mut read_buf = [0u8; 16];
        let bytes = sys_read(0, &mut read_buf);
        if bytes > 0 {
            for i in 0..bytes {
                match read_buf[i] {
                    b'q' | b'Q' => {
                        let _ = terminal.clear();
                        sys_exit(0);
                    }
                    0x11 => { // UP
                        if selected_idx > 0 { selected_idx -= 1; }
                    }
                    0x12 => { // DOWN
                        if selected_idx + 1 < count { selected_idx += 1; }
                    }
                    b'k' | b'K' => { // KILL
                        if count > 0 {
                            let target_pid = buf[selected_idx].pid;
                            sys_kill(target_pid, 9); // SIGKILL
                        }
                    }
                    _ => {}
                }
            }
        }

        let _ = terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints([Constraint::Length(3), Constraint::Min(0)].as_ref())
                .split(f.area());

            let header = Paragraph::new("LixOS Task Manager (ltop) - Ring 3 | Keys: [UP/DOWN] Navegar, [K] Kill, [Q] Salir")
                .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
                .block(Block::default().borders(Borders::ALL));
            
            f.render_widget(header, chunks[0]);

            let mut rows = alloc::vec::Vec::new();
            
            let header_row = Row::new(alloc::vec![
                TuiCell::from("PID").style(Style::default().fg(Color::Yellow)),
                TuiCell::from("Name").style(Style::default().fg(Color::Yellow)),
                TuiCell::from("Ring").style(Style::default().fg(Color::Yellow)),
                TuiCell::from("State").style(Style::default().fg(Color::Yellow)),
                TuiCell::from("CPU Ticks").style(Style::default().fg(Color::Yellow)),
                TuiCell::from("RAM (Bytes)").style(Style::default().fg(Color::Yellow)),
            ]).style(Style::default().add_modifier(Modifier::BOLD));
            
            for i in 0..count {
                let p = &buf[i];
                let name_len = p.name.iter().position(|&c| c == 0).unwrap_or(16);
                let name = str::from_utf8(&p.name[..name_len]).unwrap_or("?");
                
                let state_str = match p.state {
                    0 => "Ready",
                    1 => "Running",
                    2 => "Blocked",
                    3 => "Sleep",
                    4 => "Zombie",
                    5 => "Dead",
                    _ => "?",
                };
                
                let ring_str = if p.privilege_level == 0 { "Ring 0" } else if p.privilege_level == 1 { "Ring 1" } else { "Ring 3" };
                
                let mut row_style = Style::default();
                if i == selected_idx {
                    row_style = row_style.bg(Color::DarkGray).fg(Color::White).add_modifier(Modifier::BOLD);
                }
                
                rows.push(Row::new(alloc::vec![
                    TuiCell::from(alloc::format!("{}", p.pid)),
                    TuiCell::from(alloc::string::String::from(name)),
                    TuiCell::from(alloc::string::String::from(ring_str)),
                    TuiCell::from(alloc::string::String::from(state_str)),
                    TuiCell::from(alloc::format!("{}", p.cpu_ticks)),
                    TuiCell::from(alloc::format!("{}", p.ram_bytes)),
                ]).style(row_style));
            }

            let table = Table::new(rows, [
                Constraint::Length(5),
                Constraint::Length(16),
                Constraint::Length(8),
                Constraint::Length(10),
                Constraint::Length(12),
                Constraint::Length(15),
            ])
            .header(header_row)
            .block(Block::default().borders(Borders::ALL).title("Active Processes"));
            
            f.render_widget(table, chunks[1]);
        });
        
        sys_sleep(100); // 10 FPS
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    sys_exit(1);
}
