# Gestión de memoria

> **Tipo:** diseño · **Estado:** borrador · **ADR:** 19
> **Relacionado:** [Mapa de memoria virtual](../spec/memory-map.md) · Todos los SLO son los de ADR-14

Define la disciplina de asignación del kernel: RAII, asignaciones falibles, contabilidad estricta, orden de recuperación bajo presión y verificación de fugas.

Objetivo: **cero fugas, cero desperdicio evitable, y que cualquier falta de memoria sea un error manejable y nunca un cuelgue.**

## 1. Reglas de oro

1. **Todo recurso tiene un dueño y se libera al soltarlo (RAII).** Frames, mapeos, handles, buffers DMA, stacks y objetos del kernel son tipos con `Drop`. No existe un `free()` manual en el código del kernel.
2. **Prohibido `mem::forget` y `Box::leak`** fuera de la inicialización de arranque. Se aplica con `clippy::disallowed_methods` y falla el CI.
3. **Toda asignación del kernel es falible.** Una falta de memoria devuelve `Err(NoMem)` a quien la pidió. El kernel solo hace `panic` por falta de memoria durante el arranque. Se usan colecciones propias (`KVec`, `KBox`, `KMap`) con API `try_*`, y se prohíben `Vec::push`, `Box::new`, `String::from` y similares en `kernel/` mediante `disallowed_methods`.
4. **Sin ciclos de referencia.** Los objetos compartidos usan `Arc` hacia abajo (dueño → hijo) y `Weak` hacia arriba (hijo → padre).
5. **Nada se asigna con un spinlock tomado ni en un manejador de interrupción**, salvo desde reservas preasignadas.
6. **La memoria que sale del kernel hacia userspace siempre está a cero.** La que contuvo secretos (claves, hashes) se borra con `zeroize` antes de liberarse.

## 2. Capas

```text
+--------------------------------------------------------------+
|  Userspace: mmap anónimo, stacks, heap de sys-libc           |
+--------------------------------------------------------------+
|  Memoria virtual: VMAs, demanda, zero-page, page tables      |
+--------------------------------------------------------------+
|  Slab (objetos del kernel)   |   Page cache (ext4)           |
+--------------------------------------------------------------+
|  Asignador de páginas (buddy + cachés por CPU)               |
+--------------------------------------------------------------+
|  Frame database  |  Boot allocator (bitmap, solo al inicio)  |
+--------------------------------------------------------------+
```

### 2.1 Arranque
- Un asignador **bitmap** sencillo sirve solo hasta que el buddy está inicializado. Después se descarta.
- La memoria que el bootloader marca como recuperable (`BootloaderReclaimable`) se devuelve al buddy cuando ya no se usa `BootInfo`.
- Mapa de memoria validado: se rechazan regiones solapadas y se reservan 0-1 MB, el kernel, el initrd y los framebuffers.

### 2.2 Asignador de páginas (buddy)
- Órdenes 0 a 10 (4 KB a 4 MB). Dos zonas: `DMA32` (< 4 GB, para drivers de 32 bits) y `Normal`.
- **Frame database:** un descriptor de 16 bytes por frame (contador de referencias, flags, orden, dueño). Sobre 4 GB son 16 MB, el 0,4%, y se cuentan dentro del presupuesto del kernel.
- **Cachés por CPU** de páginas de orden 0 (magazines) para evitar contención cuando haya SMP. Se vacían bajo presión de memoria.
- **Watermarks** `min / low / high` por zona. Cruzar `low` despierta al hilo de reclamo. Cruzar `min` solo permite asignaciones críticas.
- **Fragmentación:** los frames no movibles (kernel, DMA) y los movibles (userspace, page cache) se agrupan por separado para poder formar bloques grandes.

### 2.3 Heap del kernel (slab)
- Un caché **por tipo de objeto** (`Process`, `Thread`, `Inode`, `Handle`, etc.) más cachés genéricos por clase de tamaño.
- Clases de tamaño geométricas con **desperdicio interno ≤ 12,5%** (16, 32, 48, 64, 80, 96, 112, 128, 160, 192...).
- Los slabs completamente libres se devuelven al buddy, con histéresis para no oscilar.
- Cada caché lleva contadores de asignados y liberados, visibles en `/dev/meminfo`.

### 2.4 Memoria virtual
- Cada proceso tiene un árbol de **VMAs** (`BTreeMap` por rango). Cada VMA lleva permisos, tipo (anónima, archivo, stack, framebuffer) y dueño.
- **Paginación por demanda con Contabilidad Estricta (*Strict Commit*):** `mmap` / `sys_vm_map` no reserva frames físicos de inmediato (se asignan en el primer `#PF`), pero **debita inmediatamente** de un contador global `committed_bytes`.
  - Invariante estricto: $\text{committed\_bytes} \le \text{RAM\_física} + \text{ZRAM\_capacidad}$.
  - Si una llamada a `sys_vm_map` excede el límite, falla de inmediato con `ENOMEM`. Se elimina cualquier overcommit implícito: cuando ocurre el `#PF`, el frame está matemáticamente respaldado.
