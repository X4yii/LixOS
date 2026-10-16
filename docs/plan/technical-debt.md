# Plan de recuperación de deuda técnica — LixOS

> Estado: working tree sin commit `origin/main == ba194a2`. Los cambios ya aplicados
> (remediación Fases 0–3, selección de disco, TUI del instalador, AHCI writable)
> quedan fuera del plan; esto planifica lo que **falta**.

## Objetivo
Llevar el sistema a un estado **usable como OS real**:
arranque reproducible, instalación funcional end-to-end, multitarea SMP real,
memoria con paginación y swap, FS journalado con cache, E/S NVMe estable.

Criterio de éxito global: `cargo xtask test-installer` verde *y* `boot_installed`
desde el disco instalado + `e2fsck` limpio + reinicio estable.

## Fases

### Fase 1 — Estabilidad de bloque y GPT (2 semanas)  [BLOQUEANTE]
1. **NVMe lost-wakeup** (`kernel/src/drivers/nvme.rs`)
   - Convertir `wait_for_completion` en espera atómica con IRQs off
     (`check_and_block` único).
   - Test de estrés: 1000 lecturas/escrituras con E/S concurrente (fuzz).
2. **`format_ext4` Fase A** (`libs/lix-storage/src/ext4.rs`)
   - Bitmap e inodos en *todos* los grupos.
   - Contadores reales de libre (`s_free_blocks_count`, `s_free_inodes_count`,
     `s_first_data_block` por tamaño SG):
   - Soporte de múltiples grupos, journaling mínimo (EXT4_MKFS_FEATURES).
   - Verificación: `e2fsck -fn` limpio sobre volumen generado desde el kernel.
3. **Extendir harness SATA de QEMU** (`xtask/src/smoke.rs`)
   - Añadir `-device ide-hd` + AHCI controller para probar la escritura AHCI
     end-to-end.
4. **Diseñar e implementar `wait_for_completion` tests** con timers.

**Puerta de salida**: `test-installer` completo (EXT4 + boot).

### Fase 2 — Paginación, COW y memoria (4 semanas)  [escalable]
1. Páginas on-demand: faults para `mmap`, guard pages, anonymous memory.
2. Copy-on-write para `fork`/`clone`; refcounts en PMM.
3. Swap a disco con lest/LRU clock; reclaim bajo presión.
4. Working set del instalador sin buffer 4 KiB por syscall:
   - `vecs` grandes por vía `sys_read_file_all` (ya acotado a 64 MiB).
   - Extensión de ABI para `readv/writev` y `mmap` (read-only por ahora).
5. Test de presión: 4 procesos llenando `RAM`, swap + OOM sin panic.

### Fase 3 — Almacenamiento real (6 semanas)
- Drivers no polling: interrupciones + eventos completos.
- Colas por dispositivo, NVMe namespaces y NCQ en AHCI.
- Bloque layer `bio` con vectores, scheduler por CPU (none/mq-deadline).
- Prefetch y `fallocate` para offsets extendidos.
- Journaling EXT4 real: metadata + ext-data incl., checksums, recovery.
- Page cache + writeback: `dirty` buffers, `dirty_writeback_interval`.
- Soporte `fsync`/`fdatasync`.

### Fase 4 — Seguridad y observabilidad (3 semanas)
- KASLR/ASLR, NX, W^X, canaries de stack, syscopy safeguards.
- `seccomp`-like según clase de syscall.
- Capabilities/violências con `NS` y cgroups.
- Dmesg + ring buffer con niveles + ftrace simple.
- Watchdog (soft/hard lockup), OOPS handler, crash-dump.

### Fase 5 — SMP real y scheduling (4 semanas)
- Run queues por CPU, work stealing, afinidad+bandwidth.
- Locks per-CPU en lugar de globales (`DISK_IO_LOCK` → por dispositivo).
- Preempción con preemption points.
- `procfs` y `sysfs` para diagnóstico.

### Fase 6 — Drivers/ABI y portabilidad (8 semanas)
- Hot-plug PCI; DMI/ACPI, suspend/resume.
- GPU/DRM KMS + cursor HW; input by IRQ.
- Networking zerocopy + QoS + virtio-net.
- RISC-V/ARM64 + UEFI variants.
- portables toolchain (`target/riscv64gc-unknown-none-elf`).

## Gobernanza / prácticas
- **Fuzzing syscalls**: cargo-fuzz con cargo-lift-to-JSON fixed point.
- **Conventional commits**: cada fase cerrada se mergea tras tests-stress.
- **Docs**: actualizar `docs/design/memory-management.md`, `capabilities.md` por fase.
- **Test matrix**: QEMU i386/x86_64 + ARM64 + ARM; real HW.
- **Release**: solo tras boot 1000x consecutivos + `e2fsck` limpio + stress de carga.

## Dependencias y orden
```
Fase 1 (NVMe + ext4 + harness)
  └── Fase 2 (memoria)
        └── Fase 3 (FS real + cache)
              └── Fase 4 (seguridad)
                    └── Fase 5 (SMP)
                          └── Fase 6 (portabilidad)
```
Nunca merges sin tests en verde; cualquier regression aborta el merge.

## Riesgos identificados
- **Race conditions en multi-core**: el race NVMe actual indica que las primitivas `Atomic`/IRQ-of no sellan. Prioridad.
- **Formatext4 con grupos**: los cambios tocan estructuras on-disk; requieren `make test + debugfs + e2fsck` por commit.
- **Costo TCG**: el test kernel/install tarda ~20 min; planear días de CI nightly.
- **Hispano/regionalización**: el código es en español; mantener nomenclatura consistente.

## Criterios de "done" por fase
- Phase 1: 10 arranques consecutivos, instalación en 10 GiB EXT4, boot desde disco.
- Phase 2: 4 procesos × 2 GiB sin panics + `kswapd` activo.
- Phase 3: `fsck` limpio en suspensión, `writeback` real, journal recovery.
- Phase 4: fuzzer 10k casos / 0 panics, ASAN + UBSAN configs.
- Phase 5: scheduler stable bajo 100 hilos en 4 vCPU.
- Phase 6: boot ARM/RISC-V y Doom demo.