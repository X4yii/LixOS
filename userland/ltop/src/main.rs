#![no_std]
#![no_main]

extern crate alloc;

pub mod tui_backend;

use sys_libc::syscalls::{sys_sleep, sys_exit, sys_process_list, ProcessInfo, SystemMetrics, sys_system_metrics, sys_read_nonblocking, sys_kill};
use core::panic::PanicInfo;
use linked_list_allocator::LockedHeap;
use core::str;

use litui::Terminal;
use litui::widgets::{Block, Borders, Paragraph, Table, Row, Cell as TuiCell};
use litui::layout::{Layout, Constraint, Direction, Rect};
use litui::style::{Style, Color, Modifier};

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

const HEAP_SIZE: usize = 1024 * 1024 * 4;
const REFRESH_INTERVAL_MS: u64 = 50;
#[repr(align(16))]
struct HeapStorage([u8; HEAP_SIZE]);
static mut HEAP: HeapStorage = HeapStorage([0; HEAP_SIZE]);

#[no_mangle]
pub extern "C" fn _start() -> ! {
    unsafe {
        let heap_ptr = core::ptr::addr_of_mut!(HEAP.0) as *mut u8;
        ALLOCATOR.lock().init(heap_ptr, HEAP_SIZE);
    }

    let backend = tui_backend::LixosBackend::new();
    let mut terminal = match Terminal::new(backend) {
        Ok(t) => t,
        Err(_) => sys_exit(2),
    };

    sys_libc::syscalls::sys_write("\x1b[?1049h\x1b[?25l\x1b[2J");
    let _ = terminal.clear();

    let empty_process = ProcessInfo {
        pid: 0,
        name: [0; 16],
        state: 0,
        privilege_level: 0,
        cpu_ticks: 0,
        ram_bytes: 0,
        disk_read_bytes: 0,
    };
    let mut processes = [empty_process; 64];
    let mut previous = [ProcessSample::default(); 64];
    let mut selected_process = 0usize;
    let mut selected_process_pid = None;
    let mut selected_section = 0usize;
    let mut sidebar_focused = true;
    let mut sort_mode = SortMode::Cpu;
    let mut pending_kill: Option<u64> = None;
    let mut esc_state = 0u8;
    let mut previous_ticks = 0u64;
    let mut previous_disk_bytes = 0u64;
    let mut cpu_usage = 0u16;
    let mut has_previous_metrics = false;
    let mut measured_tsc_ghz: Option<f64> = None;
    let mut tsc_sample = read_tsc();
    let mut tsc_elapsed_ms = 0u64;

    loop {
        let count = sys_process_list(processes.as_mut_ptr(), processes.len()).min(processes.len());
        let mut metrics = SystemMetrics::default();
        let _ = sys_system_metrics(&mut metrics);

        let tick_delta = metrics.scheduler_ticks.saturating_sub(previous_ticks);
        let elapsed_ms = tick_delta.saturating_mul(10).max(1);
        let disk_delta = metrics.disk_read_bytes.saturating_sub(previous_disk_bytes);
        let disk_read_kib_s = if has_previous_metrics { disk_delta.saturating_mul(1000) / elapsed_ms / 1024 } else { 0 };

        let mut process_rows = [ProcessRow::default(); 64];
        let mut total_process_delta = 0u64;
        for i in 0..count {
            let proc = processes[i];
            let old = previous.iter().find(|sample| sample.pid == proc.pid);
            let cpu_delta = old.map_or(0, |sample| proc.cpu_ticks.saturating_sub(sample.cpu_ticks));
            let disk_delta = old.map_or(0, |sample| proc.disk_read_bytes.saturating_sub(sample.disk_read_bytes));
            let cpu_tenths = if tick_delta == 0 { 0 } else {
                (cpu_delta.saturating_mul(1000) / tick_delta).min(1000) as u16
            };
            process_rows[i] = ProcessRow {
                info: proc,
                cpu_tenths,
                disk_kib_s: disk_delta.saturating_mul(1000) / elapsed_ms / 1024,
            };
            total_process_delta = total_process_delta.saturating_add(cpu_delta);
        }
        if tick_delta > 0 {
            cpu_usage = (total_process_delta.saturating_mul(1000) / tick_delta).min(1000) as u16;
        }
        process_rows[..count].sort_unstable_by(|a, b| sort_mode.compare(a, b));
        if let Some(pid) = selected_process_pid {
            if let Some(index) = process_rows[..count].iter().position(|row| row.info.pid == pid) {
                selected_process = index;
            }
        }
        if selected_process >= count { selected_process = count.saturating_sub(1); }

        let mut read_buf = [0u8; 16];
        let bytes = sys_read_nonblocking(0, &mut read_buf);
        if bytes > 0 {
            for i in 0..bytes {
                let b = read_buf[i];

                if esc_state == 1 {
                    if b == b'[' {
                        esc_state = 2;
                    } else {
                        esc_state = 0;
                    }
                    continue;
                } else if esc_state == 2 {
                    esc_state = 0;
                    if b == b'A' || b == b'B' {
                        let delta = if b == b'A' { -1isize } else { 1 };
                        if sidebar_focused {
                            selected_section = (selected_section as isize + delta).clamp(0, 1) as usize;
                        } else if selected_section == 0 {
                            selected_process = (selected_process as isize + delta).clamp(0, count.saturating_sub(1) as isize) as usize;
                        }
                    } else if b == b'D' {
                        sidebar_focused = true;
                    } else if b == b'C' {
                        sidebar_focused = false;
                    }
                    continue;
                }

                if b == 0x1B {
                    esc_state = 1;
                    continue;
                }

                if b == b'q' || b == b'Q' {
                    let _ = terminal.clear();
                    let _ = terminal.show_cursor();
                    let _ = terminal.flush();
                    sys_libc::syscalls::sys_write("\x1b[0m\x1b[?1049l\x1b[?25h");
                    sys_exit(0);
                } else if let Some(pid) = pending_kill {
                    if b == b'y' || b == b'Y' { sys_kill(pid, 9); }
                    pending_kill = None;
                } else if b == b'\t' {
                    sidebar_focused = !sidebar_focused;
                } else if b == b'\r' || b == b'\n' {
                    sidebar_focused = false;
                } else if b == 0x11 {
                    if sidebar_focused { selected_section = selected_section.saturating_sub(1); }
                    else if selected_section == 0 { selected_process = selected_process.saturating_sub(1); }
                } else if b == 0x12 {
                    if sidebar_focused { selected_section = (selected_section + 1).min(1); }
                    else if selected_section == 0 { selected_process = (selected_process + 1).min(count.saturating_sub(1)); }
                } else if b == 0x13 {
                    sidebar_focused = true;
                } else if (b == b's' || b == b'S') && !sidebar_focused {
                    sort_mode = sort_mode.next();
                } else if b == b'k' || b == b'K' {
                    if !sidebar_focused && selected_section == 0 && count > 0 {
                        pending_kill = Some(process_rows[selected_process].info.pid);
                    }
                }
            }
        }

        if count > 0 { selected_process_pid = Some(process_rows[selected_process].info.pid); }
        let running_count = processes[..count].iter().filter(|p| p.state == 1).count();
        let used_ram = metrics.total_ram_bytes.saturating_sub(metrics.free_ram_bytes);
        let ram_permille = if metrics.total_ram_bytes == 0 { 0 } else {
            (used_ram.saturating_mul(1000) / metrics.total_ram_bytes).min(1000) as u16
        };

        let _ = terminal.draw(|f| {
            let shell = Layout::default().direction(Direction::Vertical).constraints([
                Constraint::Length(3), Constraint::Min(0), Constraint::Length(1),
            ].as_ref()).split(f.area());
            let header_text = alloc::format!(" LixOS · Monitor del sistema     {} procesos · {} ejecutándose     Q salir ", count, running_count);
            let header = Paragraph::new(header_text)
                .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
                .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::LightBlue)));
            f.render_widget(header, shell[0]);
            let body = Layout::default().direction(Direction::Horizontal).constraints([
                Constraint::Length(20), Constraint::Min(0),
            ].as_ref()).split(shell[1]);
            let nav_style = if sidebar_focused { Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD) } else { Style::default().fg(Color::Gray) };
            let nav_text = alloc::format!("{} Procesos\n{} Rendimiento\n\nTab: cambiar foco\nEnter: abrir",
                if selected_section == 0 { ">" } else { " " },
                if selected_section == 1 { ">" } else { " " });
            f.render_widget(Paragraph::new(nav_text).style(nav_style)
                .block(Block::default().borders(Borders::ALL).title(" Secciones ").border_style(Style::default().fg(Color::Cyan))), body[0]);

            if selected_section == 0 {
                draw_processes(f, body[1], &process_rows[..count], selected_process, sort_mode, metrics.total_ram_bytes, pending_kill);
            } else {
                draw_performance(f, body[1], cpu_usage, ram_permille, used_ram, metrics, disk_read_kib_s, measured_tsc_ghz);
            }
            let help = if sidebar_focused { " ↑/↓ sección · Enter abrir · Tab panel · Q salir " }
                else if selected_section == 0 { " ↑/↓ proceso · S ordenar · K terminar · ← sidebar · Tab cambiar · Q salir " }
                else { " ← sidebar · Tab cambiar · Q salir " };
            f.render_widget(Paragraph::new(help).style(Style::default().fg(Color::Gray)), shell[2]);
        });

        previous = [ProcessSample::default(); 64];
        for i in 0..count {
            previous[i] = ProcessSample { pid: processes[i].pid, cpu_ticks: processes[i].cpu_ticks, disk_read_bytes: processes[i].disk_read_bytes };
        }
        previous_ticks = metrics.scheduler_ticks;
        previous_disk_bytes = metrics.disk_read_bytes;
        has_previous_metrics = true;

        sys_sleep(REFRESH_INTERVAL_MS);
        tsc_elapsed_ms = tsc_elapsed_ms.saturating_add(REFRESH_INTERVAL_MS);
        if tsc_elapsed_ms >= 1000 {
            let now = read_tsc();
            measured_tsc_ghz = Some(now.wrapping_sub(tsc_sample) as f64 / tsc_elapsed_ms as f64 / 1_000_000.0);
            tsc_sample = now;
            tsc_elapsed_ms = 0;
        }
    }
}

