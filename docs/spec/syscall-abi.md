# ABI de syscalls

> **Tipo:** especificación · **Estado:** consolidado · **ABI:** `0.1.0` · **ADR:** 03, 05, 17, 22, 24
> **Relacionado:** [Capabilities](capabilities.md) · [Gráficos](../design/graphics.md)

Define la convención de llamada, los tipos base, los códigos de error y la **tabla completa de las 35 syscalls** que componen el ABI `0.1.0` de LixOS, más los alias de Linux y los IDs reservados sin implementar.

La fuente única de verdad es **`abi/syscalls.toml`** (ADR-24). Esta documentación es una réplica derivada de ese archivo: se escribe a mano, pero se construye a partir de sus campos `id`, `name`, `args` y `rights`, y debe actualizarse en el mismo cambio (§5). Hasta `1.0.0` el ABI puede cambiar; desde `1.0.0` queda congelado (ADR-17).

## 1. Convención de llamada

ABI System V adaptada a x86_64, declarada en el TOML como `calling_convention = "sysv_registers"`:

| Registro | Uso |
| --- | --- |
| `rax` | número de syscall (entrada) y resultado (salida) |
| `rdi, rsi, rdx, r10, r8` | argumentos 1 a 5 |
| `r9` | **reservado**: el *entry stub* actual no lo reenvía al dispatcher |
| `rcx, r11` | destruidos por `SYSCALL`/`SYSRET` |
| resto | preservados por el kernel |

**Resultado:** `rax >= 0` es éxito (el valor depende de la syscall). `rax < 0` es `-errno`, construido como `-(errno as i32) as u64`.

Hay **dos vías de entrada**, ambas hacia la misma función `syscall_dispatcher` (`kernel/src/task/syscall/mod.rs`):

| Vía | Mecanismo | Número | Argumentos |
| --- | --- | --- | --- |
| Primaria | `SYSCALL` / `SYSRET` (MSRs `EFER.SCE`, `STAR`, `LSTAR`, `FMASK` vistos en `init_syscall_msrs_for_cpu`) | `rax` | `rdi, rsi, rdx, r10, r8` |
| Secundaria | `int 0x80` (vector `0x80`, DPL Ring 3) | `rax` | `rdi, rsi, rdx, r10, r8` |

El ensamblado de ambas rutas está en `kernel/asm/syscall.S` (`syscall_entry_asm`, `syscall_handler_asm`). Ninguna de las dos usa la convención `ebx/ecx/edx/esi/edi` de Linux: **la convención de argumentos es la misma en las dos vías** y el número va siempre en `rax` desde el espacio de usuario. Todas las syscalls actuales usan como máximo cuatro argumentos; añadir una sexta exigiría modificar los *stubs* para reenviar `r9`.

### Reglas generales del contrato (objetivo de diseño)

Estas reglas describen el contrato que el ABI pretende garantizar hasta `1.0`. **No todas se aplican hoy**: la §10 recoge lo que el kernel implementa efectivamente.

1. **Punteros de usuario se validan y se copian una sola vez** (`user_slice`, `copy_to_user`) y no se vuelven a leer: elimina TOCTOU.
2. **Estructuras versionadas:** las estructuras que cruzan la frontera son `#[repr(C)]`.
3. **Flags desconocidos se rechazan** (`EINVAL`), nunca se ignoran en silencio.
4. **Todo bloqueo tiene plazo** (`deadline` monotónico; `0` = no bloquear) y ningún bloqueo es ininterrumpible: `sys_process_kill` siempre puede despertar al hilo.
5. **Los números de syscall nunca se reutilizan.** Una syscall retirada conserva su ID reservado (§7).
6. **Límites máximos documentados** por longitud (§9).
7. **Ninguna syscall asigna memoria sin cobrarla** al proceso que la pide (ADR-19); todas pueden devolver `ENOMEM`.
8. **Fallo parcial limpio:** si falla, no deja recursos a medias.

## 2. Tipos base

```rust
pub type Handle   = u64;   // 32 bits generación | 32 bits índice (ADR-05). 0 = inválido
pub type Rights   = u32;   // máscara de derechos (newtype lix_sys::Rights)
pub type Deadline = u64;   // ns monotónico absoluto
```

Estructuras `#[repr(C)]` que definen la frontera:

