# Estrategia de pruebas y CI

> **Tipo:** guía · **Estado:** borrador · **ADR:** 19, 21, 22, 14
> **Relacionado:** [Gestión de memoria](../design/memory-management.md) · [Instalador](../design/installer.md)

Define las categorías de prueba, los controles de CI y los umbrales de regresión.

Las garantías de Rust capturan en tiempo de compilación la mayoría de errores de memoria y *data races*. Los errores lógicos, las fugas de memoria y los *deadlocks* se verifican mediante CI y pruebas en hardware emulado y real.

## 1. Pruebas Unitarias y de Integración (`cargo test`)
Todo código independiente del hardware (`#![no_std]`, utilidades, parsers como PML o ext4) debe contar con tests exhaustivos.
- Se ejecutan en el host (Linux/Windows) nativamente durante el CI.
- **Miri:** Validación de todo el código `unsafe` en las bibliotecas de abstracción y `sys-libc` utilizando `cargo miri test` para detectar comportamientos indefinidos (UB).

## 2. Pruebas de Sistema (QEMU Integration)
Scripts que compilan el kernel, crean la imagen ISO y la ejecutan en QEMU con el parámetro `-isa-debug-exit`.
- **Línea Base de Memoria (Leak Detection):** El entorno registra los frames y *slabs* exactos después de arrancar. El test crea y destruye recursos (p.ej., 10,000 procesos o lecturas de FS), y verifica que al finalizar, la memoria vuelva **exactamente a la línea base**. (Decisión ADR-19).
- **Pruebas de Caída (Crash Resilience):** Se inducen fallos (ej. pánico en el compositor `lix-compositor`) y se verifica si `init` lo reinicia adecuadamente o si el sistema retrocede correctamente a TTY.

## 3. Pruebas de Transacciones y Corte de Energía
Crítico para `ext4` y `lix-installer` (ADR-21).
- El CI ejecuta escrituras y transferencias en un script que "mata" el proceso QEMU en intervalos aleatorios.
- Luego, arranca nuevamente y ejecuta `fsck.ext4` para validar la integridad del _journaling_.

## 4. Pruebas de Compatibilidad POSIX (`compat/`)
Directorio de programas reales en C (ej. `lua`, `sqlite`, `busybox`).
- En cada commit, el CI recompila estos programas vinculados contra la nueva `sys-libc`.
- Se genera un reporte automático mostrando si compilan y cuántas líneas de parche se requieren. Si la mediana de parches supera el límite (ADR-22), la build falla.

## 5. Fuzzing
Todos los parsers que interactúan con el exterior o en Ring 0:
- Parser del FS (ext4).
- Parseador de ELF64 del kernel.
- Entrada IPC (`sys_channel_read`) del compositor.
Se realiza mediante `cargo-fuzz`.

## 6. Pruebas de Regresión de Rendimiento (Benchmarking)
- El CI mide tiempos de `SYSCALL` (ej. `getpid` < 300ns), tiempo de booteo a prompt (< 1s en QEMU) y rendimiento del allocador.
- Cualquier commit que ralentice estas métricas más del 5% sin justificación es rechazado.

---

## 7. Implementación actual

Comportamiento observado en `xtask/`, `Cargo.toml`, `.github/workflows/ci.yml` y el árbol de fuentes. Los apartados siguientes describen hechos del código; no modifican la especificación de las secciones anteriores.

### 7.1 Tareas disponibles

`xtask` acepta las tareas de `xtask/src/main.rs`:

| Tarea | Acción |
| --- | --- |
| `build` | `cargo build` de tres targets (kernel, espacio de usuario, xtask) |
| `check` | `cargo check --workspace` |
| `codegen` | Regenera los tres artefactos del ABI desde `abi/syscalls.toml` |
| `iso` | `build` + `initramfs.tar` + `xorriso` + `limine bios-install` |
| `run` | `iso` + creación del disco + lanzamiento de QEMU |
| `deps`, `asm`, `test-nvme`, `bench`, `nvme-image`, `nvme-image-4k` | dependencias, ensamblado, pruebas de arranque, microbenchmarks e imágenes |

No hay tareas `test`, `fuzz` ni `miri` en `xtask`: las pruebas se ejecutan con `cargo test --workspace`.

### 7.2 Dónde viven las pruebas hoy

`cargo test --workspace` ejecuta los `#[test]` de los crates del workspace que no desactivan las pruebas. El recuento total y el estado real lo fija el job `test` del CI: no se documenta un número fijo aquí para que este documento no envejezca.