#[inline]
fn read_tsc() -> u64 {
    let low: u32;
    let high: u32;
    unsafe { core::arch::asm!("rdtsc", out("eax") low, out("edx") high, options(nomem, nostack, preserves_flags)); }
    ((high as u64) << 32) | low as u64
}

#[derive(Clone, Copy, Default)]
struct ProcessSample {
    pid: u64,
    cpu_ticks: u64,
    disk_read_bytes: u64,
}

#[derive(Clone, Copy, Default)]
struct ProcessRow {
    info: ProcessInfo,
    cpu_tenths: u16,
    disk_kib_s: u64,
}

#[derive(Clone, Copy)]
enum SortMode { NameAsc, NameDesc, Cpu, Ram, Disk }

impl SortMode {
    fn next(self) -> Self {
        match self { Self::NameAsc => Self::NameDesc, Self::NameDesc => Self::Cpu, Self::Cpu => Self::Ram, Self::Ram => Self::Disk, Self::Disk => Self::NameAsc }
    }

    fn label(self) -> &'static str {
        match self { Self::NameAsc => "A–Z", Self::NameDesc => "Z–A", Self::Cpu => "CPU ↓", Self::Ram => "RAM ↓", Self::Disk => "Disco ↓" }
    }

    fn compare(self, a: &ProcessRow, b: &ProcessRow) -> core::cmp::Ordering {
        match self {
            Self::NameAsc => process_name(&a.info).cmp(process_name(&b.info)),
            Self::NameDesc => process_name(&b.info).cmp(process_name(&a.info)),
            Self::Cpu => b.cpu_tenths.cmp(&a.cpu_tenths),
            Self::Ram => b.info.ram_bytes.cmp(&a.info.ram_bytes),
            Self::Disk => b.disk_kib_s.cmp(&a.disk_kib_s),
        }
    }
}