```rust
pub struct InputEvent {          // leída con sys_read sobre un handle de entrada
    pub time_ns: u64,
    pub device: u16,
    pub kind: u16,               // KEY, REL, ABS, BUTTON, ...
    pub code: u32,               // keycode físico, no carácter
    pub value: i32,
}

pub struct TerminalInfo {        // sys_terminal_info
    pub cols: u16, pub rows: u16,
    pub pixel_width: u16, pub pixel_height: u16,
    pub font_width: u8, pub font_height: u8,
    pub scale: u8, pub _reserved: u8,
}

pub struct ProcessInfo {         // sys_process_list
    pub pid: u64,
    pub name: [u8; 16],          // truncado a 16 bytes
    pub state: u8,               // 0 Ready, 1 Running, 2 Blocked, 3 Sleeping, 4 Zombie, 5 Dead
    pub privilege_level: u8,
    pub cpu_ticks: u64,
    pub ram_bytes: u64,
    pub disk_read_bytes: u64,
}

pub struct SystemMetrics {       // sys_monitor_metrics
    pub scheduler_ticks: u64,
    pub total_ram_bytes: u64,
    pub free_ram_bytes: u64,
    pub disk_read_ops: u64,
    pub disk_read_bytes: u64,
    pub disk_mounted: u64,       // 0/1
}
```

Los derechos están definidos en [capabilities.md](capabilities.md).

## 3. Códigos de error

Se usan los **números de `errno` de Linux** (ADR-22) para que la libc no tenga que traducir. Códigos propios a partir de `1000`. Catálogo completo declarado en `lix_sys::ErrorCode` (24 valores):

| Código | Nombre | Significado | `errno` que ve C |
| --- | --- | --- | --- |
| 1 | `EPERM` | operación no permitida | `EPERM` |
| 2 | `ENOENT` | no existe | `ENOENT` |
| 3 | `ESRCH` | no existe ese proceso | `ESRCH` |
| 4 | `EINTR` | interrumpido | `EINTR` |
| 5 | `EIO` | error de E/S | `EIO` |
| 8 | `ENOEXEC` | ELF no ejecutable | `ENOEXEC` |
| 9 | `EBADF` | handle inválido o de tipo incorrecto | `EBADF` |
| 11 | `EAGAIN` | no disponible ahora | `EAGAIN` |
| 12 | `ENOMEM` | sin memoria o cuota agotada | `ENOMEM` |
| 13 | `EACCES` | derechos insuficientes | `EACCES` |
| 14 | `EFAULT` | puntero de usuario inválido | `EFAULT` |
| 16 | `EBUSY` | recurso ocupado (p. ej. directorio no vacío) | `EBUSY` |
| 17 | `EEXIST` | ya existe | `EEXIST` |
| 19 | `ENODEV` | dispositivo no disponible | `ENODEV` |
| 22 | `EINVAL` | argumento o flag inválido | `EINVAL` |
| 24 | `EMFILE` | tabla de handles llena | `EMFILE` |
| 38 | `ENOSYS` | syscall no implementada o número desconocido | `ENOSYS` |
| 98 | `EADDRINUSE` | dirección/puerto en uso | `EADDRINUSE` |
| 110 | `ETIMEDOUT` | venció el `deadline` | `ETIMEDOUT` |
| 1001 | `ELIX_REVOKED` | handle revocado (diseño; §7) | `EBADF` |
| 1002 | `ELIX_LIMIT` | límite de recursos del proceso/Job | `EMFILE`/`EAGAIN` |
| 1003 | `ELIX_PEER_CLOSED` | el otro extremo se cerró | `EPIPE` |

Los errores de fichero se expresan como `FsError` en `kernel/src/fs/error.rs` y se traducen a estos códigos (`NotFound → ENOENT`, `PermissionDenied/ReadOnly → EACCES`, `NotEmpty → EBUSY`, `NoSpace/OutOfMemory → ENOMEM`, `Corrupt/Io → EIO`).

## 4. Cómo se aplica un derecho (obligatorio para toda syscall)

Antes de ejecutar cualquier handler, el dispatcher (`kernel/src/task/syscall/mod.rs`) hace dos comprobaciones:

1. **Número:** `syscall_table::lookup_syscall(id)` sobre el registro generado `kernel/src/task/syscall_table.rs`. Si no existe la fila → `-ENOSYS`.
2. **Derecho:** `task::rights::required(entry.required_rights)` traduce el campo `rights` del TOML a una máscara; si `Process.rights` del llamante no la contiene → `-EACCES`. Si el nombre no corresponde a un derecho conocido → también `-EACCES`.

