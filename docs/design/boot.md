# Arranque y carga del sistema

> **Tipo:** diseño · **Estado:** borrador · **ADR:** 07, 13
> **Relacionado:** [Almacenamiento](storage.md) · [Consola](console.md) · [Construcción y ejecución](../guides/build.md)

Describe la abstracción de arranque (`BootInfo`), el desacoplamiento respecto a cargadores externos (Limine como shim transitorio hacia el bootloader nativo `LixBoot`), el orden de inicialización del kernel, la carga del initrd y la transferencia de control al espacio de usuario.

## 1. Desacoplamiento del Bootloader y Contrato `BootInfo`

El kernel se diseña bajo un principio estricto de **independencia del bootloader** (ADR-07). Limine se utiliza temporalmente como cargador de desarrollo y pruebas en UEFI x86_64, pero el núcleo no se acopla a sus estructuras propietarias ni a sus protocolos de multiprocesamiento. En el futuro, el cargador nativo `LixBoot` asumirá el control de la fase previa al kernel entregando idéntico contrato de datos.

### 1.1 Estructura Agnóstica `BootInfo`

Toda la información entregada por el cargador (sea Limine a través de `kernel/src/boot/limine.rs` o `LixBoot` en el futuro) se traduce a una estructura pura de interfaz:

```rust
#[repr(C)]
pub struct BootInfo {
    pub hhdm_offset: u64,
    pub rsdp_addr: u64,
    pub memory_map_addr: u64,
    pub memory_map_entries: u64,
    pub memory_map_entry_size: u64,
    pub framebuffer: BootFramebuffer,
    pub initrd_addr: u64,
    pub initrd_size: u64,
    pub cmdline_addr: u64,
    pub cmdline_len: u64,
}
```

### 1.2 Peticiones del Shim Provisional (Limine)

En la etapa de desarrollo con Limine, el kernel se enlaza como binario higher-half en `0xFFFFFFFF_8000_0000` (`kernel/linker.ld`) y registra peticiones mínimas en `.requests`:

| Petición | Utilidad en el arranque |
| --- | --- |
| `BaseRevision` | Negociación de versión del protocolo |
| `MemoryMapRequest` | Mapa de memoria usado por el asignador de frames (`kernel/src/memory/pmm.rs`) |
| `HhdmRequest` | Offset del mapeo directo; respaldo `0xFFFF_8000_0000_0000` si el bootloader no lo entrega |
| `RsdpRequest` | Puntero físico a la estructura ACPI RSDP para enumeración de hardware |
| `ModuleRequest` | Initrd (imágenes `tar`) cargadas como módulos |
| `FramebufferRequest` | Primer framebuffer GOP para la consola |
| `StackSizeRequest` | Tamaño de la pila de Ring 0: `0x200000` (2 MiB) |

> [!IMPORTANT]
> **Prohibición de `limine::MpRequest`:** El kernel **no delega** el arranque multinúcleo al bootloader. Delegar el SMP a Limine rompería la compatibilidad cuando se introduzca `LixBoot`. El kernel enumera las CPUs secundarias directamente desde las tablas de hardware ACPI MADT (`'APIC'`) y las levanta mediante la secuencia canónica x86 Local APIC INIT-SIPI-SIPI.


## 2. Orden de inicialización

Secuencia de `_start()` en `kernel/src/main.rs`:

1. **Serial COM1** → se registra como primer sumidero de `klog`.
2. **Framebuffer** → se inicializa la consola sobre el primer framebuffer GOP y se registra como segundo sumidero.
3. **Banner** con versión, dirección de enlace higher-half y offset HHDM.
4. **Memoria** (`memory::init`): `pmm::init()` → `vmm::init()` → `heap::init()`.
5. **Arquitectura** (`arch::x86_64::init`): `gdt` → `idt` → `pic` → `apic` → MSRs de `SYSCALL` → `sti`.
6. **Sistema de archivos** (`fs::init`): inicialización del VFS.
7. **Dispositivos** (`drivers::pci::init`): barrido PCI → detección de AHCI → inicialización del disco → montaje de ext4 desde el disco primario si existe.
8. **Tareas y Multiprocesamiento** (`task::init`):
   - Inicialización de runqueues locales por CPU (`RUNQUEUES`).
   - Parseo de ACPI MADT (`'APIC'`) para enumerar cores físicos/lógicos y APIC IDs.
   - Secuencia de arranque de APs vía LAPIC ICR (INIT-SIPI-SIPI con trampolín en `0x8000`).
   - Creación de hilos de sistema (`idle` y `reaper`).
9. **Initrd**: cada módulo del bootloader se interpreta como archivo `tar` y sus entradas se vuelcan en la ramfs del VFS.
10. **Configuración de teclado**: `drivers::keyboard::load_config_from_vfs()` lee `/etc/keyboard.conf` o `/etc/vconsole.conf` y aplica la disposición si encuentra una clave `KEYMAP=` / `LAYOUT=`.
11. **Shell**: se intenta cargar `/bin/vanish.elf` en Ring 3; si falla, se crea un hilo de kernel ocioso como respaldo.
12. Se marca el scheduler como iniciado en todos los núcleos online y se entra en `halt_loop()` (drenaje de serie + `hlt`).

## 3. Initrd

- Formato: archivos `tar` planos. El parser (`kernel/src/fs/tar.rs`) reconoce solo entradas de fichero regular (`type_flag` `0` o `'0'`); ignora directorios y enlaces simbólicos, no valida el checksum y no implementa extensiones GNU/PAX.
- Cabecera: nombre en los bytes 0–100 (hasta NUL, sin prefijo ustar), tamaño en octal en los bytes 124–135, avance `512 + align512(size)`.
- Contenido: se copia a la lista `ramfs_files` del VFS (`kernel/src/fs/vfs.rs`) como pares `(ruta, bytes)`.

## 4. Carga del espacio de usuario

1. Se resuelve la ruta de la shell con `fs::vfs::read_file` intentando, en orden: `/bin/vanish.elf` y `/vanish.elf`.
2. Se carga con `task::elf::load_elf(datos, "/bin/vanish.elf", privilege = 3)` (`kernel/src/task/elf.rs`); el nombre registrado es siempre `/bin/vanish.elf` con independencia de la ruta resuelta.
3. El PID resultante se publica en `scheduler::SHELL_PID` y `spawned_user_shell` pasa a `true`. Si el ELF no valida, se imprime `AVISO: Error cargando vanish.elf …` y se continúa con el respaldo.
4. Si ninguna ruta produce un ELF válido, se crea un hilo de kernel sobre `arch::x86_64::halt_loop` con nombre `idle_shell` y su PID se publica también en `scheduler::SHELL_PID`.

## 5. Bucle ocioso

`arch::x86_64::halt_loop()` drena el puerto serie hacia la cola estándar de entrada y ejecuta `hlt` en bucle. La planificación se reanuda en cada interrupción del temporizador.

## 6. Segmentos enlazados

`kernel/linker.ld` produce tres segmentos `PT_LOAD`:

| Segmento | Permisos |
| --- | --- |
| `.text` | R-X |
| `.rodata` | R-- |
| `.data` / `.bss` | RW- |

La sección `.requests` se conserva dentro de `.data`. Las secciones `.eh_frame*` y `.note*` se descartan, por lo que no hay información de unwind DWARF disponible en tiempo de ejecución.
