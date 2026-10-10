# Almacenamiento: PCI, bloque, VFS y ext4

> **Tipo:** diseño · **Estado:** borrador · **ADR:** 08, 01
> **Relacionado:** [Arranque](boot.md) · [Instalador](installer.md) · [Construcción y ejecución](../guides/build.md)

Describe la cadena de acceso al almacenamiento: barrido PCI, controladora AHCI, abstracción de bloque, VFS y el subconjunto de ext4 que se soporta.

## 1. Barrido PCI

`kernel/src/drivers/pci.rs` usa el mecanismo de configuración #1 (`0xCF8` / `0xCFC`) y recorre bus `0..=255`, slot `0..31`, función `0..7` del segmento 0. Para NVMe respeta el bit multifunction; al encontrar un controlador habilita Memory Space y Bus Master con una escritura de solo 16 bits a Command (sin confirmar por accidente flags W1C de Status). Se prioriza NVMe estándar PCIe (`01/08/02`) y luego AHCI (`01/06/01`). ECAM, múltiples segmentos y VMD/RAID siguen fuera de alcance.

| Campo | Desplazamiento | Valor buscado |
| --- | --- | --- |
| NVMe class/subclass/prog-if | `01/08/02` | controlador NVMe estándar |
| AHCI class/subclass/prog-if | `01/06/01` | compatibilidad SATA secundaria |

El primer dispositivo coincidente se pasa a `ahci::init`. El barrido no asigna IRQ ni activa el bus master.

## 2. Abstracción de bloque

`kernel/src/drivers/block.rs` define direcciones DMA mediante `DmaBuffer` (una página física contigua), `BlockGeometry` y errores tipados. Su API opera en bloques lógicos y comprueba rangos; `PartitionDevice` limita cada petición a la partición antes de delegarla al dispositivo padre. Por ahora el límite DMA por petición es 4 KiB y los drivers hacen polling síncrono.

```rust
pub trait BlockDevice: Send + Sync {
    fn geometry(&self) -> Result<BlockGeometry, BlockError>;
    fn read_blocks(&self, start: u64, count: u16, buffer: &mut DmaBuffer) -> Result<(), BlockError>;
    fn write_blocks(&self, start: u64, count: u16, buffer: &DmaBuffer) -> Result<(), BlockError>;
}
```

No se pasa un puntero arbitrario de kernel a un controlador DMA ni se infiere la geometría a partir de sectores de 512 bytes.

## 3. NVMe PCIe estándar

`kernel/src/drivers/nvme.rs` inicializa una controladora NVMe de segmento 0 con BAR0 MMIO, colas admin/I/O de polling, Identify Controller/Namespace y un namespace (NSID 1). Comprueba CAP (tamaño de cola, stride, MPSMIN), espera RDY con timeout y soporta lectura/escritura de una página por comando mediante PRP1. Se rechazan tamaños de bloque superiores a 4 KiB. La integración QEMU valida sectores lógicos de 512 B y 4 KiB; siguen pendientes MSI/MSI-X, múltiples namespaces/colas, PRP lists, VMD/RAID, NVMe USB y validación en hardware real.

`kernel/src/drivers/gpt.rs` usa el parser compartido `libs/lix-storage` para validar CRC32 de cabecera primaria/backup, CRC de tabla, GUIDs duplicados, límites y solapamientos antes de exponer una única partición Linux filesystem. La misma librería serializa GPT para imágenes de prueba. `root=PARTUUID=` permite escoger una identidad; sin selector se exige una única raíz candidata. `root=none` mantiene el initrd como raíz Live y evita montar una partición del disco objetivo. Aún no hay recuperación desde una GPT primaria dañada ni escritura GPT desde el kernel.

## 4. Controladora AHCI (compatibilidad secundaria)

`kernel/src/drivers/ahci.rs`:

1. Lee el BAR5 (offset `0x24`) para obtener el ABAR y comprueba que sea memoria mapeada.
2. Mapea el ABAR en una región MMIO dedicada con caching deshabilitado.
3. Itera el registro `PI` y selecciona puertos con `det == 3` (dispositivo presente) e `ipm == 1` (sin errores de interfaz).
4. Según la firma del dispositivo:
   - `0x0101` → unidad SATA: se rebasa el puerto. Aún no se consulta ATA IDENTIFY/capacidad y por seguridad no se publica un dispositivo utilizable.
   - `0xEB140101` → SATAPI (CD-ROM): se registra en el registro, sin inicializar.
5. La lectura SATA existente está limitada por LBA32 y escritura no está implementada; AHCI no se considera aún camino de arranque soportado.

### 3.1 Rebasing del puerto

`port_rebase` detiene el motor de comandos con watchdogs de 1×10⁶ iteraciones y reserva **una página de 4 KiB** de región DMA:

| Offset | Estructura |
| --- | --- |
| 0 | Lista de comandos (CLB), 1 KiB |
| 1024 | Lista de FIS recibidos (FB) |
| 1280 | Tabla de comandos (CT) con **1 entrada PRDT** |

Al terminar se reinician `FRE` y `ST`.

### 3.2 Lectura

`read_sector` construye un FIS de escritura de registros (`0x27`) con comando `0x24` (**`ATA READ_EXT`**, LBA48) y una única entrada PRDT con `dbc = count * 512 - 1`. La operación es **polleo**: limpia `IS`, espera `TFD` sin `BSY` ni `DRQ` con un watchdog de 5×10⁶, activa `CI` y espera su bit con un timeout de 1×10⁷, comprobando el bit de error `0x40000000`.