El campo `required_rights` de la §6 es exactamente ese campo `rights` de `abi/syscalls.toml`. Las máscaras de proceso (`KERNEL`, `SESSION`, `BASE_USER`) están en `kernel/src/task/rights.rs` y se documentan en [capabilities.md](capabilities.md) §13.7.

## 5. Fuente única y regeneración (ADR-24)

`abi/syscalls.toml` declara:

```toml
[abi]
version = "0.1.0"
arch = "x86_64"
calling_convention = "sysv_registers"

[[syscall]]
id = 0x301
name = "sys_read"
description = "Lee bytes desde un handle hacia un búfer de usuario"
args = [{ name = "handle", type = "u64" }, { name = "buf_ptr", type = "u64" }, { name = "len", type = "u64" }]
rights = "READ"
```

De ese archivo se **generan** (nunca se editan a mano):

| Artefacto | Contenido |
| --- | --- |
| `kernel/src/task/syscall_table.rs` | `SyscallEntry { id, name, description, required_rights }` y `lookup_syscall()` |
| `libs/lix-sys/src/syscalls.rs` | constantes `SYS_*` (incluidas en `lib.rs` con `include!`) |
| `include/lix/syscall.h` | macros `#define SYS_*` para C |

Procedimiento:

```sh
# 1. editar abi/syscalls.toml
cargo xtask codegen     # 2. regenera los tres artefactos
# 3. actualizar la tabla de este documento (§6) a mano, reflejando el TOML
git add abi/syscalls.toml kernel/src/task/syscall_table.rs include/lix/syscall.h libs/lix-sys/src/syscalls.rs
```

El job **`codegen-drift`** de CI (`.github/workflows/ci.yml`) ejecuta `cargo xtask codegen` y después `git diff --exit-code` sobre los tres artefactos: si alguien cambia el TOML sin regenerar, o edita un artefacto generado, el pipeline falla. Ese job es el que impide que el ABI vuelva a desincronizarse. Este documento **no** está cubierto por ese job: su §6 se mantiene a mano y debe revisarse en cada cambio del TOML.

## 6. Tabla de syscalls (ABI `0.1.0`)

**35 syscalls.** Columnas = campos de `abi/syscalls.toml`: `id`, `name`, `args`, `rights`. Convención de nombres: `sys_<objeto>_<acción>`. Todas pueden devolver `EFAULT`, `EINVAL` y `ENOSYS`; no se repiten abajo.

### 6.1 Núcleo: handles (`0x00xx`)

| Nº | Syscall | Argumentos | `required_rights` |
| --- | --- | --- | --- |
| 0x000 | `sys_abi_version` | — | `NONE` |
| 0x001 | `sys_handle_close` | `handle: u64` | `NONE` |
| 0x002 | `sys_handle_dup` | `handle: u64, rights: u32` | `DUP` |

### 6.2 Procesos (`0x10x`)

| Nº | Syscall | Argumentos | `required_rights` |
| --- | --- | --- | --- |
| 0x100 | `sys_process_spawn` | `spec_ptr: u64` | `SPAWN` |
| 0x101 | `sys_process_exit` | `code: i32` | `NONE` |
| 0x102 | `sys_process_kill` | `pid: u64, signal: u32` | `MANAGE` |
| 0x107 | `sys_sleep` | `ms: u64` | `NONE` |
| 0x10A | `sys_waitpid` | `pid: u64, status_ptr: u64` | `MANAGE` |
| 0x10C | `sys_process_list` | `buffer: u64, max_count: u64` | `INSPECT` |

### 6.3 Archivos y directorios (`0x03xx`)

Rutas siempre absolutas; la autorización por prefijo está descrita en [capabilities.md](capabilities.md) §13.7.

| Nº | Syscall | Argumentos | `required_rights` |
| --- | --- | --- | --- |
| 0x301 | `sys_read` | `handle: u64, buf_ptr: u64, len: u64` | `READ` |
| 0x302 | `sys_write` | `handle: u64, buf_ptr: u64, len: u64` | `WRITE` |
| 0x306 | `sys_list_dir` | `path_ptr: u64, path_len: u64, out_ptr: u64, out_len: u64` | `READ` |
| 0x307 | `sys_read_file` | `path_ptr: u64, path_len: u64, out_ptr: u64, out_len: u64` | `READ` |
| 0x308 | `sys_write_file` | `path_ptr: u64, path_len: u64, data_ptr: u64, data_len: u64` | `WRITE` |
| 0x309 | `sys_read_nonblocking` | `handle: u64, buf_ptr: u64, len: u64` | `READ` |
| 0x30A | `sys_remove_file` | `path_ptr: u64, path_len: u64, recursive: u64` | `WRITE` |
| 0x30B | `sys_mkdir` | `path_ptr: u64, path_len: u64` | `WRITE` |

