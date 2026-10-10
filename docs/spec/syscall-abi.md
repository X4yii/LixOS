# ABI de syscalls

> **Tipo:** especificación · **Estado:** borrador · **ABI:** `0.x` · **ADR:** 03, 05, 22, 24
> **Relacionado:** [Capabilities](capabilities.md) · [Gráficos](../design/graphics.md)

Define la convención de llamada, los tipos base, los códigos de error y la tabla completa de syscalls del ABI nativo. Hasta `1.0.0` el ABI puede cambiar; desde `1.0.0` queda congelado (ADR-17).

## 1. Convención de llamada

Instrucción `SYSCALL`, ABI System V adaptada a x86_64:

| Registro | Uso |
| --- | --- |
| `rax` | número de syscall (entrada) y resultado (salida) |
| `rdi, rsi, rdx, r10, r8, r9` | argumentos 1 a 6 |
| `rcx, r11` | destruidos por `SYSCALL`/`SYSRET` |
| resto | preservados por el kernel |

**Resultado:** `rax >= 0` es éxito (el valor depende de la syscall). `rax < 0` es `-código_de_error`.

### Reglas generales (obligatorias para todas las syscalls)

1. **Punteros de usuario se validan y se copian una sola vez** al kernel (`copy_from_user`), y no se vuelven a leer. Elimina TOCTOU.
2. **Estructuras versionadas:** toda estructura que cruza la frontera empieza con `u32 size`. El kernel acepta tamaños mayores o menores conocidos, así se pueden añadir campos sin romper binarios.
3. **Flags desconocidos se rechazan** (`EINVAL`). Nunca se ignoran en silencio, así los programas no dependen de bits sin significado.
4. **Todo bloqueo tiene plazo.** Las operaciones que esperan reciben un `deadline` absoluto en ns del reloj monotónico (`0` = no bloquear, `u64::MAX` = sin límite). Ningún bloqueo es ininterrumpible: `process_kill` siempre puede despertar al hilo.
5. **Los números de syscall nunca se reutilizan.** Una syscall obsoleta sigue funcionando después de 1.0.
6. **Límites máximos documentados** para cada longitud (por ejemplo, ruta ≤ 4096 bytes, ≤ 64 handles por mensaje).
7. **Ninguna syscall asigna memoria sin cobrarla** al proceso que la pide (ADR-19) y todas pueden devolver `ENOMEM`.
8. **Fallo parcial limpio:** si una syscall falla, no deja recursos a medias (los handles creados se cierran).

## 2. Tipos base

```rust
pub type Handle   = u64;   // 32 bits generación | 32 bits índice (ADR-05). 0 = inválido
pub type Rights   = u32;   // máscara de derechos
pub type Deadline = u64;   // ns monotónico absoluto
pub type Signals  = u32;   // señales de objeto (READABLE, WRITABLE, TERMINATED, ...)
```

Los derechos están definidos en [capabilities.md](capabilities.md).

## 3. Códigos de error

Se usan los **números de `errno` de Linux** (ADR-22) para que la libc no tenga que traducir. Códigos propios a partir de `1000`:

| Código | Nombre | Significado | `errno` que ve C |
| --- | --- | --- | --- |
| 1 | `EPERM` | operación no permitida | `EPERM` |
| 2 | `ENOENT` | no existe | `ENOENT` |
| 4 | `EINTR` | interrumpido | `EINTR` |
| 9 | `EBADF` | handle inválido o de tipo incorrecto | `EBADF` |
| 11 | `EAGAIN` | no disponible ahora | `EAGAIN` |
| 12 | `ENOMEM` | sin memoria o cuota agotada | `ENOMEM` |
| 13 | `EACCES` | derechos insuficientes | `EACCES` |
| 14 | `EFAULT` | puntero de usuario inválido | `EFAULT` |
| 22 | `EINVAL` | argumento o flag inválido | `EINVAL` |
| 38 | `ENOSYS` | syscall no implementada | `ENOSYS` |
| 110 | `ETIMEDOUT` | venció el `deadline` | `ETIMEDOUT` |
| 1001 | `ELIX_REVOKED` | handle revocado | `EBADF` |
| 1002 | `ELIX_LIMIT` | límite de recursos del proceso/Job | `EMFILE`/`EAGAIN` |
| 1003 | `ELIX_PEER_CLOSED` | el otro extremo se cerró | `EPIPE` |

## 4. Tabla de syscalls de v1.0