| Módulo | Archivos con pruebas |
| --- | --- |
| `kernel/` | `src/memory/address.rs`, `src/task/handle.rs`, `src/drivers/nvme.rs` — **no se ejecutan**: `kernel/Cargo.toml` declara `test = false` en el `[[bin]]` |
| `libs/lix-net/` | `lib.rs`, `routing.rs`, `tcp.rs`, `tcp_state.rs` (la pila de protocolos es host-testable, ADR-26) |
| `libs/lix-storage/` | `lib.rs` (validación GPT) |
| `libs/lix-wifi/` | `crypto.rs`, `htc.rs` |
| `libs/lix-usb/`, `libs/lix-sys/` | `lib.rs` |
| `tools/lix-installer/` | `plan.rs`, `fat32.rs` |
| `userland/*` | sin pruebas: `vanish`, `edit`, `ltop` y `lixinstall` declaran `test = false` |
| `services/*`, `xtask/`, `pkg/pml/`, `libs/lix-edit` | sin `#[test]` |

Los módulos recién separados siguen la misma regla: `kernel/src/task/scheduler/{runqueue,spawn,wait,metrics}.rs`, `kernel/src/task/syscall/{block,io,net,proc,term}.rs` y `kernel/src/drivers/framebuffer/{…}.rs` no tienen pruebas propias; las de la lógica subyacente viven en `libs/`. `userland/lix-shell` y `libs/lix-ui` fueron retirados del árbol, así que sus filas desaparecen de cualquier inventario de pruebas.

### 7.3 Pipeline de CI (`.github/workflows/ci.yml`)

Seis jobs, todos requeridos, en `ubuntu-latest`, con `actions/checkout@v4` (`submodules: recursive`, porque `libs/litui` son miembros del workspace) y `dtolnay/rust-toolchain@stable` (`components: rust-src, rustfmt, clippy`, target `x86_64-unknown-none`):

| Job | Comando | Notas |
| --- | --- | --- |
| `fmt` | `cargo fmt --package <paquetes propios> -- --check` | falla y lista los archivos con diferencias. No se usa `--all` porque `libs/litui` es un submódulo con repositorio y licencia propios: LixOS no puede imponerle formato (el commit del padre sólo arrastraría el puntero, no el cambio) |
| `clippy` | `cargo clippy --workspace --all-targets -- -D warnings` | con `RUSTC_BOOTSTRAP=1` (entorno del workflow) |
| `codegen-drift` | `cargo xtask codegen` + `git diff --exit-code` | **es el control que impide que el ABI se desincronice** (ADR-24). `codegen` además pasa `rustfmt` por los ficheros que genera, para que este job y el de `fmt` no se contradigan |
| `test` | `cargo test --workspace --locked` | incluye los tests unitarios del **kernel**, que se ejecutan en el host (§1.3) |
| `deps` | `cargo xtask deps` | paquetes base de Ubuntu (qemu, xorriso, mtools, ovmf…) **y Limine**, que Ubuntu no empaqueta: lo baja de su release binario fijado y compila el CLI con el `cc` del runner |
| `smoke` | `cargo xtask smoke` | arranca la ISO en QEMU y le habla por el puerto serie: `help`, `pwd`/`cat`, `ls`, `hello` (ELF de C con `int 0x80`), `ps` (derecho `INSPECT`); además busca marcadores de pánico/`EIP`/`BUG` en toda la salida |

`RUSTC_BOOTSTRAP=1` está en el `env` global porque así se compila todo el árbol (ver `xtask/src/main.rs`).

Estado real de los controles de las §1–§6:

| Elemento de las §1–§6 | Estado |
| --- | --- |
| Integración continua | `.github/workflows/ci.yml` con los 7 jobs de la §7.3 |
| `cargo test` sobre el workspace propio | presente en `libs/lix-net`, `libs/lix-storage`, `libs/lix-wifi`, `libs/lix-usb`, `libs/lix-sys`, `tools/lix-installer` y en el **binario del kernel** (8 tests: `memory::address`, `task::handle`, `drivers::nvme`), que corren en el host; **ausente** en `services/`, `userland/`, `xtask/`, `pkg/` y `libs/lix-edit` |
| Miri | sin configuración ni invocación |
| `-isa-debug-exit` en QEMU | los argumentos de `xtask run` no lo incluyen |
| `compat/` (§4) | no existe el directorio |
| `cargo-fuzz` (§5) | no existe `fuzz/` ni dependencia asociada |
| Benchmarks (§6) | `criterion = "0.8"` está declarado en `[workspace.dependencies]` de `Cargo.toml` y no lo usa ningún crate; no hay `[[bench]]`. Existe `cargo xtask bench`, una suite propia con RDTSC, sin umbral de regresión en CI |
| Detección de fugas (§2) | no hay línea base de frames ni contadores de slab (ver [memory-management.md](../design/memory-management.md) §9) |
| Latencia de `SYSCALL` (§6) | sin medición en CI (ADR-14) |

Fuera del workspace propio, `libs/litui/` (submódulo de terceros) trae sus propias pruebas.