- **Zero page compartida:** leer memoria anónima aún no tocada mapea una única página de ceros de solo lectura. Solo se asigna un frame real al escribir.
- **Sin `fork`, no hace falta copy-on-write general** (ADR-04). Se evita esa complejidad.
- **W^X estricto:** ninguna página es escribible y ejecutable a la vez. NX, **SMEP y SMAP** activados (SMAP además atrapa accesos accidentales del kernel a punteros de usuario fuera de `copy_*_user`).
- **Stacks de kernel** de 16 KB con una **página de guarda**. Los stacks de usuario crecen bajo demanda con su propia guarda.
- **Páginas grandes (2 MB)** solo para el mapeo directo del kernel. En usuario son opt-in, para no inflar el consumo de procesos pequeños.
- **Tablas de páginas:** se liberan las que quedan vacías al desmapear. Los TLB flushes se agrupan por lote.
- Direcciones con tipos distintos (`PhysAddr`, `VirtAddr`, `UserPtr<T>`) para que el compilador impida mezclarlas.

### 2.5 Page cache (ext4)
- Tamaño **acotado**: 25% de la RAM como máximo por defecto, y siempre desalojable.
- Las páginas limpias se descartan sin costo. Las sucias se escriben antes (writeback periódico, cada 5 s o al superar un umbral).
- Es el **primer** lugar donde se recupera memoria bajo presión.

## 3. Presión de memoria: orden de recuperación y ZRAM resiliente

```text
 libre > high ──► normal
 libre < low  ──► 1. descartar page cache limpia
                  2. vaciar cachés por CPU y slabs libres
                  3. escribir page cache sucia
                  4. comprimir páginas anónimas inactivas a ZRAM (ADR-09)
 libre < min  ──► 5. OOM killer (solo si todo lo anterior falla)
```

- **Inmunidad a Deadlocks en ZRAM:**
  - **Scratch Buffers Estáticos:** Cada CPU reserva en el arranque un buffer de compresión/descompresión LZ4 fijado en memoria de 64 KB. Cero asignaciones dinámicas durante el swap.
  - **Emergency Memory Reserve Pool:** El asignador Buddy mantiene una reserva de emergencia accesible únicamente bajo el flag `GFP_ZRAM_CRITICAL` cuando la memoria física cae por debajo de la marca `min`. Esto garantiza que ZRAM siempre pueda alojar sus metadatos de slab y liberar frames físicos sin bloquearse.
- **Sin overcommit** (ADR-09): `mmap` falla si el sistema no puede cumplir la reserva contable. Los fallos son predecibles y nunca ocurren como sorpresas en un `#PF`.
- **OOM killer** determinista. Elige el proceso con mayor consumo dentro del *job* que más excede su cuota, nunca servicios críticos (`init`, `authd`, el compositor). Registra la causa y las cifras en el log.
- Una falta de memoria en el kernel nunca debe llegar a `panic`, solo a `Err(NoMem)` o a una muerte limpia del proceso afectado.

## 4. Contabilidad y límites

- Cada proceso y cada *job* (ver `process-scheduler.md`) tienen una cuota de memoria. **La memoria del kernel que un proceso provoca se le cobra a él**: tablas de páginas, stacks de kernel, handles y objetos.
- Límites por defecto configurables: memoria total, hilos, handles abiertos. Impiden que un programa defectuoso agote el sistema.

## 5. Presupuesto para 4 GB de RAM (mínimo)

| Concepto | Presupuesto |
| --- | --- |
| Kernel base (código, datos, slabs) en reposo | ≤ 64 MB |
| Frame database | 16 MB (se cuenta en lo anterior) |
| Rootfs LiveISO (ADR-18) | ≤ 400 MB meta, 1 GB tope |
| Pool ZRAM | ≤ 25% de la RAM |
| Page cache | ≤ 25% de la RAM |
| Procesos de usuario | el resto |

## 6. Cómo se verifica que no hay fugas

La disciplina sin verificación no sirve. Estas comprobaciones son obligatorias en CI:

1. **Test de línea base:** al terminar cada test de integración en QEMU, el número de frames libres y los contadores de cada caché slab deben volver a la línea base inicial. Si no, el test falla.
2. **Test de ciclo:** crear y destruir un proceso 10.000 veces (con hilos, mapeos, handles y I/O en curso) debe dejar la memoria exactamente igual.
3. **Modo depuración del asignador:**
   - *Poisoning:* al liberar se rellena con `0xDE`. Al asignar, con `0xAA`.
   - *Redzones* y canarios alrededor de cada objeto slab.
   - *Cuarentena* de objetos liberados antes de reutilizarlos, para detectar use-after-free.
   - Detector de *double free*.
