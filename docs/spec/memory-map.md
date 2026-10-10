# Mapa de memoria virtual (x86_64)

> **Tipo:** especificación · **Estado:** borrador · **ADR:** 09, 19
> **Arquitectura:** x86_64 con paginación de 4 niveles (PML4, 48 bits canónicos)
> **Relacionado:** [Gestión de memoria](../design/memory-management.md)

Define el reparto del espacio de direcciones de 48 bits entre el kernel (higher-half) y el espacio de usuario (lower-half), y las reglas de paginación y seguridad.

---

## 1. Principio Rector

La arquitectura x86_64 divide el espacio direccionable de 64 bits en dos mitades canónicas de 128 TB cada una, separadas por un agujero no canónico no direccionable:
- **Espacio de Usuario (Lower Half):** Bits 47 a 63 en `0`. Rango: `0x0000_0000_0000_0000` – `0x0000_7FFF_FFFF_FFFF`.
- **Agujero No Canónico:** `0x0000_8000_0000_0000` – `0xFFFF_7FFF_FFFF_FFFF` (genera `#GP` al acceder).
- **Espacio de Kernel (Higher Half):** Bits 47 a 63 en `1`. Rango: `0xFFFF_8000_0000_0000` – `0xFFFF_FFFF_FFFF_FFFF`.

```text
0x0000_0000_0000_0000 ┌──────────────────────────────────────┐
                       │  Espacio de Usuario (128 TB)         │
                       │  - Código ELF (PIE)                  │
                       │  - Heap de sys-libc                  │
                       │  - VMAs mapeadas (mmap)              │
                       │  - Stacks de usuario con guard pages │
0x0000_7FFF_FFFF_FFFF └──────────────────────────────────────┘
                       [ Agujero No Canónico (genera #GP)     ]
0xFFFF_8000_0000_0000 ┌──────────────────────────────────────┐
                       │  Higher-Half Direct Map (HHDM, 64 TB)│ ◄── Toda la RAM física 1:1
0xFFFF_C000_0000_0000 ├──────────────────────────────────────┤
                       │  Heap del Kernel (Slab, 16 TB)       │
0xFFFF_D000_0000_0000 ├──────────────────────────────────────┤
                       │  MMIO / Dispositivos / VMOs (16 TB)  │ ◄── GOP, AHCI, NVMe, PCI
0xFFFF_E000_0000_0000 ├──────────────────────────────────────┤
                       │  Stacks del Kernel (16 TB)           │ ◄── 16 KB + Guard Page
0xFFFF_FFFF_8000_0000 ├──────────────────────────────────────┤
                       │  Kernel Code & Data (-2 GB)          │ ◄── .text, .rodata, .data, .bss
0xFFFF_FFFF_FFF0_0000 ├──────────────────────────────────────┤
                       │  APIC / Local APIC / Core IO (-1 MB) │
0xFFFF_FFFF_FFFF_FFFF └──────────────────────────────────────┘
```

---

## 2. Desglose del Espacio del Kernel (Higher Half)

### 2.1 Higher-Half Direct Map (HHDM)
- **Rango:** `0xFFFF_8000_0000_0000` – `0xFFFF_BFFF_FFFF_FFFF` (64 TB).
- **Función:** Mapeo 1:1 de toda la memoria física accesible. Cualquier dirección física `PhysAddr(p)` se traduce inmediatamente en el kernel a `VirtAddr(0xFFFF_8000_0000_0000 + p)`.
- **Implementación:** Limine provee este mapeo en el arranque mediante el flag `LIMINE_HHDM_REQUEST`. El kernel reutiliza estas tablas durante la fase de arranque.
- **Páginas:** Mapeado con páginas gigantes de 1 GB y 2 MB para minimizar entradas de TLB.

### 2.2 Heap del Kernel (Slab Allocator)
- **Rango:** `0xFFFF_C000_0000_0000` – `0xFFFF_CFFF_FFFF_FFFF` (16 TB).
- **Función:** Almacena objetos del kernel de tamaño fijo (`Process`, `Thread`, `Inode`, `HandleTable`) y colecciones dinámicas falibles (`KVec`, `KBox`).
- **Páginas:** Páginas de 4 KB gestionadas por el Buddy Allocator bajo demanda.

### 2.3 Región de MMIO y Mapeos de Dispositivos
- **Rango:** `0xFFFF_D000_0000_0000` – `0xFFFF_DFFF_FFFF_FFFF` (16 TB).
- **Función:** Mapeo de registros de hardware en memoria:
  - Registros BAR de controladores AHCI y NVMe.
  - Framebuffer físico de arranque (GOP) configurado con *Write-Combining* (PAT).
  - VMOs de buffers gráficos administrados por `lix-drm`.
- **Atributos de Paginación:** Flags `PCD` (Page Cache Disable) y `PWT` (Page Write Through) activos según el tipo de dispositivo.