Convención de nombres: `sys_<objeto>_<acción>`. Todas pueden devolver `EFAULT`, `EINVAL`, `ENOMEM` y `ENOSYS`, no se repiten abajo.

### 4.1 Núcleo: handles y espera (`0x00xx`)

| Nº | Syscall | Argumentos | Derecho necesario | Notas |
| --- | --- | --- | --- | --- |
| 0x000 | `sys_abi_version` | — | — | devuelve `(major<<16 \| minor)` |
| 0x001 | `sys_handle_close` | `h` | — | libera el handle y, si era el último, el objeto |
| 0x002 | `sys_handle_dup` | `h, rights` | `DUP` | `rights ⊆ derechos de h` (solo atenuar) |
| 0x003 | `sys_handle_replace` | `h, rights` | — | como dup pero cierra el original |
| 0x004 | `sys_handle_info` | `h, *info` | — | tipo, derechos, id de objeto |
| 0x005 | `sys_wait_one` | `h, signals, deadline, *observed` | `WAIT` | espera señales de un objeto |
| 0x006 | `sys_wait_many` | `*items, n, deadline` | `WAIT` cada uno | base de `poll`/`select`/`epoll` |
| 0x007 | `sys_revoker_create` | — | — | devuelve handle de revocación |
| 0x008 | `sys_revoker_revoke` | `rev` | — | invalida todos los handles derivados |
| 0x009 | `sys_handle_dup_revocable` | `h, rights, rev` | `DUP` | handle atado al revoker |

### 4.2 Procesos, hilos y Jobs (`0x01xx`)

| Nº | Syscall | Argumentos | Derecho | Notas |
| --- | --- | --- | --- | --- |
| 0x100 | `sys_process_spawn` | `*spec` → `proc_h` | `SPAWN` en el Job | ver 5 |
| 0x101 | `sys_process_exit` | `code` | — | no retorna |
| 0x102 | `sys_process_kill` | `proc_h, code` | `MANAGE` | sigue la secuencia de terminación (ADR-20) |
| 0x103 | `sys_process_info` | `proc_h, *info` | `INSPECT` | estado, código de salida, memoria |
| 0x104 | `sys_thread_create` | `proc_h, *spec` → `th_h` | `MANAGE` | entrada, stack, TLS |
| 0x105 | `sys_thread_exit` | — | — | no retorna |
| 0x106 | `sys_thread_yield` | — | — | |
| 0x107 | `sys_thread_set_priority` | `th_h, prio` | `MANAGE` | limitado por la clase del Job |
| 0x108 | `sys_job_create` | `parent_job` → `job_h` | `MANAGE` | |
| 0x109 | `sys_job_kill` | `job_h` | `MANAGE` | termina todo el árbol |
| 0x10A | `sys_job_set_limits` | `job_h, *limits` | `MANAGE` | memoria, hilos, handles |
| 0x10B | `sys_vfork_spawn` | `*spec` | `SPAWN` | base de `vfork` en libc (ADR-22) |
| 0x10C | `sys_process_list` | `*buf, max` → `n` | — | vuelca un arreglo de `ProcessInfo` con los procesos vivos |

`ProcessInfo` es la estructura devuelta por `sys_process_list`:

```rust
#[repr(C)]
pub struct ProcessInfo {
    pub pid: u64,
    pub name: [u8; 16],        // truncado a 16 bytes
    pub state: u8,              // 0 Ready, 1 Running, 2 Blocked, 3 Sleeping, 4 Zombie, 5 Dead
    pub privilege_level: u8,
    pub cpu_ticks: u64,
    pub ram_bytes: u64,
}
```

### 4.3 Memoria (`0x02xx`)

| Nº | Syscall | Argumentos | Derecho | Notas |
| --- | --- | --- | --- | --- |
| 0x200 | `sys_vmo_create` | `size, flags` → `vmo_h` | — | objeto de memoria (anónimo) |
| 0x201 | `sys_vmo_resize` | `vmo_h, size` | `WRITE` | |
| 0x202 | `sys_vm_map` | `vmo_h \| 0, addr_hint, len, prot, flags` → `addr` | `MAP` | `vmo_h=0` = anónimo; archivo con handle de archivo |
| 0x203 | `sys_vm_unmap` | `addr, len` | — | |
| 0x204 | `sys_vm_protect` | `addr, len, prot` | — | W^X: nunca `W` y `X` a la vez (`EINVAL`) |
| 0x205 | `sys_vm_info` | `*info` | — | cuota, uso (para `getrlimit`/diagnóstico) |