### 6.4 Consola y entrada (`0x07xx`)

| Nº | Syscall | Argumentos | `required_rights` |
| --- | --- | --- | --- |
| 0x703 | `sys_terminal_info` | `info_ptr: u64` | `NONE` |
| 0x704 | `sys_terminal_set_scale` | `command: u64` | `NONE` |
| 0x705 | `sys_input_open` | `class: u32` | `INPUT_READ` |
| 0x706 | `sys_set_keymap` | `layout_id: u8` | `DEVICES` |
| 0x707 | `sys_get_keymap` | — | `NONE` |

### 6.5 Sistema (`0x80x`)

| Nº | Syscall | Argumentos | `required_rights` |
| --- | --- | --- | --- |
| 0x801 | `sys_log_write` | `buf_ptr: u64, len: u64` | `LOG` |
| 0x803 | `sys_power` | `cmd: u64` | `POWER` |
| 0x80A | `sys_monitor_metrics` | `metrics_ptr: u64` | `INSPECT` |

### 6.6 Almacenamiento de bloques (`0x82x`)

| Nº | Syscall | Argumentos | `required_rights` |
| --- | --- | --- | --- |
| 0x820 | `sys_block_open` | `device_index: u64, flags: u64` | `DEVICES` |
| 0x821 | `sys_block_info` | `handle: u64, info_ptr: u64` | `READ` |
| 0x822 | `sys_block_read` | `handle: u64, start_block: u64, block_count: u16, buf_ptr: u64` | `READ` |
| 0x823 | `sys_block_write` | `handle: u64, start_block: u64, block_count: u16, buf_ptr: u64` | `WRITE` |
| 0x824 | `sys_block_sync` | `handle: u64` | `WRITE` |
| 0x825 | `sys_usb_enumerate` | `buffer: u64, capacity: u32` | `ENUMERATE` |

### 6.7 Red: UDP/IPv4 (`0x83x`)

| Nº | Syscall | Argumentos | `required_rights` |
| --- | --- | --- | --- |
| 0x830 | `sys_socket_open` | — | `NET_CONNECT` |
| 0x831 | `sys_socket_bind` | `socket: u64, port: u16` | `WRITE` |
| 0x832 | `sys_socket_sendto` | `socket: u64, address_ptr: u64, data_ptr: u64, length: u64` | `WRITE` |
| 0x833 | `sys_socket_recvfrom` | `socket: u64, data_ptr: u64, capacity: u64, address_ptr: u64` | `READ` |

**Total: 3 + 6 + 8 + 5 + 3 + 6 + 4 = 35.**

Notas de semántica:

- `sys_abi_version` devuelve `(major << 16) | minor`; hoy `0x00000001` (ABI 0.1).
- `sys_read_nonblocking` exige `handle == 0` hoy y nunca bloquea: devuelve `0` si no hay entrada (§10.4).
- `sys_write` solo acepta los handles `StdOut`/`StdErr`; sobre cualquier otro objeto devuelve `ENOSYS` (§10.4).
- `sys_socket_*` opera sobre UDP IPv4 no bloqueante: `sendto` devuelve `EAGAIN` si debe resolverse ARP y `recvfrom` si la cola está vacía. `SocketAddrV4` usa *layout* C de 16 bytes (`family: u16` = 2, `port_be: u16`, `address: [u8; 4]`, `reserved: [u8; 8]`).
- `sys_input_open` entrega eventos de la cola `drivers::input` con `class`: 0 = todos, 1 = teclado, 2 = puntero. Se leen con `sys_read` (arreglos de `InputEvent`, con alineación y tamaño comprobados).
- `sys_power` con `cmd` hoy ejecuta el apagado ordenado; la reinterpretación de `cmd` (`shutdown`/`reboot`) está pendiente.

## 7. IDs reservados sin implementar

Estos seis números **pertenecieron al ABI y fueron retirados** de `abi/syscalls.toml` porque no tienen handler en el kernel. Conservan su reserva: **no se reutilizan** (regla 5 de la §1), y una llamada a cualquiera de ellos devuelve `-ENOSYS`.

