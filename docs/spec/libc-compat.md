# Compatibilidad de libc (`sys-libc`)

> **Tipo:** especificación · **Estado:** borrador · **ADR:** 06, 22
> **Relacionado:** [Portabilidad de software](porting-compat.md)

Define la capa de compatibilidad `sys-libc`: constantes y layouts de Linux x86_64, APIs emuladas en userspace y las diferencias con POSIX que exigen parches.

El objetivo de portabilidad es la **compatibilidad a nivel de código fuente** de proyectos en C/C++ POSIX estándar (ver ADR-22). No se ejecutan binarios de Linux directamente. El SDK de LixOS incluye `sys-libc`, que traduce las llamadas POSIX a las llamadas nativas de LixOS.

Para minimizar la necesidad de parchar software, `sys-libc` copia deliberadamente el valor de constantes y los _layouts_ de ciertas estructuras clave del ABI de Linux x86_64, aunque internamente el kernel funcione distinto. Esto evita fallos espurios en código que asume tamaños o valores de macros (`#ifdef __linux__`).

## 1. Constantes y Macros Copiadas de Linux x86_64

La librería base de C garantiza que las siguientes macros compartan el mismo valor entero que en Linux.

### 1.1 Sistema de Archivos y Open
- `O_RDONLY` (00), `O_WRONLY` (01), `O_RDWR` (02)
- `O_CREAT` (0100), `O_EXCL` (0200), `O_TRUNC` (01000), `O_APPEND` (02000)
- `O_DIRECTORY` (0200000)
- Valores `SEEK_SET` (0), `SEEK_CUR` (1), `SEEK_END` (2)

### 1.2 Códigos de Error (`errno.h`)
Aunque internamente el kernel usa códigos de error jerárquicos tipados en Rust, `sys-libc` expone la tabla estándar de errores.
- `EPERM` (1), `ENOENT` (2), `ESRCH` (3), `EINTR` (4), `EIO` (5), `ENXIO` (6), `E2BIG` (7), `ENOEXEC` (8), `EBADF` (9), `ECHILD` (10), `EAGAIN` (11), `ENOMEM` (12), `EACCES` (13), `EFAULT` (14), `EINVAL` (22), etc.

### 1.3 Permisos de Archivo (`sys/stat.h`)
LixOS usa Handles con Derechos, no permisos de Unix, pero la función `stat()` debe simular y devolver modos estándar.
- `S_IFMT` (0170000)
- `S_IFDIR` (0040000), `S_IFCHR` (0020000), `S_IFBLK` (0060000), `S_IFREG` (0100000)
- Máscaras rwx: `S_IRUSR` (00400), `S_IWUSR` (00200), `S_IXUSR` (00100)

## 2. Layouts de Estructuras (Struct Compatibility)

El tamaño (`sizeof`) y la alineación (`alignof`) de estas estructuras son idénticos a los de `glibc` / `musl` en x86_64.

- `struct timespec`: `time_t tv_sec` (8 bytes), `long tv_nsec` (8 bytes).
- `struct stat`: 144 bytes en x86_64. (Los campos que no aplican a LixOS, como UID/GID tradicionales o Inodos, se rellenan con 0 o valores *dummy*).
- `struct sockaddr` y `struct sockaddr_un` (Cuando se introduzca AF_UNIX).
- `struct pollfd`.

## 3. Emulación de APIs

Las siguientes APIs son emuladas completamente en espacio de usuario por `sys-libc` utilizando primitivas nativas de LixOS (`wait_many` y canales):
- `poll()` / `select()`
- `epoll` (ADR-22, simulado con canales de eventos)
- `posix_spawn()` / `vfork()`
- Subconjunto de señales: `SIGINT`, `SIGTERM`, `SIGKILL`, `SIGSEGV`, `SIGALRM`, `SIGCHLD`. (Las señales complejas o de control de trabajos se ignoran o se traducen a matanzas del proceso padre según corresponda).

## 4. Diferencias con POSIX (Lo que fallará)