4. **Fuzzing de syscalls** con argumentos aleatorios, verificando la línea base al final.
5. **Estrés de OOM:** un test que consume toda la memoria debe terminar con el OOM killer, no con `panic`.
6. **Miri y `loom`** sobre las estructuras de datos puras (buddy, slab, VMAs) compiladas en el host.
7. **Soak en hardware real:** el uso de memoria del kernel en reposo no debe crecer en 24 horas. Se grafica `/dev/meminfo` periódicamente.

## 7. Observabilidad

`/dev/meminfo` (y `meminfo` en Vanish) muestra: frames libres por zona, tamaño de page cache, ZRAM (original, comprimido, ratio), por caché slab (objetos vivos, memoria total, desperdicio), por proceso (RSS, memoria de kernel cobrada) y eventos de OOM.

## 8. Decisiones abiertas

- Política exacta de qué páginas anónimas se consideran inactivas (CLOCK sobre el bit *accessed*). Se define junto a ZRAM.
- Si se añade KASLR.

---

## 9. Implementación actual

Comportamiento observado en `kernel/src/memory/`. Los apartados siguientes describen hechos del código; no modifican la especificación de las secciones anteriores.

### 9.1 Inicialización

`memory::init()` ejecuta `pmm::init()` → `vmm::init()` → `heap::init()` (`kernel/src/memory/mod.rs`). Los módulos `buddy` y `slab` se declaran en `mod.rs` pero no participan en la inicialización.

### 9.2 Asignador de páginas

`kernel/src/memory/pmm.rs` mantiene un `BitmapAllocator` dentro de un candado, con `alloc_frame`, `free_frame`, `alloc_contiguous_frames` y `free_contiguous_frames`. `PAGE_SIZE` es 4096. Este bitmap es el asignador en uso, no un provisional que se descarte después del arranque (§2.1).

### 9.3 Asignadores no utilizados

| Archivo | Contenido | Llamantes |
| --- | --- | --- |
| `kernel/src/memory/buddy.rs` (28 líneas) | `pub struct BuddyAllocator` con `alloc_order` / `free_order`; `MAX_ORDER = 11` (2048 marcos = 8 MiB) | ninguno |
| `kernel/src/memory/slab.rs` (106 líneas) | `SlabCache` y `SlabAllocator` con lista libre de `FreeNode` | ninguno |

Ningún fichero del árbol referencia `BuddyAllocator` ni `SlabAllocator`. La §2.2 (buddy con cachés por CPU y histéresis) y la §2.3 (slab devolviendo páginas al buddy) no tienen implementación correspondiente.

### 9.4 Heap del kernel

`kernel/src/memory/heap.rs`:

```rust
pub const HEAP_START: usize = 0xFFFF_9000_0000_0000;
pub const INITIAL_HEAP_SIZE: usize = 8 * 1024 * 1024;    // 8 MB
pub const MAX_HEAP_SIZE: usize = 256 * 1024 * 1024;      // 256 MB
```

El `#[global_allocator]` es un `linked_list_allocator::LockedHeap`, no una caché slab. Se reservan marcos bajo demanda hasta `MAX_HEAP_SIZE`.

### 9.5 Componentes sin implementación

| Apartado | Componentes citados | Estado en el árbol |
| --- | --- | --- |
| §3 | ZRAM, reserva de emergencia `GFP_ZRAM_CRITICAL`, recuperación de páginas anónimas | no hay código ZRAM |
| §4 | contabilidad estricta, cuotas por proceso, OOM killer | no hay contadores de uso por proceso |
| §5 | presupuesto para 4 GB | sin instrumentación que lo verifique |
| §6 | línea base de frames, redzones, canarios, Miri, `loom` | sin tests en `kernel/` |
| §7 | `/dev/meminfo`, `meminfo` en Vanish, eventos OOM | ni el nodo de dispositivo ni la salida existen; sí están `pmm::memory_stats() -> (total, libre)` y `vfs::disk_io_stats()`, consultables con `sys_monitor_metrics` (`0x80A`, ver [syscall-abi.md](../spec/syscall-abi.md) §10.12) |

La §7 asume además un montaje de `ext4`; `kernel/src/fs/ext4.rs` existe, pero no expone ninguna métrica por sí mismo: los contadores de E/S viven en `kernel/src/fs/vfs.rs` (`DISK_READ_OPS`, `DISK_READ_BYTES`) y se acumulan en `record_disk_read`.