| Nº | Syscall | Grupo | Motivo de la retirada |
| --- | --- | --- | --- |
| 0x007 | `sys_revoker_create` | handles | no existe el objeto `Revoker` ni la revocación activa |
| 0x008 | `sys_revoker_revoke` | handles | ídem; sin *reverse mappings* ni TLB shootdown |
| 0x200 | `sys_vmo_create` | memoria | no hay manejo de VMOs en el dispatcher |
| 0x202 | `sys_vm_map` | memoria | no hay syscall de mapeo; el mapeo lo hace la carga de ELF |
| 0x700 | `sys_display_info` | pantalla | el grupo `0x07xx` está ocupado por la consola de texto; no hay subsistema de presentación |
| 0x702 | `sys_display_present` | pantalla | ídem (ADR-25 delega la presentación en `lix-drm`, aún inexistente) |

Además, **los números que no figuran en la §6 no forman parte del ABI**: no están asignados y deben considerarse libres hasta que se añadan al TOML con su handler. El diseño previsto para revocación, VMOs, pantalla y el resto de la superficie v1.0 (~75 llamadas) está en [capabilities.md](capabilities.md), [memory-map.md](memory-map.md) y [graphics.md](../design/graphics.md); ninguno de esos números se promete hasta que exista fila en `abi/syscalls.toml`.

## 8. Alias de Linux

El dispatcher canonicaliza el número **antes** de buscarlo en el registro (`kernel/src/task/syscall/mod.rs`):

| Número Linux | Equivalente LixOS | Syscall |
| --- | --- | --- |
| `60` (0x3C) | `0x101` | `sys_process_exit` |
| `35` | `0x107` | `sys_sleep` |
| `169` (0xA9) | `0x803` | `sys_power` |

- Solo estos tres alias existen. **No** hay alias de escritura: el número `1` **no** es `write`, es `sys_handle_close` (0x001).
- Cualquier otro número no listado en la §6 ni en esta tabla devuelve `-ENOSYS`.
- `rootfs/src/hello.c` sigue usando el número `1` para imprimir: su llamada de salida resuelve hoy a `sys_handle_close` y está **pendiente de corrección** (ver §10.6). Su `exit(42)` con el número `60` sí funciona gracias al alias.

## 9. Límites y validación de punteros

`validate_user_buffer(ptr, len, writeable)` (`kernel/src/task/syscall/mod.rs`):

| Condición | Resultado |
| --- | --- |
| `len == 0` | `Ok` sin comprobar el puntero |
| `ptr + len` desborda | `Err` → `EFAULT` |
| `ptr < USER_ADDRESS_START` (`0x0000_0000_0040_0000`) | `Err` |
| `end > USER_ADDRESS_END` (`0x0000_8000_0000_0000`) | `Err` |

Superado eso, recorre las *VMA* del proceso desde `ptr & !0xFFF` hasta `end`; si un tramo no tiene VMA, o tiene VMA sin `VmaFlags::WRITE` cuando `writeable` es `true`, devuelve `Err`.

Límites máximos declarados en `kernel/src/task/syscall/mod.rs`:

| Constante | Valor | Ámbito |
| --- | --- | --- |
| `MAX_PATH_BYTES` | 256 | rutas de `sys_process_spawn` y ficheros |
| `MAX_USER_IO_BYTES` | 64 MiB | búferes de `sys_read`, `sys_write`, `sys_read_nonblocking` |
| `MAX_PROCESS_INFO_ENTRIES` | 256 | `sys_process_list` |

## 10. Implementación actual

Comportamiento observado en `kernel/src/task/syscall/`, `kernel/src/task/syscall_table.rs`, `libs/lix-sys/src/` y `libs/sys-libc/src/lib.rs`. Los apartados siguientes describen hechos del código; no modifican la especificación de las secciones anteriores.

### 10.1 Registro y dispatcher

`kernel/src/task/syscall_table.rs` es un archivo **generado** con cabecera «NO EDITAR DIRECTAMENTE»: contiene `SYSCALL_REGISTRY` con las 35 filas de la §6 y `lookup_syscall(id)`. El dispatcher no contiene *matches* numéricos literales: compara contra `lix_sys::SYS_*`, que también son generados (`libs/lix-sys/src/syscalls.rs`, 35 constantes incluidas desde `lib.rs`). Ninguna constante `SYS_*` está escrita a mano fuera de ese fichero.