Restricciones de la interfaz actual:

- La dirección LBA se trunca a `u32` (límite teórico 2 TiB).
- Un solo puerto activo y una sola entrada PRDT por comando.
- No hay identificación `ATA IDENTIFY`, por lo que el tamaño de la unidad no se consulta.

## 5. VFS y overlay en RAM

`kernel/src/fs/vfs.rs` define un trait de dos operaciones:

```rust
pub trait FileSystem: Send + Sync {
    fn read_file(&self, path: &str) -> Option<Vec<u8>>;
    fn list_files(&self, path: &str) -> Option<Vec<String>>;
}
```

La estructura `Vfs` mantiene:

- `ramfs_files`: lista de pares `(ruta, bytes)` procedentes del initrd.
- `root_fs`: referencia al sistema de archivos montado en la raíz, si existe.
- `overlay_files` y tombstones: capa superior escribible en memoria que oculta/escribe sobre root sin modificar ext4.

Reglas de resolución:

- `read_file` consulta overlay, root ext4 y luego initrd.
- `write_file` y `remove_file` modifican solo overlay/tombstones; no hacen persistencia.
- Las rutas se canonicalizan antes de consultar. `list_files` une root e initrd/overlay y filtra tombstones.
- No hay tabla de montajes, nodos índice ni syscall de escritura conectada todavía.

El sistema operativo no distingue tipos de archivo en el VFS: no hay escritura, creación, renombrado, borrado ni metadatos en esta capa.

## 6. ext4 (solo lectura)

`kernel/src/fs/ext4.rs` implementa el subconjunto de lectura del sistema de archivos.

### 5.1 Superbloque

Se lee el superbloque en byte offset 1024 relativo a la partición, usando la geometría del dispositivo, y `Ext4Fs::new` aplica estas comprobaciones; cualquiera que falle impide el montaje:

| Comprobación | Condición exigida |
| --- | --- |
| Magia | `s_magic == 0xEF53` |
| Tamaño de bloque | `s_log_block_size <= 2` (bloques de 1, 2 u 4 KiB) |
| Geometría | `s_inodes_per_group != 0` y `s_blocks_per_group != 0` |
| Tamaño de inodo | `s_inode_size >= size_of::<Ext4Inode>()`, `<= block_size` y `block_size % s_inode_size == 0` |
| Rasgos incompatibles | Actualmente se admiten `filetype` (0x02), `extents` (0x40), `64bit` (0x80) y los flags estructurales usados por la imagen de prueba. No se validan checksums ext4 ni se monta con journal replay. |
| Obligatorios | `incompat & 0x40` (extents) y `incompat & 0x02` (filetype) deben estar puestos |
| Recuento de bloques | `s_blocks_count_lo != 0` y el alto `blocks_count_hi` (leído sin alineación en el offset `0x150`) debe ser `0` |

Campos derivados:

| Campo | Uso |
| --- | --- |
| `s_log_block_size` | Tamaño de bloque: `1024 << s_log_block_size` |
| `s_feature_incompat & 0x80` | Habilita el modo 64-bit |
| `s_desc_size` | Tamaño del descriptor de grupo: 64 si 64-bit, 32 en otro caso |

Para el descriptor de grupo se exige además que, con 64-bit, `s_desc_size >= 64`; sin 64-bit, el tamaño queda fijado en 32; y en ambos casos que no supere `block_size`.

### 5.2 Inodos

La tabla de grupos (`BGDT`) empieza en el bloque 1, o en el 2 para bloques de 1 KiB. El inodo se localiza por desplazamiento de bytes dentro de la tabla, usando `inode_size` y el campo de alto orden `bg_inode_table_hi` cuando el formato es 64-bit.

### 5.3 Datos de archivo

`read_inode_data` exige un **árbol de extents**: si `eh_magic != 0xF30A` devuelve `None` (los bloques directos e indirectos no se soportan). Los controles aplicados son `eh_magic == 0xF30A`, `eh_depth == expected_depth` (la profundidad esperada en cada nivel) y `expected_depth <= MAX_EXTENT_TREE_DEPTH`, donde `MAX_EXTENT_TREE_DEPTH = 5`.

Se recorre el árbol de forma recursiva respetando la profundidad, se interpretan los extents sin escritura (`ee_len > 32768`) y se respeta `i_size_high` para ficheros regulares. `find_in_dir` repite la misma comprobación de magia y profundidad al leer cada directorio.

### 5.4 Directorios

`find_in_dir` recorre la lista de `Ext4DirEntry` de forma lineal con validación de `rec_len`. El componente de ruta se resuelve desde el **inodo raíz 2**, separando por `/` sin soporte de `..`.

### 5.5 Lectura de bloques

`read_block` asigna un frame del PMM, realiza la lectura, copia el contenido a un `Vec` y libera el frame. **No hay caché de bloques ni page cache**: cada petición repite la I/O.

### 5.6 Montaje

`init_from_primary_disk()` construye `Ext4Fs` sobre `GLOBAL_DISK`, lo publica en `EXT4` y redirige `VFS.root_fs` a `GLOBAL_EXT4`, dejando la ramfs del initrd como respaldo.

## 6. Secuencia de apagado del subsistema

`kernel/src/power.rs` (invocado por `sys_power`) desmonta el sistema en orden: se anula `VFS.root_fs`, se vacía `EXT4`, se libera `PRIMARY_DISK` y se deshabilitan las interrupciones antes de enviar los comandos de apagado por puertos de E/S (`0x604`, `0xB004`, `0x4004`, `0x501`).