### 4.4 Archivos y directorios (`0x03xx`)

Los nombres son siempre relativos a un handle de directorio (ADR-05, ADR-22).

| Nº | Syscall | Argumentos | Derecho | Notas |
| --- | --- | --- | --- | --- |
| 0x300 | `sys_open_at` | `dir_h, path, flags, rights` → `h` | `TRAVERSE` en `dir_h` | `..` y rutas absolutas resuelven **dentro** de `dir_h` (no puede escapar) |
| 0x301 | `sys_read` | `h, buf, len` | `READ` | usa el cursor del handle |
| 0x302 | `sys_write` | `h, buf, len` | `WRITE` | |
| 0x303 | `sys_pread` | `h, buf, len, off` | `READ` | |
| 0x304 | `sys_pwrite` | `h, buf, len, off` | `WRITE` | |
| 0x305 | `sys_seek` | `h, off, whence` | — | |
| 0x306 | `sys_stat` | `h, *st` | — | `struct stat` compatible con Linux (ADR-22) |
| 0x307 | `sys_stat_at` | `dir_h, path, *st, flags` | `TRAVERSE` | |
| 0x308 | `sys_readdir` | `dir_h, buf, len` | `ENUMERATE` | |
| 0x309 | `sys_mkdir_at` | `dir_h, path, mode` | `WRITE` | |
| 0x30A | `sys_unlink_at` | `dir_h, path, flags` | `WRITE` | |
| 0x30B | `sys_rename_at` | `d1, p1, d2, p2` | `WRITE` ambos | |
| 0x30C | `sys_truncate` | `h, size` | `WRITE` | |
| 0x30D | `sys_sync` | `h` | — | `fsync`; con handle de FS, `sync` global |
| 0x30E | `sys_chmod`, `sys_chown` | `h, ...` | `WRITE` | guardan metadatos en ext4 (ADR-05) |
| 0x30F | `sys_pipe_create` | → `h_read, h_write` | — | |
| 0x310 | `sys_channel_create` | → `h_a, h_b` | — | mensajes + transferencia de handles |
| 0x311 | `sys_channel_send` | `ch, *bytes, n, *handles, nh` | `WRITE` + `TRANSFER` por handle | los handles se **mueven** |
| 0x312 | `sys_channel_recv` | `ch, *buf, n, *handles, nh, deadline` | `READ` | |
| 0x313 | `sys_namespace_create` | → `ns_h` | — | ver capabilities.md |
| 0x314 | `sys_namespace_bind` | `ns, path, dir_h, rights` | `MANAGE` | |
| 0x315 | `sys_namespace_derive` | `ns, *subset` → `ns2` | `DUP` | solo reduce |
| 0x316 | `sys_control` | `h, cmd, *arg` | según `cmd` | reemplaza `ioctl` (termios, etc.) |

### 4.5 Tiempo y sincronización (`0x04xx`, `0x05xx`)

| Nº | Syscall | Argumentos | Notas |
| --- | --- | --- | --- |
| 0x400 | `sys_clock_get` | `clock_id, *ns` | `MONOTONIC`, `REALTIME`, `PROCESS_CPU` |
| 0x401 | `sys_timer_create` | `clock, flags` → `timer_h` | se espera con `wait_one` |
| 0x402 | `sys_timer_set` | `timer_h, deadline, period` | |
| 0x500 | `sys_event_create` | → `ev_h` | |
| 0x501 | `sys_event_signal` | `ev_h, set, clear` | `SIGNAL` |
| 0x502 | `sys_futex_wait` | `*addr, expected, deadline` | base de `pthread` |
| 0x503 | `sys_futex_wake` | `*addr, n` | |

`sleep`/`nanosleep` se implementan en la libc con `sys_wait_one` sobre un timer, no hay syscall propia.

### 4.6 Notificaciones (señales) (`0x06xx`)

| Nº | Syscall | Argumentos | Notas |
| --- | --- | --- | --- |
| 0x600 | `sys_notify_set_handler` | `entry, stack` | un *trampoline* de la libc por proceso |
| 0x601 | `sys_notify_send` | `proc_h, kind` | derecho `SIGNAL` |
| 0x602 | `sys_notify_return` | `*frame` | vuelve al código interrumpido |

Las faltas de hardware (fallo de página, división por cero) llegan al proceso como notificación (`SIGSEGV`, `SIGFPE`...) si hay handler. Si no, el proceso termina con `EXIT_FAULT` (ADR-20). Detalle de máscaras en el documento de la libc.