Los handlers viven en `kernel/src/task/syscall/{block,io,net,proc,term}.rs`; el dispatcher y la validación de punteros, en `mod.rs`.

### 10.2 Cobertura

Los **35** IDs de la §6 tienen fila en el registro **y** brazo en el dispatcher: no hay IDs declarados sin handler, ni handlers sin fila. Un número sin fila registra `[syscall] Syscall no implementada: {:#x}` y devuelve `-ENOSYS`.

### 10.3 `sys_abi_version`

`0x000` devuelve `0x00000001` (mayor `0`, menor `1`), consistente con `version = "0.1.0"` del TOML.

### 10.4 Lectura y escritura estándar

- `sys_read` (`0x301`): con `handle == 0` drena la serie hacia `drivers::tty::STDIN` y bloquea (estado `Blocked` + `yield`) hasta leer al menos un carácter o morir el proceso; si el búfer admite menos de un carácter UTF-8 completo, devuelve lo leído. Con un handle `InputDevice` devuelve arreglos de `InputEvent` (exige alineación y tamaño múltiplos). Con cualquier otro handle exige el derecho `READ`.
- `sys_write` (`0x302`): exige el derecho `WRITE` en el handle y que el objeto sea `StdOut` o `StdErr`; imprime con `kprint!`. Otros objetos → `ENOSYS`.
- `sys_read_nonblocking` (`0x309`): exige `handle == 0` (otros → `EINVAL`), no bloquea jamás y devuelve `0` si no hay entrada; descarga la serie antes de leer y reinserta con `push_front` el carácter que no cabe.

### 10.5 Observabilidad: `sys_monitor_metrics`

`0x80A` copia un `lix_sys::SystemMetrics` (§2) hacia el puntero de usuario con `validate_user_buffer(write = true)` y `copy_to_user`; cualquier fallo → `EFAULT`.

### 10.6 Divergencias conocidas con `abi/syscalls.toml`

| Syscall | `abi/syscalls.toml` | Kernel |
| --- | --- | --- |
| `sys_process_spawn` (`0x100`) | un argumento `spec_ptr: u64` (implícitamente un `SpawnSpec` completo) | el dispatcher pasa `(path_ptr, path_len)` y el handler lee **solo la ruta**, acotada a `MAX_PATH_BYTES` (256), con `argv`, handles y Job fuera de alcance |
| `sys_power` (`0x803`) | `cmd: u64` con semántica `shutdown`/`reboot` | el handler ignora `cmd` y ejecuta siempre `power::system_shutdown()` |
| `sys_read_nonblocking` (`0x309`) | `handle: u64` genérico | restringido a `handle == 0` (entrada estándar) |
| `sys_socket_*` (`0x830`–`0x833`) | UDP/IPv4 no bloqueante | handlers implementados y `BASE_USER` incluye `NET_CONNECT`, pero la pila no está validada end-to-end en QEMU y no hay resolver, TCP ni IPv6 |
| — | — | `rootfs/src/hello.c` llama al número `1` esperando una escritura (§8) |

### 10.7 `ProcessInfo`

La estructura de la §2 tiene **siete** campos; el séptimo, `disk_read_bytes`, se acumula en `vfs::record_disk_read` por cada lectura física del sistema montado y no está en el diseño original. `sys_process_list` limita la salida a `MAX_PROCESS_INFO_ENTRIES` (256) entradas.

## 11. Pruebas del ABI

1. **Tests de contrato** por syscall: éxito y cada error documentado (`kernel/src/task/handle.rs`, `libs/lix-sys/src/lib.rs`).
2. **Tests de derechos:** cada syscall con un proceso al que le falta justo el derecho exigido (el enforcement de la §4 es comprobable sin hardware).
3. **Fuzzer de syscalls** generado desde `abi/syscalls.toml`: argumentos aleatorios válidos, sin `panic`, sin fugas (ADR-19) y errores consistentes. *Pendiente: no existe todavía.*
4. **Tests de punteros hostiles:** punteros del kernel, sin mapear, parcialmente mapeados, que cruzan el límite o se desmapean durante la llamada. *Pendiente.*
5. **Deriva del ABI:** el job `codegen-drift` de CI regenera los artefactos y exige `git diff --exit-code` (§5). *Implementado.*
6. Medición de latencia por syscall en CI (SLO de ADR-14). *Pendiente.*
