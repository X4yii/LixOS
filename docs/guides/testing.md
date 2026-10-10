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

Comportamiento observado en `xtask/src/main.rs`, `Cargo.toml` y el árbol de fuentes. Los apartados siguientes describen hechos del código; no modifican la especificación de las secciones anteriores.

### 7.1 Tareas disponibles

`xtask` acepta cuatro argumentos (`xtask/src/main.rs`):

| Tarea | Acción |
| --- | --- |
| `build` | `cargo build` de tres targets (kernel, espacio de usuario, xtask) |
| `check` | `cargo check --workspace` |
| `iso` | `build` + `initramfs.tar` + `xorriso` + `limine bios-install` |
| `run` | `iso` + creación del disco + lanzamiento de QEMU |

No hay tarea `test`, `bench`, `fuzz` ni `miri`.

### 7.2 Ausencia de infraestructura

| Elemento de las §1–§6 | Estado |
| --- | --- |
| Integración continua | no hay `.github/` ni ningún otro directorio de CI |
| `cargo test` sobre el workspace propio | sin `#[test]` ni `#[cfg(test)]` en `kernel/`, `libs/{lix-sys,sys-libc,lix-ui}`, `userland/`, `services/`, `tools/`, `pkg/` o `xtask/` |
| Miri | sin configuración ni invocación |
| `-isa-debug-exit` en QEMU | los argumentos de `xtask run` no lo incluyen |
| `compat/` (§4) | no existe el directorio |
| `cargo-fuzz` (§5) | no existe `fuzz/` ni dependencia asociada |
| Benchmarks (§6) | `criterion = "0.8"` está declarado en `[workspace.dependencies]` de `Cargo.toml` y no lo usa ningún crate; no hay `[[bench]]` |
| Detección de fugas (§2) | no hay línea base de frames ni contadores de slab (ver [memory-management.md](../design/memory-management.md) §9) |

Los únicos tests del repositorio están en `libs/litui/`, que es un submódulo de terceros.