### 2.4 Stacks del Kernel
- **Rango:** `0xFFFF_E000_0000_0000` – `0xFFFF_EFFF_FFFF_FFFF` (16 TB).
- **Estructura por Hilo:**
  - Stack size: 16 KB (4 páginas contiguas).
  - **Guard Page:** 1 página de 4 KB no mapeada (sin bit *Present*) en la base de cada stack. Un desbordamiento del stack del kernel detona un `#PF` inmediato e impide corromper estructuras adyacentes.

### 2.5 Binario del Kernel (Higher-Half `-2 GB`)
- **Rango:** `0xFFFF_FFFF_8000_0000` – `0xFFFF_FFFF_FFFF_FFFF` (últimos 2 GB del espacio virtual).
- **Función:** Aloja las secciones ELF del kernel cargadas por Limine:
  - `.text`: Código de Ring 0 (Sólo Lectura + Ejecutable, $R-X$).
  - `.rodata`: Constantes y tablas de syscalls (Sólo Lectura, $R--$).
  - `.data` / `.bss`: Variables globales mutables del kernel (Lectura/Escritura, $RW-$).
- **Modelo de Código:** Requiere compilar con `-C code-model=kernel` o `-mcmodel=kernel` en LLVM/Rust para que el compilador use direccionamiento relativo de 32 bits con signo.

---

## 3. Desglose del Espacio de Usuario (Lower Half)

Cada proceso posee su propio espacio de direcciones en la mitad inferior, respaldado por su propia raíz de paginación (`CR3`):

| Rango Virtual | Uso | Permisos |
| --- | --- | :---: |
| `0x0000_0000_0000_0000` – `0x0000_0000_003F_FFFF` | **Página Cero y Guarda Inferior (4 MB)** (Atrapa desreferenciaciones de punteros nulos) | Inaccesible |
| `0x0000_0000_0040_0000` – `0x0000_007F_FFFF_FFFF` | Binario de Usuario (ELF PIE base aleatorizada por ASLR) | $R-X$ / $RW-$ |
| `0x0000_0080_0000_0000` – `0x0000_6FFF_FFFF_FFFF` | Heap de Usuario (`sys-libc`) y VMAs de `sys_vm_map` | $RW-$ / $R--$ |
| `0x0000_7000_0000_0000` – `0x0000_7EFF_FFFF_FFFF` | Buffers Gráficos Compartidos (VMOs con derecho `MAP`) | $RW-$ |
| `0x0000_7F00_0000_0000` – `0x0000_7FFF_FFFF_0000` | Stacks de Hilos de Usuario (8 MB por defecto + Guard Page) | $RW-$ |

---

## 4. Reglas de Paginación y Seguridad de Hardware

1. **W^X Estricto (Write XOR Execute):**
   - El bit NX (No-Execute, bit 63 en PTEs) está **siempre activo** en toda página escribible.
   - Ninguna página puede tener simultáneamente el bit `Write` en 1 y el bit `NX` en 0. Si un proceso llama a `sys_vm_protect(prot = W | X)`, la llamada falla con `EINVAL`.
2. **SMEP y SMAP Obligatorios:**
   - **SMEP (Supervisor Mode Execution Prevention, CR4 bit 20):** La CPU entra en pánico si el kernel intenta ejecutar una instrucción ubicada en una dirección del espacio de usuario (`< 0x0000_8000_0000_0000`).
   - **SMAP (Supervisor Mode Access Prevention, CR4 bit 21):** El kernel no puede leer ni escribir datos de usuario salvo explícitamente envuelto en bloques `copy_from_user` / `copy_to_user` usando las instrucciones `STAC` / `CLAC`.
3. **Aislamiento en Context Switch:**
   - Al conmutar entre hilos del mismo proceso, el registro `CR3` se preserva intacto para evitar purgar la TLB.
   - Al conmutar entre procesos distintos, se carga el nuevo `CR3`. Con soporte de PCID (Process-Context Identifiers), el flush de TLB es selectivo.

---

## 5. Implementación actual

Comportamiento observado en `kernel/src/memory/` y `kernel/src/task/elf.rs`. Los apartados siguientes describen hechos del código; no modifican la especificación de las secciones anteriores.

### 5.1 Inicialización

`memory::init()` ejecuta `pmm::init()` → `vmm::init()` → `heap::init()` en ese orden (`kernel/src/memory/mod.rs`).

### 5.2 Heap del kernel

```rust
pub const HEAP_START: usize = 0xFFFF_9000_0000_0000;
pub const INITIAL_HEAP_SIZE: usize = 8 * 1024 * 1024;   // 8 MB
pub const MAX_HEAP_SIZE: usize = 256 * 1024 * 1024;     // 256 MB
```

