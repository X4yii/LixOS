# Consola, entrada y registro

> **Tipo:** diseño · **Estado:** borrador · **ADR:** 13, 01
> **Relacionado:** [Arranque](boot.md) · [Gráficos](graphics.md) · [Procesos y scheduler](process-scheduler.md) · [Construcción y ejecución](../guides/build.md)

Describe la consola de texto sobre framebuffer, el enrutado de entrada de teclado y serie, el subsistema de registro (`klog`) y el manejador de pánico. Todo este subsistema reside en Ring 0.

---

## 1. Consola sobre framebuffer

Implementada en `kernel/src/drivers/framebuffer/`, dividida en cinco módulos:

| Módulo | Contenido |
| --- | --- |
| `mod.rs` | `FramebufferConsole`, paleta, analizador ANSI y API pública |
| `hw.rs` | `GopDisplay`: acceso al framebuffer GOP (dirección, pitch, bpp) |
| `grid.rs` | `TextCell`, `TextGridManager`, `MAX_COLS`, `MAX_ROWS` |
| `cursor.rs` | `SoftwareCursor`, `CURSOR_COLOR` |
| `ansi.rs` | `AnsiDecoder` (estado, parámetros, marcador privado) |

### 1.1 Geometría y fuente

| Constante | Valor | Origen |
| --- | --- | --- |
| `FONT_WIDTH` | 8 px | `drivers/font.rs` |
| `FONT_HEIGHT` | 16 px | `drivers/font.rs` |
| Glifos | ASCII 32–126 (95) | `drivers/font.rs`; el resto usa el glifo de reserva |
| `MAX_COLS` | 160 celdas | `framebuffer/grid.rs` |
| `MAX_ROWS` | 60 celdas | `framebuffer/grid.rs` |
| `ZOOM_MIN` | 4 | `framebuffer/mod.rs` |
| `ZOOM_MAX` | 16 | `framebuffer/mod.rs` |
| Color de cursor | `0x007DCFFF` | `framebuffer/cursor.rs` |

El zoom usa una variable entera `zoom_q` acotada a `[ZOOM_MIN, ZOOM_MAX]`, en lugar de un factor entero. Las celdas se derivan de ella:

```
cell_width  = FONT_WIDTH  * zoom_q / ZOOM_MIN
cell_height = FONT_HEIGHT * zoom_q / ZOOM_MIN
cols = (hw.width  / cell_width ).min(MAX_COLS)
rows = (hw.height / cell_height).min(MAX_ROWS)
```

`update_geometry()` recalcula `cols` y `rows` tras cada cambio. `zoom_q = ZOOM_MIN` corresponde a escala 1; `set_scale(s)` fija `zoom_q = clamp(s, 1, 4) * ZOOM_MIN`, y `TerminalInfo::scale` devuelve `zoom_q.div_ceil(ZOOM_MIN)`.

### 1.2 Estado de la consola

`FramebufferConsole` contiene `hw: GopDisplay`, `grid: TextGridManager`, `cursor: SoftwareCursor`, `ansi: AnsiDecoder`, más los campos `is_reversed`, `is_bold`, `zoom_q`, `cols`, `rows`, `fg_color` y `bg_color`.

Las rejillas (`grid.rs`) son tres arreglos estáticos de `MAX_COLS * MAX_ROWS` celdas `TextCell { ch, fg, bg }` inicializadas con `COLOR_BG_DEFAULT`:

- `PRIMARY_GRID`: pantalla principal.
- `ALT_GRID`: pantalla alternativa, seleccionada con las secuencias privadas `?1049` y `?47`.
- `REFLOW_SCRATCH`: copia temporal usada durante el reflow.

Junto a ellos, `PRIMARY_WRAP` y `ALT_WRAP` son arreglos de `MAX_ROWS` booleanos que marcan si una fila continúa en la siguiente (*soft wrap*).

`TextGridManager` expone `get_cell` / `set_cell`, `set_wrap` / `get_wrap`, `scroll_up`, `clear`, `clear_line` y `reflow_primary` sobre la rejilla activa.

### 1.3 Reflow y soft wrap

Cuando la escritura alcanza la última columna, `write_char` marca `set_wrap(fila, true)`, reinicia la columna y avanza de fila; si desborda la última fila se hace scroll (`framebuffer/mod.rs`).