fn process_name(process: &ProcessInfo) -> &str {
    let len = process.name.iter().position(|&byte| byte == 0).unwrap_or(process.name.len());
    str::from_utf8(&process.name[..len]).unwrap_or("?")
}

fn draw_processes(f: &mut litui::Frame, area: Rect, rows: &[ProcessRow], selected: usize, sort: SortMode, total_ram: u64, pending_kill: Option<u64>) {
    let chunks = Layout::default().direction(Direction::Vertical).constraints([
        Constraint::Length(3), Constraint::Min(5), Constraint::Length(4),
    ].as_ref()).split(area);
    let sort_label = alloc::format!(" Orden: {}   [S] siguiente · CPU/RAM/disco por proceso ", sort.label());
    f.render_widget(Paragraph::new(sort_label).block(Block::default().borders(Borders::ALL).title(" Procesos ")), chunks[0]);

    let mut table_rows = alloc::vec::Vec::new();
    let headers = Row::new(alloc::vec!["PID", "Nombre", "CPU", "RAM", "Disco", "Anillo"])
        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
    for (index, row) in rows.iter().enumerate() {
        let selected = index == selected;
        let style = if selected { Style::default().fg(Color::White).bg(Color::Blue).add_modifier(Modifier::BOLD) } else { Style::default() };
        let ram_tenths = if total_ram == 0 { 0 } else { (row.info.ram_bytes.saturating_mul(1000) / total_ram).min(1000) as u16 };
        let ring = match row.info.privilege_level { 0 => "Ring 0", 1 => "Ring 1", _ => "Ring 3" };
        table_rows.push(Row::new(alloc::vec![
            TuiCell::from(alloc::format!("{}", row.info.pid)).style(style),
            TuiCell::from(alloc::string::String::from(process_name(&row.info))).style(style),
            TuiCell::from(alloc::format!("{}.{:01}%", row.cpu_tenths / 10, row.cpu_tenths % 10)).style(style),
            TuiCell::from(alloc::format!("{}.{:01}%", ram_tenths / 10, ram_tenths % 10)).style(style),
            TuiCell::from(alloc::format!("{} KiB/s", row.disk_kib_s)).style(style),
            TuiCell::from(ring).style(style),
        ]));
    }
    let table = Table::new(table_rows, [
        Constraint::Length(5), Constraint::Length(12), Constraint::Length(7),
        Constraint::Length(7), Constraint::Length(10), Constraint::Length(6),
    ]).header(headers).block(Block::default().borders(Borders::ALL).title(" PID · nombre · CPU · RAM · disco · anillo "));
    f.render_widget(table, chunks[1]);

    let details = if let Some(pid) = pending_kill {
        alloc::format!(" ¿Terminar PID {}? Pulsa Y para confirmar; otra tecla cancela.", pid)
    } else {
        rows.get(selected).map(|row| alloc::format!(
            " PID {} · {} · {} · RAM {} · Lectura de disco {} KiB/s · [K] terminar",
            row.info.pid, process_name(&row.info), process_state(row.info.state), format_bytes(row.info.ram_bytes), row.disk_kib_s,
        )).unwrap_or_else(|| alloc::string::String::from(" No hay procesos que mostrar "))
    };
    f.render_widget(Paragraph::new(details).block(Block::default().borders(Borders::ALL).title(" Selección ")), chunks[2]);
}

