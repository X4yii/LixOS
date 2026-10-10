# Portabilidad de software

> **Tipo:** especificación · **Estado:** borrador · **ADR:** 22
> **Complementa:** ADR-04, 05, 06 · **Relacionado:** [Guía de portabilidad](../guides/porting-guide.md)

Define la estrategia para que proyectos escritos para otros sistemas operativos compilen para `x86_64-lix`, los niveles de compatibilidad T0–T3 y cómo se mide el éxito.

**Meta:** con LixOS estable (`1.0`), un proyecto escrito para Linux/BSD/macOS debe compilar para `x86_64-lix` con **cero o muy pocos cambios**, y esos cambios deben vivir en un árbol de parches mantenido, no en el proyecto original.

> [!IMPORTANT]
> Esta meta entra en conflicto con decisiones anteriores (sin `fork`, sin señales, sin `epoll`, sin linker dinámico, sin autoridad ambiente); §3 revisa cada una de ellas. La compatibilidad **no se consigue copiando el kernel de Linux**, sino haciendo que la **capa C/POSIX** (`sys-libc`) esconda las diferencias.

## 1. Principio rector

```text
 Código fuente existente (C, C++, Rust, ...)
            │  compila contra
            ▼
 ┌─────────────────────────────┐
 │  sys-libc  (POSIX + extras) │  ◄── aquí vive la compatibilidad
 └──────────────┬──────────────┘
                │ usa
 ┌──────────────▼──────────────┐
 │  lix-sys  (API nativa)      │  ◄── ABI propia, capabilities
 └──────────────┬──────────────┘
                │ SYSCALL
 ┌──────────────▼──────────────┐
 │  Kernel Lix                 │
 └─────────────────────────────┘
```

El kernel mantiene su diseño (capabilities, Jobs, `spawn`). La libc **traduce** el mundo POSIX a ese diseño. Si una función POSIX no se puede traducir bien, se añade una **primitiva mínima** al kernel solo para eso, nunca se deforma el modelo entero.

## 2. Lo que se adopta de Linux x86_64 (coste cero, máximo efecto)

Se copian **convenciones**, no código:

| Aspecto | Decisión |
| --- | --- |
| Modelo de datos | **LP64**: `int`=32, `long`=64, puntero=64, `char` con signo, `wchar_t`=32 |
| Calling convention | System V AMD64 |
| `errno` | mismos números que Linux |
| Constantes | `O_*`, `SEEK_*`, `S_IF*`, `SIG*`, `AF_*`, `EPOLL*` iguales a Linux |
| Estructuras | `struct stat`, `timespec`, `dirent`, `sockaddr`, `termios`, `pollfd` con el mismo layout |
| Rutas | Layout tipo FHS: `/bin`, `/lib`, `/etc`, `/tmp`, `/var`, `/dev`, `/home`, además de `/usr` como enlace a `/` (muchos proyectos escriben `/usr/bin/env`, `/usr/lib`) |
| Archivos de sistema | `/etc/passwd`, `/etc/group`, `/etc/hosts`, `/etc/resolv.conf`, `/etc/os-release`, `/etc/localtime` (versiones **públicas**, sin hashes; los secretos siguen en `/etc/passwords`, ADR-08) |
| Macros predefinidas | `__lix__`, `__unix__`, `_POSIX_VERSION`. **No** se define `__linux__` (mentiría y activaría código específico de Linux que no existe) |
| Triple | `x86_64-unknown-lix` en LLVM/clang y `x86_64-lix` en GNU config |

## 3. Revisión de decisiones previas