`set_zoom_q` recalcula `cols`/`rows` con `update_geometry()` y, si la pantalla activa es la principal, llama a `reflow_primary(old_cols, old_rows, cols, rows, cursor.col, cursor.row, COLOR_BG_DEFAULT)`: junta con `REFLOW_SCRATCH` las líneas unidas por *soft wrap* y las vuelve a partir al nuevo ancho, devolviendo la nueva posición del cursor. En la pantalla alternativa no se reflowea: se limpia y se espera que la aplicación TTY redibuje. El resultado final es el mismo para cualquier factor de zoom: el contenido textual se preserva y solo cambia el particionado en líneas.

### 1.4 Paleta

| Constante | Valor |
| --- | --- |
| `COLOR_BG_DEFAULT` | `0x001A1B26` |
| `COLOR_FG_DEFAULT` | `0x00C0CAF5` |
| `COLOR_PANIC_BG` | `0x00801111` |
| `COLOR_PANIC_FG` | `0x00FFFFFF` |
| `ANSI_PALETTE_16` | 16 valores, índices 0–15 |

`ansi_256_to_rgb(idx)` convierte los índices 16–231 (cubo RGB con paso `55 + 40·n`) y 232–255 (escala de grises con paso 10) a RGB de 24 bits.

### 1.5 Secuencias ANSI interpretadas

El analizador reside en `FramebufferConsole::write_char` y usa `AnsiDecoder { state, params[8], param_idx, is_private }`. La transición es `ESC` → estado 1 → `[` → estado 2 (parámetros: dígitos y `;`; `?` marca la secuencia privada).

| Secuencia | Significado |
| --- | --- |
| `ESC [ … m` | SGR: `0`, `1`, `7`, `27`, `30`–`37`, `38;2` / `38;5`, `39`, `40`–`47`, `48;2` / `48;5`, `49`, `90`–`97`, `100`–`107` |
| `ESC [ ? 1049 h / l`, `ESC [ ? 47 h / l` | Pantalla alternativa |
| `ESC [ ? 25 h / l` | Visibilidad del cursor |
| `ESC [ H` / `ESC [ f` | Posicionar cursor (fila, columna) |
| `ESC [ J` (`2`, `3`) | Limpieza completa con el color de fondo actual |
| `ESC [ K` (`0`, `1`, `2`) | Borrado de línea |
| `ESC [ A / B / C / D` | Desplazar cursor (por defecto 1, acotado a los bordes) |

Un byte final alfabético o de símbolo no reconocido reinicia el analizador. Los códigos de control de escritura son `\n`, `\r`, `\t`, `\x08` (retroceso que borra la celda anterior) y el desplazamiento de la pantalla al desbordar la última fila.

### 1.6 API pública

```rust
pub fn init(addr: *mut u8, width: usize, height: usize, pitch: usize, bpp: u16)
    -> Option<&'static mut dyn EarlyConsole>;
pub fn emergency_panic_screen();
pub fn clear_screen();
pub fn get_terminal_info() -> Option<lix_sys::TerminalInfo>;
pub fn set_scale(scale: usize);
pub fn zoom_in();  pub fn zoom_out();  pub fn zoom_reset();
pub fn leave_alt_screen_if_active();
pub fn cursor_blink_tick();
```

Además existen `FRAMEBUFFER_DEVICE` (candado con la consola activa) y `FramebufferEarlySink` (sumidero sin estado). Las funciones públicas `set_scale`, `zoom_in`, `zoom_out`, `zoom_reset` y `leave_alt_screen_if_active` bloquean `FRAMEBUFFER_DEVICE` y delegan en el método homónimo de `FramebufferConsole`; `set_scale` acota a `1..=4`. `setup_panic_screen` aplica la paleta de pánico, limpia y sitúa el cursor en la fila 1.

---

## 2. Entrada

### 2.1 Cola estándar

`kernel/src/drivers/tty.rs` define la disciplina de línea y una única cola de caracteres:

```rust
pub static ref STDIN: IrqSafeSpinlock<VecDeque<char>>;

pub fn push_char(c: char);
pub fn push_str(s: &str);
pub fn pop_char() -> Option<char>;
```

`push_char` intercepta `\x03` (Ctrl+C): si `task::FOREGROUND_TASK` no es cero, envía la señal `2` a ese PID y activa `task::CANCEL_REQUEST`. El carácter se encola igualmente y se invoca `scheduler::wake_stdin_waiters()`.

### 2.2 Teclado PS/2

