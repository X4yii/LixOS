#![no_std]
#![no_main]
#![allow(unsafe_code)]
extern crate alloc;
mod tui;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use lix_storage::writer::BlockWriter;
use lix_storage::{build_gpt_image, format_esp, format_ext4, EspFiles, GptPartition};
use sys_libc::allocator::UserAllocator;
use sys_libc::syscalls::{
    sys_block_info, sys_block_open, sys_block_read, sys_block_sync, sys_block_write, sys_exit,
    sys_handle_close, sys_read_file_all, sys_sleep, sys_write, BlockDeviceInfo, Handle,
};
use litui::layout::{Constraint, Direction, Layout, Rect};
use litui::style::{Color, Modifier, Style};
use litui::text::Line;
use litui::widgets::{Block, Borders, Paragraph};
use litui::{Frame, Terminal};
use tui::{centered_rect, Input, Key, LixosBackend};
#[global_allocator]
static ALLOCATOR: UserAllocator = UserAllocator::new();
const INSTALL_STEPS: [&str; 5] = [
    "Abrir el dispositivo y adquirir acceso exclusivo",
    "Escribir la tabla de particiones GPT",
    "Formatear la ESP (FAT32) e instalar Limine",
    "Formatear la raíz (EXT4) y desplegar LixOS",
    "Sincronizar con el almacenamiento no volátil",
];
macro_rules! must {
    ($que:expr, $resultado:expr) => {
        match $resultado {
            Ok(valor) => valor,
            Err(error) => fatal(format!(
                "\x1b[31m[ERROR] {}: {}\x1b[0m\nInstalación detenida: el disco puede quedar incompleto.",
                $que, error
            )),
        }
    };
}
fn teardown_terminal() {
    sys_write("\x1b[0m\x1b[?1049l\x1b[?25h");
}
fn fatal(msg: String) -> ! {
    teardown_terminal();
    sys_write(&msg);
    sys_write("\n");
    sys_exit(1);
}
struct SysBlockWriter {
    handle: Handle,
    sector_size: u32,
}
impl BlockWriter for SysBlockWriter {
    fn write_at(&mut self, offset: u64, bytes: &[u8]) -> Result<(), &'static str> {
        let sector_size = self.sector_size as u64;
        let mut cur_offset = offset;
        let mut remaining = bytes;
        while !remaining.is_empty() {
            let start_lba = cur_offset / sector_size;
            let byte_in_sector = (cur_offset % sector_size) as usize;
            if byte_in_sector != 0 || remaining.len() < sector_size as usize {
                let mut sector_buf = vec![0u8; self.sector_size as usize];
                must!(
                    "lectura de sector para escritura parcial",
                    sys_block_read(self.handle, start_lba, 1, &mut sector_buf)
                );
                let copy_len =
                    core::cmp::min(remaining.len(), self.sector_size as usize - byte_in_sector);
                sector_buf[byte_in_sector..byte_in_sector + copy_len]
                    .copy_from_slice(&remaining[..copy_len]);
                sys_block_write(self.handle, start_lba, 1, &sector_buf)
                    .map_err(|_| "block write error")?;
                cur_offset += copy_len as u64;
                remaining = &remaining[copy_len..];
            } else {
                let max_transfer_blocks = 8u16;
                let max_blocks = core::cmp::min(
                    (remaining.len() / sector_size as usize) as u16,
                    max_transfer_blocks,
                );
                let write_bytes = max_blocks as usize * sector_size as usize;
                sys_block_write(
                    self.handle,
                    start_lba,
                    max_blocks,
                    &remaining[..write_bytes],
                )
                .map_err(|_| "block write error")?;
                cur_offset += write_bytes as u64;
                remaining = &remaining[write_bytes..];
            }
        }
        Ok(())
    }
    fn read_at(&self, offset: u64, out: &mut [u8]) -> Result<(), &'static str> {
        let sector_size = self.sector_size as u64;
        let start_lba = offset / sector_size;
        let byte_off = (offset % sector_size) as usize;
        if byte_off != 0 {
            return Err("offset not sector aligned");
        }
        sys_block_read(
            self.handle,
            start_lba,
            (out.len() as u32 / self.sector_size).max(1) as u16,
            out,
        )
        .map(|_| ())
        .map_err(|_| "block read error")
    }
}
struct DeviceEntry {
    index: u64,
    name: String,
    block_count: u64,
    block_size: u32,
    size_mb: u64,
    read_only: bool,
}
const MAX_LIVE_FILE_BYTES: usize = 64 * 1024 * 1024;
fn read_live_file(path: &str) -> Vec<u8> {
    let data = must!(
        format!("leer {path} del medio live"),
        sys_read_file_all(path)
    );
    if data.is_empty() {
        fatal(format!("\x1b[31m[ERROR] {path} está vacío\x1b[0m"));
    }
    if data.len() >= MAX_LIVE_FILE_BYTES {
        fatal(format!(
            "\x1b[31m[ERROR] {path} alcanzó el tope de lectura ({MAX_LIVE_FILE_BYTES} bytes): se instalaría truncado\x1b[0m"
        ));
    }
    data
}
fn device_name_str(name: &[u8; BlockDeviceInfo::NAME_LEN]) -> String {
    let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
    String::from_utf8_lossy(&name[..end]).into_owned()
}
fn probe_devices() -> Vec<DeviceEntry> {
    let mut list = Vec::new();
    for i in 0..8 {
        if let Ok(handle) = sys_block_open(i, false) {
            let mut info = BlockDeviceInfo::default();
            if sys_block_info(handle, &mut info).is_ok()
                && info.block_count > 0
                && info.block_size > 0
            {
                let size_mb =
                    (info.block_count.saturating_mul(info.block_size as u64)) / (1024 * 1024);
                list.push(DeviceEntry {
                    index: i,
                    name: device_name_str(&info.name),
                    block_count: info.block_count,
                    block_size: info.block_size,
                    size_mb,
                    read_only: info.flags & BlockDeviceInfo::FLAG_READ_ONLY != 0,
                });
            }
            let _ = sys_handle_close(handle);
        }
    }
    list
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    NoDevices,
    Device,
    Mode,
    Config,
    Summary,
    Progress,
    Done,
}
struct App {
    devices: Vec<DeviceEntry>,
    device_sel: usize,
    mode_whole: bool,
    hostname: String,
    keymap: String,
    username: String,
    field: usize,
    screen: Screen,
    status: String,
    progress_step: usize,
    progress_msg: String,
    success: bool,
    quit: bool,
}
impl App {
    fn new(devices: Vec<DeviceEntry>) -> Self {
        let screen = if devices.is_empty() {
            Screen::NoDevices
        } else {
            Screen::Device
        };
        Self {
            devices,
            device_sel: 0,
            mode_whole: true,
            hostname: String::from("lixos"),
            keymap: String::from("latam"),
            username: String::from("user"),
            field: 0,
            screen,
            status: String::new(),
            progress_step: 0,
            progress_msg: String::new(),
            success: false,
            quit: false,
        }
    }
    fn active_field(&mut self) -> &mut String {
        match self.field {
            0 => &mut self.hostname,
            1 => &mut self.keymap,
            _ => &mut self.username,
        }
    }
}
fn handle_key(app: &mut App, key: Key) -> bool {
    match app.screen {
        Screen::NoDevices => {
            app.quit = true;
        }
        Screen::Device => match key {
            Key::Up => {
                if app.device_sel > 0 {
                    app.device_sel -= 1;
                }
                app.status.clear();
            }
            Key::Down => {
                if app.device_sel + 1 < app.devices.len() {
                    app.device_sel += 1;
                }
                app.status.clear();
            }
            Key::Enter => {
                if app.devices[app.device_sel].read_only {
                    app.status = format!(
                        "{} es de solo lectura (el controlador aún no soporta escritura). Elige otro disco.",
                        app.devices[app.device_sel].name
                    );
                } else {
                    app.screen = Screen::Mode;
                    app.status.clear();
                }
            }
            Key::Esc => app.quit = true,
            _ => {}
        },
        Screen::Mode => match key {
            Key::Up | Key::Down | Key::Left | Key::Right => app.mode_whole = !app.mode_whole,
            Key::Char('1') => app.mode_whole = true,
            Key::Char('2') => app.mode_whole = false,
            Key::Enter => {
                app.screen = Screen::Config;
                app.field = 0;
            }
            Key::Esc => app.screen = Screen::Device,
            _ => {}
        },
        Screen::Config => match key {
            Key::Up => app.field = (app.field + 2) % 3,
            Key::Down | Key::Tab => app.field = (app.field + 1) % 3,
            Key::Enter => app.screen = Screen::Summary,
            Key::Esc => app.screen = Screen::Mode,
            Key::Backspace => {
                app.active_field().pop();
            }
            Key::Char(c) if c.is_ascii_alphanumeric() || c == '-' || c == '_' => {
                app.active_field().push(c);
            }
            _ => {}
        },
        Screen::Summary => match key {
            Key::Enter => return true,
            Key::Esc => app.screen = Screen::Config,
            _ => {}
        },
        _ => {}
    }
    false
}
fn draw_frame(app: &App, term: &mut Terminal<LixosBackend>) {
    let _ = term.draw(|f| render(app, f));
}
fn render(app: &App, f: &mut Frame) {
    let area = f.area();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);
    let header = Paragraph::new(" LixOS Live Installer — lixinstall").style(
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );
    f.render_widget(header, rows[0]);
    match app.screen {
        Screen::NoDevices => render_no_devices(f, rows[1]),
        Screen::Device => render_device(app, f, rows[1]),
        Screen::Mode => render_mode(app, f, rows[1]),
        Screen::Config => render_config(app, f, rows[1]),
        Screen::Summary => render_summary(app, f, rows[1]),
        Screen::Progress => render_progress(app, f, rows[1]),
        Screen::Done => render_done(app, f, rows[1]),
    }
    if app.status.is_empty() {
        f.render_widget(Paragraph::new(""), rows[2]);
    } else {
        f.render_widget(
            Paragraph::new(app.status.as_str()).style(Style::default().fg(Color::Red)),
            rows[2],
        );
    }
    let footer_text = match app.screen {
        Screen::NoDevices => " Presiona cualquier tecla para salir ",
        Screen::Device => " ↑/↓ seleccionar · Enter continuar · Esc salir ",
        Screen::Mode => " ←/→/↑/↓ o 1/2 elegir · Enter continuar · Esc volver ",
        Screen::Config => " ↑/↓ cambiar campo · Escribir editar · Enter continuar · Esc volver ",
        Screen::Summary => " Enter instalar · Esc volver ",
        Screen::Progress => " Instalando… no apagues el equipo ",
        Screen::Done => " Presiona cualquier tecla para salir ",
    };
    f.render_widget(
        Paragraph::new(footer_text).style(Style::default().fg(Color::DarkGray)),
        rows[3],
    );
}
fn render_list(f: &mut Frame, area: Rect, items: &[String], selected: usize, disabled: &[bool]) {
    if items.is_empty() {
        return;
    }
    let mut constraints: Vec<Constraint> = items.iter().map(|_| Constraint::Length(1)).collect();
    constraints.push(Constraint::Min(0));
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);
    for (i, item) in items.iter().enumerate() {
        let style = if i == selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else if disabled.get(i).copied().unwrap_or(false) {
            Style::default().fg(Color::Red)
        } else {
            Style::default().fg(Color::White)
        };
        let prefix = if i == selected { "▸ " } else { "  " };
        f.render_widget(
            Paragraph::new(format!("{prefix}{item}")).style(style),
            rows[i],
        );
    }
}
fn render_no_devices(f: &mut Frame, area: Rect) {
    let rect = centered_rect(64, 7, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Sin dispositivos ")
        .border_style(Style::default().fg(Color::Red));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let lines = vec![
        Line::raw(""),
        Line::styled(
            "No se detectaron dispositivos de bloque físicos (NVMe o SATA).",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::styled(
            "Conecta un disco y vuelve a ejecutar lixinstall.",
            Style::default().fg(Color::Gray),
        ),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}
fn render_device(app: &App, f: &mut Frame, area: Rect) {
    let height = (app.devices.len() as u16) + 4;
    let rect = centered_rect(72, height, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Selecciona el disco de destino ")
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let items: Vec<String> = app
        .devices
        .iter()
        .map(|d| {
            let ro = if d.read_only { "  [solo lectura]" } else { "" };
            format!(
                "{}  —  {} MiB  ({} bloques × {} B){}",
                d.name, d.size_mb, d.block_count, d.block_size, ro
            )
        })
        .collect();
    let disabled: Vec<bool> = app.devices.iter().map(|d| d.read_only).collect();
    render_list(f, inner, &items, app.device_sel, &disabled);
}
fn render_mode(app: &App, f: &mut Frame, area: Rect) {
    let rect = centered_rect(72, 7, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Modo de particionado ")
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let items = vec![
        String::from("Disco Completo (GPT nueva: 512 MiB ESP + Resto Root EXT4)  [Recomendado]"),
        String::from("Partición Existente (Preservar particiones previas)"),
    ];
    let selected = if app.mode_whole { 0 } else { 1 };
    render_list(f, inner, &items, selected, &[]);
}
fn render_config(app: &App, f: &mut Frame, area: Rect) {
    let rect = centered_rect(72, 8, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Configuración del Sistema ")
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let labels = [
        "Nombre del equipo (Hostname)",
        "Distribución de teclado (latam, es, us, etc.)",
        "Nombre de usuario principal",
    ];
    let values = [
        app.hostname.as_str(),
        app.keymap.as_str(),
        app.username.as_str(),
    ];
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);
    for i in 0..3 {
        let active = i == app.field;
        let (prefix, style, cursor) = if active {
            (
                "▸ ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
                "▌",
            )
        } else {
            ("  ", Style::default().fg(Color::White), "")
        };
        let line = format!("{prefix}{}: {}{}", labels[i], values[i], cursor);
        f.render_widget(Paragraph::new(line).style(style), rows[i]);
    }
}
fn render_summary(app: &App, f: &mut Frame, area: Rect) {
    let rect = centered_rect(72, 12, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Confirmación de Instalación ")
        .border_style(Style::default().fg(Color::Red));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let target = &app.devices[app.device_sel];
    let mode = if app.mode_whole {
        "Disco Completo (Borrado total)"
    } else {
        "Partición Existente"
    };
    let lines = vec![
        Line::raw(""),
        Line::styled(
            format!(
                "Destino: {} (dispositivo #{}, {} MiB)",
                target.name, target.index, target.size_mb
            ),
            Style::default().fg(Color::White),
        ),
        Line::styled(format!("Modo: {mode}"), Style::default().fg(Color::White)),
        Line::styled(
            format!("Hostname: {}", app.hostname),
            Style::default().fg(Color::White),
        ),
        Line::styled(
            format!("Keymap: {}", app.keymap),
            Style::default().fg(Color::White),
        ),
        Line::styled(
            format!("Usuario: {}", app.username),
            Style::default().fg(Color::White),
        ),
        Line::raw(""),
        Line::styled(
            format!(
                "¡Atención! Se escribirá en {}. Esto puede destruir datos.",
                target.name
            ),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Line::styled(
            "¿Deseas proceder con la instalación? Pulsa Enter.",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}
fn render_progress(app: &App, f: &mut Frame, area: Rect) {
    let rect = centered_rect(72, 11, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Instalando LixOS ")
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let mut lines: Vec<Line> = Vec::new();
    for (i, step) in INSTALL_STEPS.iter().enumerate() {
        let n = i + 1;
        let (mark, style) = if n < app.progress_step {
            ("✓", Style::default().fg(Color::Green))
        } else if n == app.progress_step {
            (
                "▸",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            ("·", Style::default().fg(Color::DarkGray))
        };
        lines.push(Line::styled(format!(" [{n}/5] {mark} {step}"), style));
    }
    lines.push(Line::raw(""));
    if !app.progress_msg.is_empty() {
        lines.push(Line::styled(
            format!("  {}", app.progress_msg),
            Style::default().fg(Color::LightCyan),
        ));
    }
    f.render_widget(Paragraph::new(lines), inner);
}
fn render_done(app: &App, f: &mut Frame, area: Rect) {
    let color = if app.success {
        Color::Green
    } else {
        Color::Yellow
    };
    let title = if app.success {
        " Instalación Completada "
    } else {
        " Instalación Cancelada "
    };
    let rect = centered_rect(72, 9, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(color));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let mut lines = vec![Line::raw("")];
    if app.success {
        lines.push(Line::styled(
            "¡INSTALACIÓN COMPLETADA EXITOSAMENTE!",
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            "LixOS ha quedado instalado en el disco y es 100% booteable mediante Limine.",
            Style::default().fg(Color::White),
        ));
        lines.push(Line::styled(
            "Puedes reiniciar el equipo con 'shutdown' o retirar el medio LiveISO.",
            Style::default().fg(Color::White),
        ));
    } else {
        lines.push(Line::styled(
            "Instalación cancelada por el usuario.",
            Style::default().fg(color),
        ));
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            "No se modificó ningún disco.",
            Style::default().fg(Color::White),
        ));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "Presiona cualquier tecla para salir.",
        Style::default().fg(Color::DarkGray),
    ));
    f.render_widget(Paragraph::new(lines), inner);
}
fn run_app(app: &mut App, term: &mut Terminal<LixosBackend>) -> bool {
    let mut input = Input::new();
    let mut last_screen = app.screen;
    loop {
        let mut keys = Vec::new();
        input.drain(&mut keys);
        for key in keys {
            if handle_key(app, key) {
                return true;
            }
            if app.quit {
                return false;
            }
        }
        if app.screen != last_screen {
            let _ = term.clear();
            last_screen = app.screen;
        }
        draw_frame(app, term);
        sys_sleep(50);
    }
}
fn wait_any_key() {
    let mut input = Input::new();
    loop {
        let mut keys = Vec::new();
        input.drain(&mut keys);
        if !keys.is_empty() {
            return;
        }
        sys_sleep(50);
    }
}
fn progress(app: &mut App, term: &mut Terminal<LixosBackend>, step: usize, msg: &str) {
    app.progress_step = step;
    app.progress_msg = String::from(msg);
    draw_frame(app, term);
}
fn run_install(app: &mut App, term: &mut Terminal<LixosBackend>) {
    let target_index = app.devices[app.device_sel].index;
    let target_block_size = app.devices[app.device_sel].block_size;
    let target_block_count = app.devices[app.device_sel].block_count;
    let hostname = app.hostname.clone();
    let keymap = app.keymap.clone();
    let username = app.username.clone();
    progress(
        app,
        term,
        1,
        "Adquiriendo acceso exclusivo al dispositivo de bloque",
    );
    let handle = match sys_block_open(target_index, true) {
        Ok(h) => h,
        Err(e) => fatal(format!(
            "\x1b[31m[ERROR] Falló al abrir el dispositivo #{target_index}: código {e}\x1b[0m"
        )),
    };
    let mut writer = SysBlockWriter {
        handle,
        sector_size: target_block_size,
    };
    let disk_guid = [0x54, 0x45, 0x53, 0x54, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
    let esp_guid = [0x45, 0x53, 0x50, 0x31, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
    let root_guid = [0x52, 0x4F, 0x4F, 0x54, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    const ESP_TYPE: [u8; 16] = [
        0xAF, 0x3D, 0xC6, 0x0F, 0x83, 0x84, 0x72, 0x47, 0x8E, 0x79, 0x3D, 0x69, 0xD8, 0x47, 0x7D,
        0xE4,
    ];
    const LINUX_TYPE: [u8; 16] = [
        0xAF, 0x3D, 0xC6, 0x0F, 0x83, 0x84, 0x72, 0x47, 0x8E, 0x79, 0x3D, 0x69, 0xD8, 0x47, 0x7D,
        0xE4,
    ];
    let esp_type = ESP_TYPE;
    let linux_type = LINUX_TYPE;
    let block_size = target_block_size as u64;
    let alignment_blocks = 1024 * 1024 / block_size;
    let esp_start = alignment_blocks;
    let esp_blocks = 512 * 1024 * 1024 / block_size;
    let root_start =
        (esp_start + esp_blocks + alignment_blocks - 1) / alignment_blocks * alignment_blocks;
    let last_lba = target_block_count.saturating_sub(34);
    progress(
        app,
        term,
        2,
        "Escribiendo tabla de particiones GPT y MBR protector",
    );
    let partitions = [
        GptPartition {
            type_guid: esp_type,
            unique_guid: esp_guid,
            first_lba: esp_start,
            last_lba: root_start - 1,
            attributes: 0,
        },
        GptPartition {
            type_guid: linux_type,
            unique_guid: root_guid,
            first_lba: root_start,
            last_lba,
            attributes: 0,
        },
    ];
    if let Ok(gpt) = build_gpt_image(
        target_block_size,
        target_block_count,
        disk_guid,
        &partitions,
    ) {
        sys_write("[GPT1]");
        must!(
            "escribir MBR protectoro GPT",
            writer.write_at(0, &gpt.protective_mbr)
        );
        sys_write("[GPT2]");
        must!(
            "escribir cabecera GPT primaria",
            writer.write_at(block_size, &gpt.primary_header)
        );
        sys_write("[GPT3]");
        must!(
            "escribir entradas GPT primarias",
            writer.write_at(2 * block_size, &gpt.primary_entries)
        );
        sys_write("[GPT4]");
        must!(
            "escribir entradas GPT de respaldo",
            writer.write_at(gpt.backup_entries_lba * block_size, &gpt.backup_entries)
        );
        sys_write("[GPT5]");
        must!(
            "escribir cabecera GPT de respaldo",
            writer.write_at((target_block_count - 1) * block_size, &gpt.backup_header)
        );
        sys_write("[GPT6]");
    } else {
        let _ = sys_handle_close(handle);
        fatal(String::from(
            "\x1b[31m[ERROR] Falló al generar la tabla GPT.\x1b[0m",
        ));
    }
    progress(
        app,
        term,
        3,
        "Formateando partición EFI (FAT32) e instalando Limine",
    );
    let kernel_data = read_live_file("/boot/kernel.elf");
    let bootloader_data = read_live_file("/boot/BOOTX64.EFI");
    let limine_cfg = b"timeout: 3\n\n/LixOS\n    protocol: limine\n    kernel_path: guid(524F4F54-0000-0000-0000-000000000001):/boot/kernel.elf\n    cmdline: root=PARTUUID=524F4F54-0000-0000-0000-000000000001\n";
    let esp_files = EspFiles {
        bootloader: &bootloader_data,
        config: limine_cfg,
        kernel: &kernel_data,
        initrd: &[],
    };
    let esp_offset = esp_start * block_size;
    let esp_size_bytes = (root_start - esp_start) * block_size;
    if let Err(e) = format_esp(
        &mut writer,
        target_block_size,
        esp_offset,
        esp_size_bytes,
        esp_start as u32,
        0x12345678,
        esp_files,
    ) {
        let _ = sys_handle_close(handle);
        fatal(format!(
            "\x1b[31m[ERROR] Falló al formatear ESP FAT32: {}\x1b[0m",
            e
        ));
    }
    progress(
        app,
        term,
        4,
        "Formateando partición raíz (EXT4) y desplegando LixOS",
    );
    let root_offset = root_start * block_size;
    let root_blocks_4k = (last_lba - root_start + 1) * block_size / 4096;
    let mut ext4_vol = match format_ext4(
        &mut writer,
        root_offset,
        root_blocks_4k,
        "LIXOS_ROOT",
        root_guid,
    ) {
        Ok(v) => v,
        Err(e) => {
            let _ = sys_handle_close(handle);
            fatal(format!(
                "\x1b[31m[ERROR] Falló al formatear partición EXT4: {}\x1b[0m",
                e
            ));
        }
    };
    let root_dir_block = ext4_vol.root_dir_block;
    let (_, boot_block) = must!(
        "crear /boot en EXT4",
        ext4_vol.mkdir(&mut writer, root_dir_block, 2, "boot")
    );
    let (_, etc_block) = must!(
        "crear /etc en EXT4",
        ext4_vol.mkdir(&mut writer, root_dir_block, 2, "etc")
    );
    let (bin_ino, bin_block) = must!(
        "crear /bin en EXT4",
        ext4_vol.mkdir(&mut writer, root_dir_block, 2, "bin")
    );
    let (_, bin_system_block) = must!(
        "crear /bin/system en EXT4",
        ext4_vol.mkdir(&mut writer, bin_block, bin_ino, "system")
    );
    let (_, bin_apps_block) = must!(
        "crear /bin/apps en EXT4",
        ext4_vol.mkdir(&mut writer, bin_block, bin_ino, "apps")
    );
    let (home_ino, home_block) = must!(
        "crear /home en EXT4",
        ext4_vol.mkdir(&mut writer, root_dir_block, 2, "home")
    );
    let (_, user_block) = must!(
        "crear el directorio de usuario en EXT4",
        ext4_vol.mkdir(&mut writer, home_block, home_ino, &username)
    );
    must!(
        "escribir /boot/kernel.elf en EXT4",
        ext4_vol.write_file(&mut writer, boot_block, "kernel.elf", &kernel_data, true)
    );
    let files_to_copy = [
        ("/bin/system/ltop.elf", bin_system_block, "ltop.elf", true),
        ("/bin/apps/vanish.elf", bin_apps_block, "vanish.elf", true),
        ("/bin/apps/edit.elf", bin_apps_block, "edit.elf", true),
        ("/bin/apps/hello.elf", bin_apps_block, "hello.elf", true),
    ];
    for (src_path, target_block, name, is_exec) in files_to_copy {
        let data = read_live_file(src_path);
        must!(
            format!("copiar {name} a EXT4"),
            ext4_vol.write_file(&mut writer, target_block, name, &data, is_exec)
        );
    }
    let kconf_content = format!("KEYMAP={keymap}\n");
    must!(
        "escribir /etc/keyboard.conf",
        ext4_vol.write_file(
            &mut writer,
            etc_block,
            "keyboard.conf",
            kconf_content.as_bytes(),
            false
        )
    );
    let kernel_conf_content =
        format!("HOSTNAME={hostname}\nSCHEDULER=smp\nLOGLEVEL=info\nRING=0\n");
    must!(
        "escribir /etc/kernel.conf",
        ext4_vol.write_file(
            &mut writer,
            etc_block,
            "kernel.conf",
            kernel_conf_content.as_bytes(),
            false
        )
    );
    let notas_content =
        format!("Bienvenido a LixOS, {username}.\nTu sistema está instalado y listo.\n");
    must!(
        "escribir notas.txt",
        ext4_vol.write_file(
            &mut writer,
            user_block,
            "notas.txt",
            notas_content.as_bytes(),
            false
        )
    );
    progress(
        app,
        term,
        5,
        "Sincronizando búferes con almacenamiento no volátil",
    );
    must!("sincronizar el disco", sys_block_sync(handle));
    let _ = sys_handle_close(handle);
}
#[no_mangle]
pub extern "C" fn _start() -> ! {
    sys_write("\x1b[?1049h\x1b[?25l\x1b[2J");
    let backend = LixosBackend::new();
    let mut term = match Terminal::new(backend) {
        Ok(t) => t,
        Err(_) => {
            teardown_terminal();
            sys_write("No se pudo inicializar la terminal TUI\n");
            sys_exit(1);
        }
    };
    let _ = term.clear();
    let devices = probe_devices();
    let mut app = App::new(devices);
    let confirmed = run_app(&mut app, &mut term);
    if app.screen == Screen::NoDevices {
        teardown_terminal();
        sys_exit(1);
    }
    if confirmed {
        app.screen = Screen::Progress;
        app.progress_step = 0;
        app.progress_msg = String::from("Preparando la instalación…");
        let _ = term.clear();
        draw_frame(&app, &mut term);
        run_install(&mut app, &mut term);
        app.success = true;
    } else {
        app.success = false;
    }
    app.screen = Screen::Done;
    app.progress_step = 0;
    app.status.clear();
    let _ = term.clear();
    draw_frame(&app, &mut term);
    wait_any_key();
    let _ = term.show_cursor();
    let _ = term.flush();
    teardown_terminal();
    sys_exit(0);
}
#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    sys_write("\x1b[31m[lixinstall] Panic en Ring 3\x1b[0m\n");
    sys_exit(1);
}