### 4.7 Pantalla y entrada (`0x07xx`), contrato en [graphics.md](../design/graphics.md)

| Nº | Syscall | Argumentos | Derecho |
| --- | --- | --- | --- |
| 0x700 | `sys_display_info` | `display_h, *info` | — |
| 0x701 | `sys_display_acquire` | `display_h` | `DISPLAY_OWN` |
| 0x702 | `sys_display_present` | `display_h, vmo_h, *desc, *rects, n` | `DISPLAY_OWN` |
| 0x703 | `sys_display_release` | `display_h` | `DISPLAY_OWN` |
| 0x704 | `sys_display_wait_frame` | `display_h, deadline` | `WAIT` |
| 0x705 | `sys_input_open` | `class` → `h` | `INPUT_READ` |

Los eventos de entrada se leen con `sys_read` sobre el handle (arreglos de `InputEvent`).

### 4.8 Sistema (`0x08xx`)

| Nº | Syscall | Derecho | Notas |
| --- | --- | --- | --- |
| 0x800 | `sys_system_info` | — | versión, RAM total, CPUs, uptime |
| 0x801 | `sys_log_write` | `LOG` | escribe en klog |
| 0x802 | `sys_random_get` | — | entropía (RDSEED/RDRAND + pool) |
| 0x803 | `sys_power` | `POWER` | `shutdown`, `reboot` |
| 0x804 | `sys_block_open` | `DEVICES` | handle de disco (instalador) |
| 0x805 | `sys_auth_verify` | `AUTH` | solo `authd`; delega en el servicio |

Total aproximado en v1.0: **~75 syscalls**. Es poco a propósito: menos superficie, menos bugs.

## 5. `sys_process_spawn`

```rust
#[repr(C)]
pub struct SpawnSpec {
    pub size: u32,            // sizeof(SpawnSpec)
    pub flags: u32,
    pub job: Handle,          // Job donde nace (0 = el del padre)
    pub root: Handle,         // namespace raíz del hijo (0 = hereda el del padre)
    pub path_ptr: u64, pub path_len: u32,
    pub argv_ptr: u64, pub argc: u32,
    pub envp_ptr: u64, pub envc: u32,
    pub handles_ptr: u64, pub handles_len: u32,   // [HandleDesc]
    pub cwd_ptr: u64, pub cwd_len: u32,
}

#[repr(C)]
pub struct HandleDesc {
    pub slot: u32,            // número que tendrá en el hijo (0,1,2 = stdin/out/err)
    pub rights: u32,          // ⊆ derechos del handle original
    pub handle: Handle,       // del padre; se DUPLICA al hijo
}
```

- El padre elige qué hereda el hijo. Nada se hereda por defecto, salvo la raíz.
- `slot` 0, 1 y 2 son la entrada, salida y error estándar para la libc.
- La carga del ELF y la construcción del espacio de direcciones ocurren **antes** de crear el hilo inicial, y un fallo limpia todo (regla 8).
- `sys_vfork_spawn` bloquea al padre hasta que el hijo hace `exec` o termina, compartiendo el espacio de direcciones.

## 6. Eventos de entrada

```rust
#[repr(C)]
pub struct InputEvent {
    pub time_ns: u64,
    pub device: u16,
    pub kind: u16,    // KEY, REL, ABS, BUTTON, ...
    pub code: u32,    // keycode físico, no carácter
    pub value: i32,
}
```

El kernel entrega **keycodes físicos**. La traducción a caracteres (layout US, ES) la hace userspace con tablas de teclado. La consola de texto del kernel usa una tabla mínima propia.

## 7. Versionado del ABI (ADR-17)

- `sys_abi_version` devuelve `major.minor`. `lix-sys` y la libc lo comprueban al arrancar y fallan con un mensaje claro si el kernel es más antiguo.
- Hasta `1.0`: cualquier cambio sube `minor` y se anota en `CHANGELOG`.
- Desde `1.0`: solo se **añaden** syscalls y campos nuevos al final de las estructuras. Los cambios incompatibles requieren `2.0`.
- vDSO para `sys_clock_get`: se decide con mediciones frente al SLO de 300 ns (ADR-14).

## 8. Pruebas del ABI