`kernel/src/drivers/ps2_keyboard.rs` atiende la IRQ1 (vector 33), lee el puerto `0x60` y delega la decodificación al subsistema de teclado. No mantiene estado propio.

| Tecla | Carácter emitido a `tty` |
| --- | --- |
| Unicode | el propio carácter |
| `Backspace` | `\x08` |
| Flecha arriba / abajo / izquierda / derecha | `\x11`, `\x12`, `\x13`, `\x14` |
| `Home` | `ESC [ H` |
| `End` | `ESC [ F` |
| `Delete` | `ESC [ 3 ~` |

### 2.3 Subsistema de teclado

`kernel/src/drivers/keyboard/` gestiona las disposiciones:

| Elemento | Detalle |
| --- | --- |
| `KeyboardLayoutKind` | 12 variantes con `#[repr(u8)]`: `latam` (0), `es` (1), `us` (2), `uk` (3), `de` (4), `fr` (5), `colemak` (6), `dvorak` (7), `dvp` (8), `se` (9), `no` (10), `jp` (11) |
| `LixLayout` | `Latam` \| `Spain` \| `Builtin(AnyLayout)`; implementa el trait `KeyboardLayout` de `pc-keyboard` |
| `latam.rs`, `es.rs` | Disposiciones propias; el resto usan las del crate `pc-keyboard` |
| Por defecto | `KeyboardLayoutKind::LatinAmerica` |
| `set_layout` / `get_layout` | Cambio y consulta de la disposición activa |
| `process_scancode` | Traduce un scancode a `DecodedKey` |

Atajos producidos por la capa de disposición (antes de llegar al consumidor):

| Combinación | Carácter | Significado |
| --- | --- | --- |
| `Ctrl + -` / `Ctrl + Numpad-` | `\x1f` | zoom out |
| `Ctrl + +` / `Ctrl + =` / `Ctrl + Numpad+` | `\x1e` | zoom in |
| `Ctrl + 0` / `Ctrl + Numpad0` | `\x1c` | zoom reset |

**Configuración en el arranque:** `load_config_from_vfs()` se invoca desde `_start` (`kernel/src/main.rs`) y busca `KEYMAP=`, `keymap=`, `LAYOUT=` o `layout=` en `/etc/keyboard.conf`, `etc/keyboard.conf`, `/etc/vconsole.conf` o `etc/vconsole.conf`, ignorando líneas vacías y comentarios `#`.

### 2.4 Puerto serie

`kernel/src/drivers/serial.rs` configura COM1 (`0x3F8`). `drain_to_stdin()` recorre los bytes disponibles y los traduce:

| Byte de entrada | Carácter encolado |
| --- | --- |
| `0x03` | `\x03` (señal en `push_char`) |
| `\r` | `\n` |
| `0x7F` | `\x08` |
| `>= 0x20`, `\n`, `0x08`, `0x1B`, `\t`, `0x11..=0x14` | tal cual |
| resto | se descarta |

Se invoca desde `arch::x86_64::halt_loop()` en cada iteración ociosa y desde el manejo de `sys_read` (`0x301`) en `kernel/src/task/syscall.rs`.

---

## 3. Registro (`klog`)

`kernel/src/klog/` separa el enrutado de los sumideros.

| Elemento | Detalle |
| --- | --- |
| `EarlyConsole` | trait `write_str(&mut self, &str)` (`klog/sink.rs`) |
| `MAX_SINKS` | 4 sumideros (`klog/mod.rs`) |
| `RING_BUFFER_CAPACITY` | 65536 bytes (64 KiB) |
| `KlogRouter` | `ring_buffer` + `sinks: [Option<&'static mut dyn EarlyConsole>; MAX_SINKS]` |
| `KlogRingBuffer` | búfer circular con `total_written` (`klog/buffer.rs`) |
| `register_console(sink)` | registra un sumidero, descartando si ya hay 4 |
| `force_unlock()` | liberación forzada del candado, usada por el manejador de pánico |
| `kprint!`, `kprintln!` | macros sobre `_print(args)` |

`write_str_all` replica cada escritura en el búfer circular y en todos los sumideros registrados. En el arranque se registran dos: el puerto serie COM1 y la consola de framebuffer. El enrutado no aplica niveles de severidad, filtros ni marcas de tiempo.

---

## 4. Manejador de pánico

`kernel/src/panic.rs`, con `panic = "abort"` en ambos perfiles:

1. `cli` para deshabilitar interrupciones.
2. `klog::force_unlock()`.
3. `framebuffer::emergency_panic_screen()`: fuerza el candado del framebuffer y aplica la paleta de pánico.
4. Impresión del banner `LIXOS KERNEL PANIC`, el origen (`file:line:column`, o «Ubicación desconocida») y `Detalle: <mensaje>`.
5. `halt_loop()`.

---

## 5. Consola interactiva

### 5.1 Módulo de shell en kernel (`kernel/src/shell/`)

Proporciona `execute_line(cmd_line)` y siete comandos internos:

| Comando | Acción |
| --- | --- |
| `help` | Lista los comandos internos y los ficheros de los directorios de búsqueda |
| `clear` | Limpia la pantalla |
| `ping` | Imprime `pong!` |
| `ls [dir]` | Lista directorios (raíz por defecto) |
| `cat <fichero>` | Muestra el contenido como texto o indica datos binarios |
| `kill [-9] <pid>` | Envía señal (15 por defecto, 9 con `-9`) |
| `pkill <subcadena>` | Envía la señal 15 a los procesos cuyo nombre contiene la subcadena |

- El prefijo `sudo` fija `privilege = 1` y `su0` fija `privilege = 0` para el comando siguiente; el valor por defecto es `3`.
- `VANISH_ENV.paths` contiene `/bin`, `/usr/bin` y `./`; la resolución busca el nombre tal cual y con sufijo `.elf`.
- El binario se carga con `task::elf::load_elf`, su PID se publica en `task::FOREGROUND_TASK` y `execute_line` espera en bucle a que desaparezca antes de retornar.
- `CANCEL_REQUEST` y `FOREGROUND_TASK` se declaran en `kernel/src/task/mod.rs` y se reexportan desde `shell`.

Esta función **no tiene llamantes** en el árbol actual: el módulo queda disponible pero inactivo.

### 5.2 Vanish (shell de espacio de usuario)

El REPL interactivo vive en Ring 3, en `userland/vanish/src/main.rs`, y se lanza desde `_start` (el de `kernel/src/main.rs`) buscando `/bin/vanish.elf` y después `/vanish.elf`.

**Comandos internos** (`execute_command`):

| Comando | Acción |
| --- | --- |
| `cd [dir]`, `pwd` | Resolución de rutas relativas al directorio de trabajo actual; valida con `sys_list_dir` |
| `ls [dir]`, `cat <archivo>` | `sys_list_dir` y `sys_read_file` |
| `help`, `clear`, `echo`, `exit` | Utilidades de la propia shell |
| `zoom [in\|out\|reset\|1\|2\|3\|4]`, `scale` | `sys_terminal_zoom` y `sys_terminal_set_scale` |
| `keymap [nombre]`, `loadkeys` | `sys_get_keymap` y `sys_set_keymap`; lista las 12 disposiciones con sus códigos |
| `shutdown`, `poweroff` | `sys_power(0)` |

Cualquier otra entrada se resuelve como ejecutable contra `cwd`, `cwd/<cmd>.elf`, `/bin/<cmd>.elf`, `/<cmd>.elf`, `/bin/<cmd>`, `/<cmd>` y finalmente `<cmd>`; se lanza con `sys_spawn` y se espera con `sys_waitpid`. Si ninguna ruta funciona se imprime «comando no encontrado».

- Historial en memoria con navegación ascendente y descendente.
- Completado por tabulación: con entrada vacía lista los comandos internos; con prefijo parcial filtra comandos y binarios de `cwd`, `/bin` y `/`.
- Edición de línea con cursor (inserción, borrado, `ESC [ C` / `ESC [ D`).
- Prompt con secuencias ANSI: `vanish:<cwd>$`.
- `Ctrl + -`, `Ctrl + +` y `Ctrl + 0` se traducen a `sys_terminal_zoom(-1)`, `sys_terminal_zoom(1)` y `sys_terminal_set_scale(1)`.
- Disposición de teclado consultable y cambiable en caliente con `keymap`.

Si `vanish.elf` no está disponible, `_start` crea un hilo de kernel sobre `arch::x86_64::halt_loop` con nombre `idle_shell` y publica su PID en `scheduler::SHELL_PID`.

---

## 6. Bucle ocioso

`arch::x86_64::halt_loop()` drena el puerto serie hacia `STDIN` y ejecuta `hlt` en bucle. La planificación se reanuda en cada interrupción del temporizador APIC.
