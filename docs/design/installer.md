# Instalador, LiveISO y herramientas de build

> **Tipo:** diseño · **Estado:** borrador · **ADR:** 18, 21
> **Relacionado:** [Construcción y ejecución](../guides/build.md) · [Almacenamiento](storage.md)

Define las piezas del LiveISO y del instalador, el presupuesto de tamaño, la máquina de estados de instalación y las garantías sobre los datos del usuario.

Prioridades: **no destruir datos ajenos, no dejar un equipo a medias, ISO pequeño, reproducible en CI.**

## 1. Piezas

| Pieza | Dónde corre | Función |
| --- | --- | --- |
| `lix-mkiso` | host (build) | Construye kernel, rootfs, ESP y el ISO híbrido UEFI |
| `lix-installer` | LiveISO | Flujo de instalación (TUI en TTY) |
| `lix-part` | LiveISO / instalador | Lee y escribe tablas GPT |
| `lix-mkfs` | LiveISO / instalador | Crea ext4 y FAT32 mínimos |
| `lix-bootctl` | sistema instalado | Instala y actualiza LixBoot, gestiona entradas |

## 2. El ISO

### 2.1 Contenido

```text
lixos-<version>-x86_64.iso   (híbrido UEFI: El Torito + GPT)
├── EFI/BOOT/BOOTX64.EFI      cargador (Limine → LixBoot)
├── boot/lix-kernel.elf
├── boot/lix-root.img          rootfs comprimido (Zstd) con manifiesto
├── boot/manifest.sig          firma y hashes
└── boot/boot.cfg
```

### 2.2 Presupuesto de tamaño

- Objetivo: **ISO ≤ 150 MB**, con **rootfs descomprimido ≤ 400 MB**. El tope duro de 1 GB (ADR-18) es solo un límite de seguridad.
- Rootfs comprimido con **Zstd nivel 19+ y diccionario**: el costo de comprimir se paga solo al generar el ISO.
- Binarios con `strip`, LTO, `--gc-sections` y `opt-level="s"` en lo que no es crítico para el rendimiento.
- Contenido mínimo: kernel, Vanish, PML, `lix-installer`, `authd`, utilidades base. Sin documentación extensa, sin locales ni fuentes que no se usen.
- Si el enlace estático (ADR-06) duplica demasiado código, se agrupan las utilidades base en un binario único tipo *multicall* (estilo BusyBox).
- **El CI falla si el ISO supera el presupuesto**, igual que un benchmark que regresiona.

### 2.3 Arranque del LiveISO
1. El firmware UEFI arranca el cargador desde el ISO.
2. El cargador lee `lix-kernel.elf` y `lix-root.img` y los pasa en RAM (`BootInfo.initrd_*`).
3. El kernel verifica el hash del rootfs, lo descomprime en `tmpfs` y arranca `init`.
4. El medio puede retirarse después.
5. Si la RAM libre tras descomprimir queda por debajo de un mínimo (por ejemplo 1 GB), el arranque avisa en pantalla en vez de continuar de forma inestable.

## 3. Flujo del instalador

```text
┌──────────┐   ┌──────────┐   ┌───────────┐   ┌──────────┐   ┌───────────┐
│ 1.Preflight├─►│ 2.Disco  ├─►│ 3.Plan    ├─►│ 4.Ajustes ├─►│ 5.Instalar │
└──────────┘   └──────────┘   └───────────┘   └──────────┘   └─────┬─────┘
                                                                    ▼
                                                           ┌────────────────┐
                                                           │ 6.Verificar y  │
                                                           │   reiniciar    │
                                                           └────────────────┘
```

### Paso 1: Preflight
- Arranque en **UEFI** (si no, se explica por qué no se puede continuar).
- RAM ≥ 4 GB, CPU x86_64 con las extensiones exigidas (por ejemplo SSE4.2, XSAVE).
- Se verifica el hash de `lix-root.img`.
- Se detectan discos (AHCI/NVMe) y se muestra marca, tamaño y particiones existentes.

### Paso 2: Elegir disco
- Se **excluye** el disco del que se arrancó el LiveISO.
- Se muestran las particiones y sistemas detectados (Windows, Linux, otro Lix).

### Paso 3: Plan (siempre se muestra antes de tocar nada)
Modos soportados en v1.0:

| Modo | Qué hace |
| --- | --- |
| **Disco completo** | Borra el disco elegido. GPT con ESP FAT32 de 512 MB + raíz ext4 con el resto. |
| **Junto a otro SO (espacio libre)** | Usa solo espacio **sin asignar**. **Reutiliza la ESP existente sin formatearla** y añade `\EFI\Lix\`. Crea solo la partición raíz ext4. |

- **No se redimensionan particiones existentes en v1.0** (ext4, NTFS). Es la operación más peligrosa para los datos y requiere herramientas maduras. Si no hay espacio libre, se explica cómo liberarlo desde el otro SO.
- No hay particionado manual en v1.0.
- No hay partición de swap: ZRAM cubre el caso (ADR-09).
- El plan lista: particiones a crear, dispositivo, tamaños, y **qué se borrará**. Para "Disco completo" se exige **escribir el nombre del disco** para confirmar.

### Paso 4: Ajustes
- Idioma y distribución de teclado (US, ES).
- Zona horaria. El reloj del hardware se trata como UTC. Si hay Windows, se avisa del conflicto de RTC.
- Nombre del equipo.
- Usuario y contraseña (la verifica `authd` con Argon2id). El primer usuario va al grupo que puede usar `vdo`.
- Todo se guarda en un archivo `install.toml` que también permite instalación desatendida (sección 6).

### Paso 5: Instalar (máquina de estados con registro)

| Fase | Acción | Reversible |
| --- | --- | --- |
| P1 | Escribir GPT y crear particiones (solo las nuevas) | sí, borrando solo lo creado |
| P2 | Formatear raíz ext4 (y ESP si es nueva) | sí |
| P3 | Copiar el rootfs a ext4 y generar `/etc` con los ajustes | sí |
| P4 | `fsync` y verificar la copia contra el manifiesto de hashes | sí |
| P5 | Instalar el cargador en la ESP | sí |
| P6 | Registrar la entrada de arranque (UEFI NVRAM) | **último paso** |

- **Regla clave:** la entrada de arranque se crea **al final**. Si algo falla antes, el equipo sigue arrancando como antes y lo único que sobra son particiones nuevas, que se limpian.
- Cada fase escribe un diario en `/tmp/installer.state` y en `/tmp/installer.log`. Si falla, se ofrece **reintentar** o **deshacer**.
- Si la NVRAM no se puede escribir (hay firmwares problemáticos), se usa la ruta de respaldo `\EFI\BOOT\BOOTX64.EFI` solo si no existe otra, y se avisa al usuario.
- Con dual-boot, LixBoot detecta Windows y Linux y crea el menú. **Nunca se modifica `bootmgfw.efi` ni el cargador de otros SO.**

### Paso 6: Verificar y reiniciar
- Se vuelven a leer los datos de disco y se comparan con el manifiesto.
- Se desmonta limpiamente y se reinicia pidiendo retirar el medio.
- Si falla, el log se copia a un lugar legible (por ejemplo a un USB si hay uno, o se muestra en pantalla).

## 4. Herramientas internas

### `lix-part`
- Lectura y escritura de **GPT** (con CRC32 y copia de respaldo). MBR solo se reconoce para listar discos, no se escribe.
- Alineación a 1 MiB. Se rechaza cualquier tabla inconsistente en lugar de reparar a ciegas.
- La escritura de GPT se habilita junto al flujo de instalación.

### `lix-mkfs`
- **ext4:** solo el subconjunto soportado por el driver (ADR-08): `extents`, `flex_bg`, `64bit`, `metadata_csum`, `dir_index`, con journal. Se construye sobre el código de escritura del driver ext4.
- **FAT32 mínimo:** solo lo necesario para la ESP.
- **Validación en CI:** una imagen creada por `lix-mkfs` debe pasar `e2fsck -fn` y `fsck.fat -n` de las herramientas estándar.

### `lix-bootctl`
- Instala/actualiza el cargador, lista entradas y mantiene la **entrada del kernel anterior** como respaldo (si un kernel nuevo no arranca, el menú permite volver al previo).

### `lix-mkiso` (en el host)
- Compilación reproducible: toolchain fijada, mismas entradas dan el mismo hash.
- Genera ESP, rootfs, manifiesto con hashes (BLAKE3 o SHA-256) y firma.
- Comprueba el presupuesto de tamaño y falla si se supera.
- Usa `xorriso` para el ISO híbrido.

## 5. Seguridad del instalador
- El manifiesto firmado impide instalar un rootfs alterado.
- Las contraseñas no se escriben a disco ni a logs en claro. Los buffers se limpian con `zeroize`.
- Los parámetros de Argon2id se calibran para el hardware donde se instala.
- El instalador **solo** puede escribir en el disco elegido, mediante un handle con derecho de escritura exclusivo a ese dispositivo (ADR-05).

## 6. Pruebas de integridad de datos

El instalador es el componente con mayor impacto sobre los datos del usuario; sus pruebas verifican que un fallo en cualquier fase no comprometa el disco.

| Prueba | Qué valida |
| --- | --- |
| Instalación desatendida en QEMU+OVMF (`install.toml`) y arranque posterior | camino feliz |
| Disco de 8 GB, 32 GB y 1 TB, NVMe y SATA | tamaños y controladores |
| Imagen con Windows simulado (ESP+NTFS falsa) | dual-boot sin tocar lo existente |
| **Corte de energía simulado en cada fase** (P1 a P6) | recuperación y limpieza |
| Disco con GPT corrupta | rechazo sin escribir |
| Falta de RAM en LiveISO | mensaje claro, sin cuelgue |
| `e2fsck`/`fsck.fat` sobre el resultado | validez de los filesystems |
| Hash de rootfs alterado | el instalador se niega |

Todo es automatizable en CI con QEMU. Solo el hardware real se prueba a mano.

## 7. Fuera de v1.0
Redimensionar particiones, particionado manual, cifrado de disco, RAID/LVM, instalador gráfico (requiere la shell gráfica), actualización del sistema in situ con rollback atómico (el mecanismo de respaldo disponible es conservar el kernel anterior).

## 8. Decisiones abiertas
- Manifiesto con **SHA-256** (estándar, verificable con herramientas externas y acelerado por SHA-NI donde exista). BLAKE3 se reevalúa si la verificación pesa en el tiempo de arranque del LiveISO.
- Si el LiveISO ofrece una opción "probar sin instalar" con persistencia (no en v1.0).

---

## 9. Implementación actual

Comportamiento observado en `tools/`, `kernel/src/fs/` y `xtask/src/main.rs`. Los apartados siguientes describen hechos del código; no modifican la especificación de las secciones anteriores.

### 9.1 Herramientas

| Herramienta de la §1 | Archivo | Estado |
| --- | --- | --- |
| `lix-part` | — | no existe ningún crate ni binario |
| `lix-mkfs` | — | idem |
| `lix-bootctl` | — | idem |
| `lix-installer` | `tools/lix-installer/src/` | inspección/plan offline y creador de imagen raw nueva; no escribe discos físicos ni instala desde LiveISO |
| `lix-mkiso` | `tools/lix-mkiso/src/main.rs` | 2 líneas: `println!("lix-mkiso v0.0.1")` |

`tools/lix-installer` y `tools/lix-mkiso` son miembros del workspace; `lix-installer` aún no se empaqueta en el LiveISO y `lix-mkiso` sigue siendo un stub.

El crate compartido `libs/lix-storage` valida CRC32, límites, GUID duplicados y solapamientos de entradas, y serializa layouts GPT; kernel, generador de imagen e inspector/planificador host usan el mismo formato. El plan de `whole-disk` exige confirmar el nombre exacto del destino; `free-space` requiere una ESP única, la conserva y solo propone una raíz en un hueco no asignado. El comando nuevo `image` genera una imagen raw completa, pero exclusivamente en una ruta nueva: nunca abre nodos de dispositivo ni sobrescribe archivos existentes. No es aún el aplicador transaccional del LiveISO.

Comandos actuales:

```text
lix-installer inspect <IMAGE> [BLOCK_SIZE]
lix-installer plan <IMAGE> <whole-disk|free-space> [--confirm-target <IMAGE>] [BLOCK_SIZE]
lix-installer image <NEW_IMAGE> <KERNEL> <INITRD> <BOOTX64.EFI> <ROOTFS_DIR> [SIZE_GIB] [BLOCK_SIZE]
```

`inspect` y `plan` solo aceptan imágenes raw regulares; validan MBR protector, ambas cabeceras GPT, ambas tablas y CRC/solapamientos. `image` solo crea una imagen raw nueva (4–128 GiB), con GPT, ESP FAT32, Limine en `EFI/BOOT/BOOTX64.EFI`, configuración con `root=PARTUUID` y ext4 inicializado desde un rootfs. Admite LBA lógico de 512 o 4096 B (por defecto 512). Requiere Linux, `mkfs.ext4` y `/dev/urandom`, además de que la carpeta destino pertenezca al usuario y no permita escritura a grupo/otros. Construye en un staging oculto, sincroniza, y publica mediante hard link sin reemplazo; así el nombre final no aparece hasta que la imagen está completa. Un corte abrupto puede dejar un staging oculto, nunca una imagen final parcial. No escribe device nodes y no soporta todavía aplicar el plan a un disco existente ni el modo `free-space`.

Ejemplo después de `cargo run -p xtask -- iso`:

```text
cargo run -p lix-installer -- image target/lixos_installed.img target/x86_64-unknown-none/debug/kernel target/iso_root/initramfs.tar /usr/share/limine/BOOTX64.EFI rootfs 10 512
```

El archivo resultante debe validarse con QEMU antes de cualquier uso; no se recomienda como sustituto de una instalación en hardware.

### 9.2 Generación del ISO y prueba NVMe

La imagen la produce `xtask iso` (`xtask/src/main.rs`): crea `target/iso_root/`, escribe `initramfs.tar` con `tar` desde `rootfs/`, copia kernel y configuración Limine, invoca `xorriso` y `limine bios-install`. La generación del ISO todavía conserva compatibilidad BIOS del flujo existente; el objetivo de instalación real sigue siendo UEFI.

`cargo xtask nvme-image [PATH]` crea una imagen de prueba GPT+ext4 de 10 GiB con una partición Linux desde LBA 2048; rechaza sobrescribir incluso enlaces simbólicos existentes. `xtask run` crea `target/lixos_nvme.img` si no existe, o reutiliza un archivo regular y lo conecta a QEMU en modo estrictamente read-only. Esta imagen sirve para pruebas del driver y no es un medio instalable ni una recomendación para discos reales.

`cargo xtask nvme-image-4k [PATH]` genera la variante con LBA lógico de 4 KiB; `cargo xtask run -- --headless --nvme-4k` arranca esa variante en QEMU. `cargo xtask test-nvme` automatiza UEFI+OVMF en namespaces de 512 B y 4 KiB con `root=PARTUUID`, crea imágenes instaladas de ambos tamaños y arranca solo desde cada NVMe (sin CD); también prueba que un PARTUUID inexistente no monte ninguna raíz. Los discos de prueba se abren read-only durante QEMU.

### 9.3 Estado del camino de arranque desde NVMe

El kernel detecta una controladora NVMe PCIe estándar, inicializa un namespace, valida ambas cabeceras GPT y ambas copias de la tabla (CRC, geometría y coincidencia), selecciona una única partición Linux, y monta ext4 de solo lectura relativo al inicio de dicha partición. El VFS dispone de una capa superior temporal en RAM. El ISO de desarrollo ahora arranca por defecto con `root=none`, conservando initrd como raíz Live para no montar automáticamente una partición del disco objetivo; una imagen instalada sigue usando `root=PARTUUID`. `xtask test-nvme` valida automáticamente NVMe virtual con LBA de 512 B y 4 KiB, además del arranque UEFI desde imágenes producidas por `lix-installer image` con ambos tamaños; aún no está validado en hardware real.

La ABI de bloque usa `SYS_BLOCK_OPEN` para adquirir un handle exclusivo sobre el NVMe primario; `SYS_BLOCK_INFO`, `SYS_BLOCK_READ` y `SYS_BLOCK_WRITE` requieren ese handle y sus derechos. El lease impide que dos procesos accedan simultáneamente al dispositivo por esta ABI y se libera al cerrar el handle o destruir su proceso. Cada transferencia está acotada a una página DMA, los LBA se verifican contra la geometría, y el camino de escritura termina con NVMe Flush. La apertura requiere nivel de privilegio 0 o 1 y el handle writable se pide explícitamente; el nivel 1 no constituye autenticación, y el lease solo serializa accesos hechos mediante esta API. Hasta integrar un instalador Live y evitar otros accesos al dispositivo durante la instalación, esto es infraestructura de desarrollo, no una garantía transaccional ni una frontera de seguridad para datos de otros usuarios.

Quedan fuera del camino soportado: varios namespaces, PRP list para DMA >4 KiB, VMD/RAID, NVMe USB, checksums/journal replay de ext4, persistencia de escrituras al root, aplicación del plan a discos físicos, preservación de particiones y registro explícito de entrada UEFI NVRAM. FAT32 está implementado solo en el creador de imágenes del host, no como filesystem del kernel. `root=PARTUUID` ya permite elegir una única raíz. Por tanto, LixOS puede arrancar desde una partición NVMe estándar preparada externamente o desde una imagen creada por `lix-installer image`, pero todavía no puede instalarse de forma segura en una partición NVMe real.

### 9.4 Sistema de ficheros

- `kernel/src/fs/ext4.rs` monta ext4 en una vista de partición y solo lectura. No hay escritura, journal replay ni `fsck` en kernel.
- `kernel/src/fs/vfs.rs` incluye overlay RAM con API de escritura/borrado, pero todavía no está conectado a syscalls de escritura.
- `xtask nvme-image` usa `mkfs.ext4` con un subconjunto de features compatible con el lector actual; no modifica imágenes existentes.

La prueba `test-nvme` cubre GPT/ESP/ext4 y el arranque de la imagen completa, no la aplicación a un disco ya existente. Las pruebas de la §6 (`e2fsck`, `fsck.fat`, cortes de energía, recuperación transaccional y escritura UEFI NVRAM) siguen pendientes. El instalador todavía no se incluye en el LiveISO, no abre discos físicos y no puede preservar/actualizar una GPT existente.