1. **Fuzzer de syscalls** que genera argumentos aleatorios y válidos según `syscalls.toml`, y verifica: sin `panic`, sin fugas (línea base de ADR-19), y errores consistentes.
2. **Tests de contrato**: cada syscall tiene casos de éxito y de cada error documentado.
3. **Tests de punteros hostiles**: punteros del kernel, sin mapear, parcialmente mapeados, que cruzan el límite del espacio de usuario, que se desmapean durante la llamada.
4. **Tests de derechos**: cada syscall se prueba con un handle al que le falta justo el derecho exigido.
5. Medición de latencia por syscall en CI (SLO de ADR-14).

## 9. Fuente única del ABI (ADR-24)

El archivo **`abi/syscalls.toml`** define números, nombres, argumentos, tipos, derechos y errores posibles. De él se **generan**: la tabla del kernel, `lix-sys` (Rust), `<lix/syscall.h>` (C), esta documentación y el fuzzer. Nadie escribe una syscall a mano en dos sitios.

## 10. Implementación actual

Comportamiento observado en `kernel/src/task/syscall.rs`, `libs/lix-sys/src/lib.rs` y `libs/sys-libc/src/lib.rs`. Los apartados siguientes describen hechos del código; no modifican la especificación de las secciones anteriores.

### 10.1 Validación de punteros

`validate_user_buffer(ptr, len, write)` delega en `memory::vmm::validate_user_range` (`kernel/src/memory/vmm.rs`):

| Condición | Resultado |
| --- | --- |
| `len == 0` | `Ok` sin comprobar el puntero |
| `ptr + len` desborda | `Err("Rango de usuario desbordado")` |
| `ptr < 0x1000` (página cero) | `Err("Rango fuera del espacio de usuario")` |
| `end > 0x0000_8000_0000_0000` (`USER_ADDRESS_LIMIT`) | idem |
| `end <= ptr` | idem |

Superadas esas comprobaciones, recorre las cuatro tablas de páginas desde el `CR3` actual, de la entrada PML4 a la de página, y exige en cada nivel `PAGE_PRESENT | PAGE_USER`; si `write` es `true`, exige además `PAGE_WRITABLE`. Cualquier nivel que no cumpla produce `Err("Página de usuario no accesible")`.

El límite superior coincide con el inicio del hueco no canónico definido en [memory-map.md](memory-map.md).

### 10.1.1 Límites de longitud

| Constante | Valor | Syscalls |
| --- | --- | --- |
| `MAX_PATH_BYTES` | 4096 | `0x100`, `0x306`, `0x307` |
| `MAX_USER_IO_BYTES` | 1 MiB | `0x001`, `0x301`, `0x302`, `0x306`, `0x307` |
| `MAX_PROCESS_INFO_ENTRIES` | 256 | `0x10C` |

### 10.2 Camino de interrupción `int 0x80`

El dispatcher se registra en dos vías de entrada hacia la misma función `syscall_dispatcher`:

| Vía | Mecanismo | Registro del número | Registros de argumentos |
| --- | --- | --- | --- |
| Primaria | `SYSCALL` / `SYSRET` (MSRs `EFER.SCE`, `STAR`, `LSTAR`, `FMASK`) | `rdi` (tras el remapeo del entry asm) | `rsi, rdx, rcx, r8, r9` |
| Secundaria | `int 0x80`, vector `0x80` con DPL Ring 3 | `rax` | `rdi, rsi, rdx, r10, r8` |

El camino de interrupción conserva la convención de registros del camino `SYSCALL` (argumentos en `rdi…r9`), **no** la convención `ebx/ecx/edx/esi/edi` de Linux.

Compatibilidad de numeración presente en el dispatcher:

- `0x001` con la forma de argumentos `write(fd, ptr, len)` (`arg2 >= 0x1000 && arg3 > 0`) → escritura por consola.
- `35` y `0x107` → `sys_sleep`.
- `60` y `0x101` → `sys_exit`.
- `169` y `0x803` → `sys_power`.
- `0x302` (`sys_write`) admite la forma de argumentos con el buffer en `arg1`.

`rootfs/src/hello.c` usa este camino con los números `1` y `60`.

### 10.3 `sys_terminal_info`

Estructura devuelta por la consulta de geometría de la consola (`kernel/src/drivers/framebuffer/mod.rs`):

```rust
#[repr(C)]
pub struct TerminalInfo {
    pub cols: u16,
    pub rows: u16,
    pub pixel_width: u16,
    pub pixel_height: u16,
    pub font_width: u8,
    pub font_height: u8,
    pub scale: u8,
    pub _reserved: u8,
}
```