- El espacio virtual del heap es `0xFFFF_9000_0000_0000`, no la región `0xFFFF_C000_…` de la §2.2.
- El asignador global es `linked_list_allocator::LockedHeap`; `memory/slab.rs` y `memory/buddy.rs` coexisten como módulos aparte.
- `init()` mapea `INITIAL_HEAP_SIZE / PAGE_SIZE` marcos y llama a `ALLOCATOR::init`.
- El crecimiento dinámico reserva marcos bajo demanda hasta `MAX_HEAP_SIZE`.

### 5.3 Respaldos del mapeo directo

`memory/pmm.rs`, `memory/vmm.rs` y `memory/slab.rs` usan `HHDM_REQUEST` y, en su defecto, `0xFFFF_8000_0000_0000`, coincidente con la §2.1.

### 5.4 Pila de usuario

```rust
const USER_STACK_TOP: u64  = 0x0000_7FFF_FFFF_F000;
const USER_STACK_PAGES: u64 = 16;
```

- 16 páginas (64 KiB) mapeadas hacia abajo desde `USER_STACK_TOP`.
- Cada página se mapea con `user = true`, `writable = true`, `nx = true`.
- No se inserta ninguna *guard page*.
- La pila ocupa la banda superior del espacio de usuario, no el rango `0x0000_7F00_…` de la §3.
- `validate_load_segments` y `load_elf` calculan `stack_start = USER_STACK_TOP - USER_STACK_PAGES * PAGE_SIZE` y rechazan los ELF cuyos segmentos lo crucen.

### 5.5 Pila de kernel

`KERNEL_STACK_SIZE` es `256 * 1024` (256 KiB) en `kernel/src/task/scheduler.rs`. Se obtiene con `Layout::from_size_align(256 * 1024, 16)` y no incluye *guard page*. `schedule()` fija `TSS.RSP0` a `kernel_stack_ptr + KERNEL_STACK_SIZE`.

### 5.6 Carga del ELF

`kernel/src/task/elf.rs` declara `USER_ADDRESS_LIMIT = 0x0000_8000_0000_0000` y `MAX_ELF_MAPPED_BYTES = 256 MiB`.

**Formato.** `load_elf` exige `Type::Executable` (`ET_EXEC`); un `ET_DYN` (PIE) se rechaza con «ELF no es un ejecutable». Cada segmento `PT_LOAD` se mapea en su `virtual_addr` declarada, sin reubicación ni ASLR. Permisos por segmento: `writable` y `nx = !executable`, con `map_page(virt, phys, user = true, writable, nx)` sobre una `AddressSpace` cuyo `Drop` libera los marcos si algún paso falla.

**Validación previa.** Antes de mapear, `validate_load_segments` recorre los segmentos y aplica estos controles, cada uno con su mensaje de error:

| Comprobación | Error |
| --- | --- |
| `mem_size != 0`, `file_size <= mem_size`, `file_end <= len(elf_data)` | «Tamaño de segmento ELF inválido» |
| `vaddr >= PAGE_SIZE` y `end <= USER_ADDRESS_LIMIT` | «Segmento ELF fuera del espacio de usuario» |
| `align` potencia de dos y `vaddr % align == offset % align` | «Alineación de segmento ELF inválida» |
| **no** (`is_write() && is_execute()`) | «Segmento ELF viola W^X» |
| `end_page <= stack_start` | «Segmento ELF invade la pila reservada» |
| suma de páginas `<= MAX_ELF_MAPPED_BYTES` | «Imagen ELF excede el límite de memoria permitido» |
| rangos de página de segmentos disjuntos | «Segmentos ELF comparten páginas virtuales» |
| existe al menos un `PT_LOAD` y el punto de entrada cae dentro de un segmento ejecutable | «ELF sin segmento cargable ejecutable válido» |

Los desbordamientos intermedios (`checked_add`) devuelven «Overflow en segmento ELF» o «Overflow en datos ELF».

Este es el único punto en el que la regla W^X de la §4.1 se aplica hoy: se valida sobre los flags del ELF, no sobre las tablas de páginas. `sys_vm_protect` (`0x204`) no tiene handler.

### 5.7 Validación de rangos

`validate_user_buffer` en `kernel/src/task/syscall.rs` fija el límite inferior en `0x1000` (una página) y el superior en `0x0000_8000_0000_0000`; la guarda inferior de la §3 (4 MB) no se aplica en esta función.

### 5.8 Paginación y CPU

- `PAGE_NX = 1 << 63` se aplica cuando `nx` es verdadero (`kernel/src/memory/vmm.rs`).
- No se escribe el registro `CR4` en ninguna parte del árbol, por lo que **SMEP y SMAP no están activados**.
- No se usa PCID ni se emite ningún `INVLPID` o flush selectivo; la conmutación de `CR3` recarga la raíz completa.