fn draw_performance(
    f: &mut litui::Frame, area: Rect, cpu_permille: u16, ram_permille: u16,
    used_ram: u64, metrics: SystemMetrics, disk_kib_s: u64, tsc_ghz: Option<f64>,
) {
    let mut text = alloc::string::String::new();
    use core::fmt::Write;
    let _ = writeln!(text, " CPU  {}.{:01}%  {}", cpu_permille / 10, cpu_permille % 10, usage_bar(cpu_permille));
    match tsc_ghz {
        Some(ghz) => { let _ = writeln!(text, " Reloj virtual TSC: {:.2} GHz (estimado)", ghz); }
        None => { let _ = writeln!(text, " Reloj virtual TSC: calibrando…"); }
    }
    let _ = writeln!(text, "\n RAM  {}.{:01}%  {}", ram_permille / 10, ram_permille % 10, usage_bar(ram_permille));
    let _ = writeln!(text, " Usada: {}   Libre: {}   Total: {}", format_bytes(used_ram), format_bytes(metrics.free_ram_bytes), format_bytes(metrics.total_ram_bytes));
    let _ = writeln!(text, "\n DISCO  {}", if metrics.disk_mounted != 0 { "ext4 montado · métricas VFS" } else { "sin volumen VFS montado" });
    let _ = writeln!(text, " Lectura: {} KiB/s · Acumulado: {} · Operaciones: {}", disk_kib_s, format_bytes(metrics.disk_read_bytes), metrics.disk_read_ops);
    let _ = writeln!(text, " Escritura: N/D (VFS todavía no tiene ruta de escritura)");
    f.render_widget(Paragraph::new(text).block(Block::default().borders(Borders::ALL).title(" Rendimiento · CPU / RAM / disco ")), area);
}

fn usage_bar(permille: u16) -> alloc::string::String {
    let filled = (permille as usize * 20 / 1000).min(20);
    let mut bar = alloc::string::String::from("[");
    for index in 0..20 { bar.push(if index < filled { '#' } else { '-' }); }
    bar.push(']');
    bar
}

fn format_bytes(bytes: u64) -> alloc::string::String {
    if bytes >= 1024 * 1024 {
        alloc::format!("{}.{:01} MiB", bytes / (1024 * 1024), (bytes % (1024 * 1024)) * 10 / (1024 * 1024))
    } else if bytes >= 1024 {
        alloc::format!("{} KiB", bytes / 1024)
    } else {
        alloc::format!("{} B", bytes)
    }
}

fn process_state(state: u8) -> &'static str {
    match state { 0 => "Ready", 1 => "Running", 2 => "Blocked", 3 => "Sleep", 4 => "Zombie", 5 => "Dead", _ => "?" }
}

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    sys_libc::println!("[ltop panic] {}", _info);
    sys_exit(1);
}