| Tema | Antes | Ahora | Por qué |
| --- | --- | --- | --- |
| **`fork`** | No existe (ADR-04) | **`vfork` y `posix_spawn`.** `fork` real con copy-on-write: **fuera de T2**, se evalúa según `compat/` | `vfork` (el hijo comparte el espacio de direcciones hasta `exec`) es barato, estable y cubre ~90% de los usos (`fork`+`exec`). El `fork` completo exige COW en el gestor de memoria y complica la contabilidad. |
| **Señales** | Fuera de v1.0 | **Subconjunto (nivel T2):** `SIGINT`, `SIGTERM`, `SIGKILL`, `SIGSEGV`/`SIGBUS`/`SIGFPE`, `SIGPIPE`, `SIGCHLD`, `SIGALRM`, `SIGUSR1/2`. `sigaction`, `sigprocmask`, `kill`, `raise`, `alarm` | Casi todo programa instala al menos un handler. Sin esto no enlaza. El kernel entrega una **notificación asíncrona** a un *trampoline* de la libc. |
| **`poll`/`select`/`epoll`** | Fuera de v1.0 | **`poll` y `select` en nivel T1; `epoll` emulado en libc en T2** | Todo se apoya en una primitiva única del kernel, `wait_many(handles)`. Coste bajo, valor alto. |
| **Autoridad ambiente** (ADR-05) | Sin rutas globales | Cada proceso hereda un **handle de directorio raíz** (su *namespace*). `open("/ruta")` de libc = `openat(raiz, "ruta")` | Así `open("/etc/hosts")` funciona como siempre, y un proceso **confinado** recibe una raíz restringida. El modelo de seguridad se conserva. |
| **Enlace dinámico / `dlopen`** | Fuera (D-D, 1.x) | **El loader soporta `ET_DYN` y PIE desde el inicio** (para ASLR). `dlopen` devuelve error limpio. Linker dinámico `ld-lix` en `1.x` | Preparar PIE ahora es casi gratis. Python, plugins y muchas librerías necesitan `dlopen`, pero el 90% de proyectos CLI compila estático. |
| **Sockets** | Sin red en v1.0 | **Cabeceras y `AF_UNIX` locales en nivel T2.** `AF_INET/INET6` devuelven `EAFNOSUPPORT` hasta tener red | Muchos proyectos ni compilan sin `<sys/socket.h>`. `AF_UNIX` además da la IPC estándar. |
| **`mmap` de archivos** | No definido | **`mmap` de archivo (`MAP_PRIVATE`, `MAP_SHARED`) sobre el page cache** | SQLite, linkers, compresores y bases de datos lo usan. |
| **Terminal** | TTY básica | **`termios`, `ioctl` de terminal y PTY (`/dev/ptmx`) en nivel T2** | vim, less, ssh, editores y shells los necesitan. |
| **`/bin/sh`** | Solo Vanish | Se incluye una **shell POSIX `sh`** (portada, o escrita en Rust) además de Vanish | `system()`, `popen`, `Makefile` y `configure` la usan. Vanish no es POSIX `sh` y no tiene por qué serlo. |

## 4. Rust, C++ y otros lenguajes

| Lenguaje | Plan |
| --- | --- |
| **C / C++** | `x86_64-lix-clang` (parche de LLVM para registrar el SO) y GCC después. libc++ sobre `sys-libc`. |
| **Rust** | Target `x86_64-unknown-lix`. Primero `no_std` + `alloc`. Después un **port de `std`** (`library/std/src/sys/lix`) apoyado en `lix-sys`, y soporte `target_os = "lix"` en el crate `libc`. Se mantiene fuera de árbol; se intenta subir upstream como *tier 3*. |
| **Go, Python, etc.** | Después de 1.0. Requieren `dlopen`, `fork` y sockets. |

## 5. Sistemas de compilación

| Herramienta | Qué hace falta |
| --- | --- |
| **autotools** | Reconocer `x86_64-lix` en `config.sub`/`config.guess` (parche enviado a GNU `config-patches`). Hasta entonces, se distribuye una copia. |
| **CMake** | `Lix-toolchain.cmake` y `Platform/Lix.cmake` |
| **Meson** | `lix-cross.txt` |
| **pkg-config** | Soporte nativo con rutas de `sysroot` |
| **Cargo** | Target personalizado en `.cargo/config.toml` |

El flujo realista es **compilación cruzada desde Linux** con un `sysroot` de Lix. Compilar **dentro** de Lix (autoalojamiento) es un objetivo **posterior a 1.0**, y exigiría `sh`, `make`, coreutils y un compilador portados.

## 6. Niveles de compatibilidad (lo que se promete)

| Nivel | Qué cubre | Ejemplos objetivo |
| --- | --- | --- |
| **T0** | C estándar puro | `lua`, `zlib`, `lz4`, `zstd`, `sqlite` (sin mmap) |
| **T1** | POSIX de línea de comandos: archivos, `spawn`/`vfork`, `poll`, `termios` | `make`, `coreutils`/`busybox`, `less`, `bzip2`, `xz` |
| **T2** | Hilos, señales, `mmap` de archivo, PTY, `AF_UNIX` | `git` (local), `vim`, `ripgrep`, `fd`, `tmux`, `sqlite` completo |
| **T3** | `fork` real, `dlopen`, red | `curl`, `python` con módulos, `nginx` |

El nivel que **sí se garantiza al llegar a `1.0` es T2**.

## 7. Cómo se mide el éxito

1. Directorio `compat/` y árbol de ports `ports/<nombre>/` con: receta de compilación (`build.toml`), parches y resultado de tests.
2. **Métrica por port:** número de líneas parcheadas. **Objetivos para `1.0`:** T0 y T1 con **0 parches**, T2 con mediana **≤ 20 líneas**.
3. Un informe automático en CI, `compat-report.md`: *N de M proyectos compilan sin parches / con parches / fallan*. Una regresión rompe el CI.
4. **Regla:** si un mismo parche aparece en 3 ports, se convierte en una **corrección de `sys-libc` o del `sysroot`**, no se repite.
5. Los parches pequeños y genéricos se envían a los proyectos originales (upstream).

