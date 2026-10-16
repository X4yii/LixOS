# Plan de implementaciones pendientes (LixOS)

> Creado a partir de la auditoría de deuda técnica y la validación efectuada.
> No incluye cambios en código; es la hoja de ruta operativa.

## Principios

- Sin commits hasta validar cada hito con `cargo test`, `smoke` y `test-nvme`.
- Todo cambio en drivers pasa primero por build `check` y tests unitarios.
- Cada fase produce un artefacto medible (métrica/bench o log de robustness).

---

## Fase 0 – Consolidación (AHORA)

- [x] NVMe lost-wakeup race corregida.
- [x] AHCI `write_sector` + `flush_cache`.
- [x] `format_ext4` Phase A (multi-grupo) + `mkdir`/`write_file`.
- [x] Disk selector real (NVMe/AHCI labels, read_only flag, leases).
- [x] TUI `lixinstall`.
- [x] Block layer skeleton: merge + queue + page cache placeholder + io_uring skeleton.
- [ ] `cargo clippy --workspace -D warnings` verde tras imports limpios.
- [ ] `cargo test --workspace` con 0 regresiones.
- [ ] Subir chunk loader a 256 KiB (`max_transfer_blocks = 512`).

## Fase 1 – Block + I/O core (prioridad alta, 2–4 semanas)

1. **Cablear merge real a I/O**
   - Codificar `BlockRequestQueue` en `NvmeDisk`/`AhciDisk` (mutex per-device).
   - `process_completions` → merged request → PRD list completa.
   - Unit tests con mock virtual NVMe.
   - Criterio: latencia por sector (bench) −20%.

2. **Page cache real**
   - Radix tree por inode + LRU (active/inactive).
   - Readahead 64–512 KiB adaptivo.
   - Writeback thread (bdi-like) con intervalos.
   - Criterio: test-NVMe `e2fsck`+`debugfs` tras instalación intacta.

3. **io_uring APIs**
   - Surface syscall `io_uring_setup/enter/sqe`.
   - Fixed buffers + submit queue ring por proceso.
   - Criterio: throughput ≥ 3x sobre syscall read/write (benchmark).

4. **Scheduler + CPU**
   - Per-CPU run queues + work stealing; O(1) priority bitmap.
   - IRQ rebalance e iopoll para NVMe.
   - Criterio: scheduling jitter < 20 us.

## Fase 2 – FS + VFS (3–6 semanas)

1. **VFS dentry/inode/super-block cache**
2. **EXT4 journaling** (commit marker, checksum)
3. **Snapshot/rollback** (A/B partitions)
4. **Network stack zero-copy** (slab pools)

## Fase 3 – Payload (resto)

5. **Big pages + swap/HugeTLB**
6. **Tracing lock-free ringbuf**
7. **Fast path async I/O via MSI-X**
8. **Observability Crash Dump** + OOPS handler.
9. **Fuzzing de drivers (NASM / C)**

---

## Entregables diarios (ideal)

- Cada día: código + 1 test nuevo + actualización detalle de bloqueo.
- Cada semana: ejecutar `cargo xtask smoke` y `test-nvme` y reportar.

## Riesgos y controles

- NVMe PRP mutación (risk AOC) → tests regresión + rollback git stash.
- PTE coherency → smoke cada merge.
- Ext4 formato → `e2fsck -fy` en host sparse file.

---
EOF
echo "plan escrito"