El código fuente que dependa fuertemente de lo siguiente requerirá parches o no será soportado en v1.0:
- Uso explícito de `fork()` seguido de trabajo complejo en el hijo sin hacer `exec()`. LixOS provee `vfork()`, pero si el software necesita copy-on-write pesado, fallará en T2.
- APIs exclusivas de Linux (`inotify`, `signalfd`, `timerfd`).
- Demonios que utilicen `setuid()` o intenten cambiar permisos UID/GID con `chown()`. Estas llamadas simplemente devolverán éxito y no harán nada (o devolverán `ENOSYS`), puesto que en LixOS el control de acceso se basa en capabilities y revocación dinámica (ADR-23).

## 5. Implementación actual

Comportamiento observado en `libs/sys-libc/src/lib.rs` y `libs/lix-sys/src/lib.rs`. Los apartados siguientes describen hechos del código; no modifican la especificación de las secciones anteriores.

### 5.1 Estructura del crate

`sys-libc` declara dos módulos públicos:

| Módulo | Contenido |
| --- | --- |
| `syscalls` | primitivas `syscall1`…`syscall4` (inline asm con la instrucción `SYSCALL`) y los envoltorios `sys_*` |
| `io` | `Stdout`, que implementa `core::fmt::Write` sobre `sys_write` |

Además exporta las macros `print!` y `println!`.

### 5.2 Envoltorios disponibles

| Función | Número | Argumentos |
| --- | --- | --- |
| `sys_write(s: &str)` | `0x302` | fija el descriptor en `1` |
| `sys_read(fd, buf) -> usize` | `0x301` | espera hasta que haya entrada |
| `sys_read_nonblocking(fd, buf) -> usize` | `0x309` | solo `fd == 0`; devuelve `0` si no hay entrada |
| `sys_sleep(ms)` | `0x107` | |
| `sys_exit(code) -> !` | `0x101` | |
| `sys_power(cmd) -> !` | `0x803` | |
| `sys_process_list(buf, max) -> usize` | `0x10C` | usa `ProcessInfo` local al crate |
| `sys_system_metrics(&mut SystemMetrics) -> i32` | `0x80A` | métricas de scheduler, RAM y E/S |
| `sys_kill(pid, signal) -> i32` | `0x102` | |
| `sys_spawn(path) -> Result<u64, i32>` | `0x100` | ruta + longitud |
| `sys_waitpid(pid, &status) -> Result<u64, i32>` | `0x10A` | |
| `sys_list_dir(path, &mut out) -> Result<usize, i32>` | `0x306` | |
| `sys_read_file(path, &mut out) -> Result<usize, i32>` | `0x307` | |
| `sys_terminal_info(&mut info) -> i32` | `0x703` | reutiliza `lix_sys::TerminalInfo` |
| `sys_terminal_set_scale(scale) -> i64` | `0x704` | |
| `sys_terminal_zoom(delta) -> i64` | `0x704` | traduce a `10` (acercar), `11` (alejar) u `0` |
| `sys_set_keymap(layout_id) -> i64` | `0x706` | |
| `sys_get_keymap() -> u8` | `0x707` | |

Los errores se devuelven como `Result<_, i32>` o como entero negativo.

### 5.3 Ausencias frente a la especificación

- **No hay `errno`.** Las secciones §1.2 y §2 suponen constantes de error y *layouts* de estructuras copiados de Linux; el crate no declara `errno`, `struct stat`, `struct termios` ni `struct sockaddr`. El único catálogo de errores es `lix_sys::ErrorCode` (23 valores, incluidos `ELIX_REVOKED = 1001`, `ELIX_LIMIT = 1002`, `ELIX_PEER_CLOSED = 1003`).
- **No hay API POSIX.** No existen `open`, `read`, `write`, `fork`, `exec*`, `poll`, `select`, `epoll`, `pthread_*`, `malloc` ni `stdio` en `sys-libc`; la §3 (emulación de APIs) no tiene implementación correspondiente.
- **No hay `sysroot` ni cabeceras C.** No hay `<lix/syscall.h>` ni árbol de cabeceras generado.