## 8. Exclusiones

- **Capa de compatibilidad de binarios Linux** (ejecutar ELF de Linux tal cual, como WSL1 o el *linuxulator* de FreeBSD). Duplicaría el trabajo del kernel, contradice el ABI propio y rompe el modelo de capabilities. La promesa es **compatibilidad de código fuente**, no de binarios.
- Emular `/proc` y `/sys` completos. Solo un subconjunto mínimo de `/proc/self` si un port importante lo exige.
- Definir `__linux__`.

## 9. Congelar el ABI en 1.0

La estabilidad es lo que permite que los ports **no se rompan**. Por eso, al llegar a `1.0.0` (ADR-17):
- Se congelan los números y semántica de syscalls (y los tipos de `lix-sys`).
- Se congelan el layout de las estructuras expuestas, las constantes y los nombres de la libc.
- Se versiona el `sysroot` (`sysroot-1.0`), y las nuevas funciones se **añaden**, sin cambiar las existentes.
- Los cambios incompatibles solo ocurren en `2.0`.

## 10. Primitivas requeridas por la portabilidad

La tabla siguiente recoge las piezas del kernel y de `sys-libc` de las que dependen los niveles de compatibilidad. Es una lista de requisitos técnicos, sin orden de implementación.

| Nivel | Piezas requeridas |
| --- | --- |
| T0–T1 | Loader con soporte `ET_DYN`/PIE. Primitiva `wait_many`. Handle de raíz por proceso. `vfork`/`posix_spawn`, `poll`/`select`, `termios`, `devfs` (`null`, `zero`, `random`, `urandom`, `tty`, `ptmx`), archivos `/etc/*` de compatibilidad. |
| T1–T2 | Cabeceras de sockets, `AF_UNIX`, señales básicas, toolchain `x86_64-lix`, shell `sh` POSIX, toolchains CMake/Meson, informe `compat/`. `mmap` de archivo sobre el page cache, `epoll` emulado, PTY completa. |
| T2 | Verificación del nivel T2 y congelación del ABI (§9). Evaluación de `fork` real con copy-on-write. |
| T3 | `ld-lix`/`dlopen`, red, autoalojamiento. |

---

## 11. Implementación actual

Comportamiento observado en `libs/sys-libc/`, `libs/lix-sys/`, `kernel/src/task/elf.rs` y `xtask/src/main.rs`. Los apartados siguientes describen hechos del código; no modifican la especificación de las secciones anteriores.

### 11.1 Niveles

Ninguno de los niveles T0–T3 está implementado. `sys-libc` (`libs/sys-libc/src/lib.rs`) expone dos módulos, `syscalls` e `io`, y 18 envoltorios `sys_*` sobre la instrucción `SYSCALL`. No hay `open`, `read`, `write`, `fork`, `exec*`, `poll`, `select`, `epoll`, `pthread_*`, `malloc` ni `stdio`.

### 11.2 Convenciones de la §2

| Elemento de la §2 | Estado |
| --- | --- |
| Macros `__lix__`, `__unix__`, `_POSIX_VERSION` | no se definen en ningún `RUSTFLAGS` (`xtask/src/main.rs`) |
| `errno` | `sys-libc` no declara `errno`; los errores vuelven como `Result<_, i32>` o entero negativo |
| Constantes y *layouts* de Linux | `lix_sys::ErrorCode` (23 valores) y `lix_sys::TerminalInfo`; no hay `struct stat`, `termios` ni `sockaddr` |
| Estructura de directorios (FHS), `/etc/*` | no hay montaje de `devfs` ni nodos `tty`/`ptmx`/`null` |
| `sysroot` y cabeceras `<lix/syscall.h>` | no existe ningún fichero `.h` en el árbol |

### 11.3 Carga de binarios

`kernel/src/task/elf.rs` exige `Type::Executable` (`ET_EXEC`) y devuelve «ELF no es un ejecutable» para `ET_DYN`. Los segmentos se mapean en su `virtual_addr` declarada, sin reubicación ni ASLR.

En consecuencia, la fila «Enlace dinámico / `dlopen`» de la §2 («El loader soporta `ET_DYN` y PIE desde el inicio») no corresponde al árbol actual, aunque la §10 sí lo lista como requisito del nivel T0–T1. El detalle está en [memory-map.md](memory-map.md) §5.6.

### 11.4 Medición

No existe el directorio `compat/` (§7), ni `ports/`, ni el toolchain `x86_64-lix`, ni integración continua. `xtask` ofrece `build`, `check`, `iso` y `run`, sin tarea de pruebas (ver [testing.md](../guides/testing.md) §7).