Devuelve `ENODEV` si no hay framebuffer. Está declarada como `SYS_TERMINAL_INFO` en `lix-sys` y se enruta en el dispatcher en el número `0x703`.

### 10.4 Número de versión y errores

- `sys_abi_version` (`0x000`) devuelve `0x00000001`.
- Un número de syscall sin handler registra el aviso `[syscall] Syscall no implementada` y devuelve `0xFFFF_FFFF_FFFF_FFFF`.
- `sys_process_spawn` (`0x100`) recibe `(path_ptr, path_len)` y limita la ruta a 4096 bytes; `abi/syscalls.toml` lo declara con `spec_ptr`.
- `sys_read` (`0x301`) atiende la cola estándar de entrada; `sys_write` (`0x302`) escribe por `kprint!`.

### 10.5 Disparador implementado

`kernel/src/task/syscall.rs` enruta los siguientes números:

| Número | Handler | Observación |
| --- | --- | --- |
| `0x000` | `sys_abi_version` | devuelve `0x00000001` |
| `0x001` | `sys_handle_close` / escritura por consola | selección por forma de los argumentos (§10.2) |
| `0x002` | `sys_handle_dup` | |
| `0x100` | `sys_process_spawn` | ruta acotada a 4096 bytes |
| `0x101` / `60` | `sys_exit` | |
| `0x102` | `sys_kill` | valida `signal < 64`; solo 2, 9 y 15 tienen efecto (§10.9) |
| `0x107` / `35` | `sys_sleep` | |
| `0x10A` | `sys_waitpid` | |
| `0x10C` | `sys_process_list` | |
| `0x301` | `sys_read` | cola estándar de entrada |
| `0x302` | `sys_write` | `kprint!` |
| `0x306` | `sys_list_dir` | |
| `0x307` | `sys_read_file` | |
| `0x309` | `sys_read_nonblocking` | lectura no bloqueante para sondeo TUI; solo `fd == 0` (§10.11) |
| `0x703` | `sys_terminal_info` | geometría de la consola (§10.3) |
| `0x704` | `sys_terminal_set_scale` | argumentos `0` (reinicio), `1`–`3` (escala), `10` (acercar), `11` (alejar) |
| `0x706` | `sys_set_keymap` | `layout_id` |
| `0x707` | `sys_get_keymap` | devuelve el identificador de la disposición activa |
| `0x803` / `169` | `sys_power` | secuencia de apagado (ver [storage.md](../design/storage.md) §6) |
| `0x80A` | `sys_monitor_metrics` | vuelca un `lix_sys::SystemMetrics` (§10.12) |

En el grupo `0x07xx` se enrutan funciones de consola de texto, mientras que la §4.7 asigna esos números a `sys_display_release` (`0x703`) y `sys_display_wait_frame` (`0x704`).

En el grupo `0x03xx`, el número `0x309` corresponde en la §4.4 a `sys_mkdir_at` (`dir_h, path, mode`, derecho `WRITE`), mientras que el dispatcher lo enruta a `sys_read_nonblocking`. `libs/lix-sys` lo declara como `SYS_READ_NONBLOCK`.

`libs/lix-sys/src/lib.rs` declara constantes de número para las filas de la §4 y además `SYS_WAITPID` (`0x10A`), `SYS_TERMINAL_INFO` (`0x703`), `SYS_READ_FILE` (`0x307`) y `SYS_LIST_DIR` / `SYS_STAT` (ambas `0x306`). En este grupo, `0x10A`, `0x306` y `0x307` coinciden con filas de la §4 asignadas a `sys_job_set_limits`, `sys_stat` y `sys_stat_at` respectivamente; `SYS_LIST_DIR` y `SYS_STAT` comparten el mismo número.

### 10.6 Fuente única del ABI

`abi/syscalls.toml` (99 líneas) declara `[abi]` con `version = "0.1.0"`, `arch = "x86_64"` y `calling_convention = "sysv_registers"`, más 12 entradas `[[syscall]]`: `0x000`, `0x001`, `0x002`, `0x007`, `0x008`, `0x100`, `0x101`, `0x200`, `0x202`, `0x700`, `0x702` y `0x801`. Cada una lleva `id`, `name`, `description`, `args` y `rights`.

Estado de la cadena descrita en la §9:

| Artefacto | Estado |
| --- | --- |
| Tabla del kernel | escrita a mano en `kernel/src/task/syscall.rs` |
| `libs/lix-sys` | constantes escritas a mano (46 `SYS_*`) |
| `<lix/syscall.h>` | no existe en el árbol (no hay ningún fichero `.h`) |
| Documentación (§4) | escrita a mano |
| Fuzzer | no existe |

No hay ningún programa que lea `abi/syscalls.toml`: no aparece referenciado en `xtask/`, `tools/`, `kernel/` ni `libs/`. El archivo coexiste con las tres fuentes anteriores sin ser su origen.

### 10.7 Divergencias con `abi/syscalls.toml`

| Syscall | `abi/syscalls.toml` | Kernel |
| --- | --- | --- |
| `sys_process_spawn` (`0x100`) | `spec_ptr: u64` | `(path_ptr, path_len)`, ruta acotada a 4096 bytes |
| `sys_log_write` (`0x801`) | búfer + longitud sobre klog | sin handler |
| `sys_vmo_create` (`0x200`), `sys_vm_map` (`0x202`) | declaradas | sin handler |
| `sys_revoker_create` (`0x007`), `sys_revoker_revoke` (`0x008`) | declaradas | sin handler |
| `sys_display_info` (`0x700`), `sys_display_present` (`0x702`) | declaradas | sin handler |

Los números con handler que también figuran en el TOML (`0x000`, `0x001`, `0x002`, `0x101`) coinciden en nombre y argumentos.

### 10.8 `sys_abi_version`

`0x000` devuelve `0x00000001`, mientras que `abi/syscalls.toml` declara `version = "0.1.0"` y `libs/lix-sys/src/lib.rs` declara `SYS_ABI_VERSION: usize = 0x000`. Los tres números (retorno, TOML y constante) no usan la misma codificación.

### 10.9 Señales y códigos de retorno

**Señales.** `sys_kill` (`0x102`) rechaza con `EINVAL` solo cuando `signal >= 64` (`u64::BITS`). Antes de enviar comprueba el privilegio: el llamante debe tener `current_priv <= privilege_level` del objetivo (el número menor denota más privilegio).

`scheduler::send_signal` (`kernel/src/task/scheduler.rs`) hace `pending_signals |= 1 << signal`. Solo tres señales tienen efecto observable:

| Señal | Efecto |
| --- | --- |
| `9` | `state = Dead` de inmediato y limpieza de `FOREGROUND_TASK` |
| `2` | si el proceso está `Blocked` pasa a `Ready`; el scheduler lo detecta en `pending_signals & ((1 << 15) | (1 << 2))` |
| `15` | idéntico a la 2 |

Las demás señales (0–63 salvo estas tres) se registran en `pending_signals` sin consumidor.

**Códigos de retorno.** El dispatcher devuelve los errores como `-(errno as i32) as u64`. Una excepción: `sys_kill` devuelve `1` (`EPERM`) y `2` (`ENOENT`) **positivos** cuando el proceso no existe o no hay privilegio.

**Catálogo completo.** `lix_sys::ErrorCode` declara 23 valores; la §3 recoge 14 de ellos. Ausentes en la §3: `ESRCH = 3`, `EIO = 5`, `ENOEXEC = 8`, `EBUSY = 16`, `EEXIST = 17`, `ENODEV = 19` y `EMFILE = 24`.

### 10.10 Constantes declaradas sin handler

`libs/lix-sys/src/lib.rs` declara 48 constantes `SYS_*` que resuelven a 47 números distintos (`SYS_STAT` y `SYS_LIST_DIR` valen ambas `0x306`). De ellas, **18 tienen handler**: `0x000`, `0x001`, `0x002`, `0x100`, `0x101`, `0x102`, `0x10A`, `0x301`, `0x302`, `0x306`, `0x307`, `0x309`, `0x703`, `0x704`, `0x706`, `0x707`, `0x803` y `0x80A`.

**31 no lo tienen**:

| Grupo | Constantes |
| --- | --- |
| Handles (`0x00xx`) | `SYS_HANDLE_REPLACE` (3), `SYS_HANDLE_INFO` (4), `SYS_WAIT_ONE` (5), `SYS_WAIT_MANY` (6), `SYS_REVOKER_CREATE` (7), `SYS_REVOKER_REVOKE` (8), `SYS_HANDLE_DUP_REVOCABLE` (9) |
| Procesos (`0x10x`) | `SYS_PROCESS_INFO` (3), `SYS_THREAD_CREATE` (4), `SYS_THREAD_EXIT` (5), `SYS_THREAD_YIELD` (6), `SYS_JOB_CREATE` (8), `SYS_JOB_KILL` (9) |
| Memoria (`0x20x`) | `SYS_VMO_CREATE`, `SYS_VMO_RESIZE`, `SYS_VM_MAP`, `SYS_VM_UNMAP`, `SYS_VM_PROTECT`, `SYS_VM_INFO` |
| Ficheros (`0x30x`) | `SYS_OPEN_AT` (0), `SYS_SEEK` (5), `SYS_NAMESPACE_CREATE` (13), `SYS_NAMESPACE_BIND` (14) |
| Gráficos (`0x70x`) | `SYS_DISPLAY_INFO` (0), `SYS_DISPLAY_ACQUIRE` (1), `SYS_DISPLAY_PRESENT` (2), `SYS_INPUT_OPEN` (5) |
| Sistema (`0x80x`) | `SYS_SYSTEM_INFO` (0), `SYS_LOG_WRITE` (1) |

Un número sin handler registra `[syscall] Syscall no implementada` y devuelve `0xFFFF_FFFF_FFFF_FFFF`.

En sentido inverso, hay **handlers sin constante** en `lix-sys`: `0x107` (`sys_sleep`), `0x10C` (`sys_process_list`) y las señales de Linux `35`, `60` y `169`. Por eso `libs/sys-libc/src/lib.rs` escribe esos números a mano: `syscall1(0x107, ms)` y `syscall2(0x10C, …)`.

### 10.11 `sys_read_nonblocking`

`0x309`, declarada como `SYS_READ_NONBLOCK` en `lix-sys` y expuesta en `sys-libc` como `sys_read_nonblocking(fd, buf) -> usize`.

Comportamiento (`kernel/src/task/syscall.rs`):

1. Exige `fd == 0`; cualquier otro descriptor devuelve `EINVAL`. Igualmente `EINVAL` si `len > MAX_USER_IO_BYTES` (1 MiB) o si `validate_user_buffer(buffer, len, write = true)` falla.
2. `len == 0` devuelve `0`.
3. Drena el puerto serie hacia la cola estándar (`drivers::serial::drain_to_stdin`) para que los bytes pendientes entren en la cola antes de leer.
4. Vacía `drivers::tty::STDIN` copiando cada carácter codificado en UTF-8 al búfer de usuario, hasta que la siguiente escritura exceda la longitud; en ese caso el carácter se devuelve a la cola con `push_front`.
5. Devuelve el número de bytes escritos. Si no hay entrada, devuelve `0`: la llamada nunca bloquea.

La llamada bloqueante equivalente sigue siendo `sys_read` (`0x301`).

### 10.12 Observabilidad: `sys_monitor_metrics`

`0x80A`, declarada como `SYS_MONITOR_METRICS` en `lix-sys` y expuesta en `sys-libc` como `sys_system_metrics(&mut SystemMetrics) -> i32`.

```rust
#[repr(C)]
pub struct SystemMetrics {
    pub scheduler_ticks: u64,   // SCHEDULER.total_ticks
    pub total_ram_bytes: u64,   // pmm::memory_stats().0
    pub free_ram_bytes: u64,    // pmm::memory_stats().1
    pub disk_read_ops: u64,     // vfs::disk_io_stats().read_ops
    pub disk_read_bytes: u64,   // vfs::disk_io_stats().read_bytes
    pub disk_mounted: u64,      // vfs::disk_io_stats().mounted como 0/1
}
```

El puntero se valida con `validate_user_buffer(…, write = true)` y se copia con `copy_to_user`; cualquier fallo devuelve `EFAULT`.

### 10.13 `ProcessInfo`

La estructura devuelta por `sys_process_list` (`0x10C`) tiene hoy **siete** campos. La definición de la §4.2 no incluye el último:

| Campo | Tipo | Notas |
| --- | --- | --- |
| `pid` | `u64` | |
| `name` | `[u8; 16]` | truncado a 16 bytes |
| `state` | `u8` | 0 Ready, 1 Running, 2 Blocked, 3 Sleeping, 4 Zombie, 5 Dead |
| `privilege_level` | `u8` | |
| `cpu_ticks` | `u64` | no se incrementa para el PID 1 |
| `ram_bytes` | `u64` | `Process.ram_bytes_used` |
| `disk_read_bytes` | `u64` | ausente en la §4.2; se acumula en `vfs::record_disk_read` por cada lectura física del sistema montado |
