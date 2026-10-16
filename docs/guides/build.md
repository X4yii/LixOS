# Construcción y ejecución

> **Tipo:** guía · **Estado:** borrador · **ADR:** 07, 13
> **Relacionado:** [Arranque](../design/boot.md) · [Instalador](../design/installer.md) · [Pruebas](testing.md)

Describe el workspace, la toolchain, las tareas de `xtask` y la composición de las imágenes de arranque.

## 1. Estructura del workspace

`Cargo.toml` declara 21 miembros:

| Ruta | Crate | Función |
| --- | --- | --- |
| `xtask` | `xtask` | Driver de compilación e imágenes |
| `kernel` | `kernel` | Kernel `#![no_std]` |
| `libs/lix-sys` | `lix-sys` | Tipos, constantes y números de syscall (`src/syscalls.rs` generado) |
| `libs/sys-libc` | `sys-libc` | Capa de compatibilidad C/POSIX |
| `libs/lix-storage` | `lix-storage` | GPT, particiones y utilidades de almacenamiento |
| `libs/lix-net` | `lix-net` | Ethernet, ARP, IPv4, UDP, TCP, DNS, DHCP y checksums |
| `libs/lix-wifi` | `lix-wifi` | Descriptores y primitivas ath9k_htc |
| `libs/lix-bluetooth` | `lix-bluetooth` | Codificación/parseo de comandos y eventos HCI |
| `libs/lix-usb` | `lix-usb` | Parseo de descriptores e informes USB |
| `libs/lix-edit` | `lix-edit` | Lógica de edición compartida por el editor |
| `services/authd` | `authd` | Servicio de autenticación |
| `services/netmgr` | `netmgr` | Gestor de interfaces y perfiles de red |
| `services/bluetoothd` | `bluetoothd` | Gestor de controladores HCI |
| `services/lix-compositor` | `lix-compositor` | Compositor en userspace |
| `userland/vanish` | `vanish` | Shell de espacio de usuario |
| `userland/edit` | `edit` | Editor de texto |
| `userland/ltop` | `ltop` | Gestor de tareas en TUI |
| `userland/lixinstall` | `lixinstall` | Instalador en userspace |
| `pkg/pml` | `pml` | Gestor de paquetes |
| `tools/lix-installer` | `lix-installer` | Instalador / generador de imágenes |
| `tools/lix-mkiso` | `lix-mkiso` | Constructor de ISO |

`libs/litui` **no** es miembro del workspace: es un submódulo git con su propio workspace (`litui-core`, `litui-widgets`, `litui`) consumido por `ltop` mediante dependencias de ruta. Los crates `libs/lix-ui` y `userland/lix-shell` fueron **retirados** del árbol.

Paquetes con valores propios: `edition = "2024"` y `rust-version = "1.85.0"` en `[workspace.package]`.

### 1.1 Módulos del kernel

`kernel/src/` se organiza en: `arch/` (x86_64: idt, gdt, apic, smp, percpu…), `boot/` (peticiones Limine y `BootInfo`), `drivers/`, `fs/` (`vfs`, `ext4`, `tar`, `permissions`, `error`), `klog/`, `memory/` (`pmm`, `vmm`…), `shell/`, `sync/`, `task/` y `panic.rs`/`power.rs`. `task/` contiene:

```
kernel/src/task/
├── scheduler.rs            raíz del módulo: planificador, señales, dispatcher de eventos
│   ├── metrics.rs          contadores de planificación
│   ├── runqueue.rs         colas MLFQ por CPU
│   ├── spawn.rs            creación de procesos
│   └── wait.rs             bloqueo y despertar
├── syscall/                handlers de syscalls
│   ├── mod.rs              dispatcher, validación de punteros, MSRs de SYSCALL
│   ├── block.rs  io.rs  net.rs  proc.rs  term.rs
├── syscall_table.rs        GENERADO por `cargo xtask codegen` (no editar)
├── elf.rs  handle.rs  process.rs  rights.rs  mod.rs
```

`kernel/src/drivers/` contiene `ahci.rs`, `block.rs`, `gpt.rs`, `nvme.rs`, `pci.rs`, `virtio_net.rs`, `net.rs`, `wifi.rs`, `xhci.rs`, `usb.rs`, `input.rs`, `tty.rs`, `serial.rs`, `ps2_keyboard.rs`, `font.rs` y dos módulos con subdirectorio:

- `framebuffer/{mod,theme,render,console,batch,ansi,cursor,hw,grid}.rs` — consola de texto sobre GOP.
- `keyboard/{mod,shared,es,latam}.rs` — layouts y capa común de entrada.

### 1.1 Perfiles

| Perfil | Ajustes |
| --- | --- |
| `dev` | `opt-level = 1`, `debug = true`, `overflow-checks = true`, `panic = "abort"` |
| `release` | `opt-level = 3`, `debug = true`, `lto = "fat"`, `codegen-units = 1`, `panic = "abort"` |
| `bench` | `codegen-units = 1`, `lto = true` |

## 2. Toolchain

`rust-toolchain.toml`:

```toml
[toolchain]
channel = "stable"
components = ["rust-src", "llvm-tools-preview"]
targets = ["x86_64-unknown-none"]
```

`.cargo/config.toml` define el alias `xtask` y no fija `RUSTFLAGS`: las banderas se inyectan por paquete desde `xtask/src/main.rs`.

## 3. Tareas de `xtask`

El alias es `cargo xtask <comando>`.

| Comando | Acción |
| --- | --- |
| `help` | Mensaje de uso (valor por defecto) |
| `build` | Inicializa submódulos, ensambla `.S` en `kernel/asm/` y compila kernel, `ltop` y `vanish` |
| `check` | `cargo check --workspace` |
| `codegen` | Regenera `kernel/src/task/syscall_table.rs`, `include/lix/syscall.h` y `libs/lix-sys/src/syscalls.rs` desde `abi/syscalls.toml` |
| `asm` | Ensambla archivos `.S` en `kernel/asm/` a objetos `.o` en `target/x86_64-unknown-none/asm_objs/` |
| `deps` | Verifica e instala dependencias del sistema requeridas (`qemu`, `xorriso`, `mtools`, `ovmf`, etc.) |
| `iso` | Ejecuta `build` y genera `target/lixos.iso` |
| `run` | Ejecuta `deps`, `iso`, crea la imagen de disco y lanza QEMU |
| `test-nvme` | Arranque UEFI+NVMe de 512 B y 4 KiB en QEMU con fixtures propios |
| `bench` / `microbench` | Suite de microbenchmarks con RDTSC |
| `nvme-image [PATH]` / `nvme-image-4k [PATH]` | Imágenes GPT+ext4 desechables de prueba |

Las pruebas unitarias **no** son una tarea de `xtask`: se ejecutan con `cargo test --workspace`. El formato y los lints también quedan fuera de `xtask` (`cargo fmt --all -- --check`, `RUSTC_BOOTSTRAP=1 cargo clippy --workspace -- -D warnings`) y se ejecutan en CI (§10).

### 3.1 Banderas de compilación

Definidas como constantes en `xtask/src/main.rs`:

| Constante | Valor |
| --- | --- |
| `KERNEL_RUSTFLAGS` | `-C code-model=kernel -C relocation-model=static -C force-frame-pointers=yes -C link-arg=-Tkernel/linker.ld -C link-arg=-nostdlib -C link-arg=-zmax-page-size=0x1000` |
| `USER_RUSTFLAGS` | `-C relocation-model=static -C force-frame-pointers=yes -C link-arg=-Tuserland/linker.ld -C link-arg=-nostdlib -C link-arg=-zmax-page-size=0x1000` |

Ambas rutas compilan con `RUSTC_BOOTSTRAP=1` y:

```
--target x86_64-unknown-none -Z build-std=core,alloc,compiler_builtins
```

El kernel se compila en perfil `dev`; `ltop` y `vanish` en `--release`.

### 3.2 Salidas

`task_build` compila cinco targets (`kernel`, `ltop`, `vanish`, `edit`, `lixinstall`) y después ejecuta `stage_rootfs()`:

| Artefacto | Destino |
| --- | --- |
| Kernel | `target/x86_64-unknown-none/debug/kernel` → `rootfs/boot/kernel.elf` y `target/iso_root/kernel.elf` |
| `ltop`, `lixinstall` (release) | `rootfs/bin/system/*.elf` |
| `vanish`, `edit` (release) | `rootfs/bin/apps/*.elf` |
| `BOOTX64.EFI` | `/usr/share/limine/BOOTX64.EFI` → `rootfs/boot/BOOTX64.EFI` (si Limine está instalado) |

`stage_rootfs()` borra las copias heredadas (`rootfs/*.elf`, `rootfs/bin/*.elf`, `rootfs/bin/dummy.txt`), crea `/boot`, `/bin/system`, `/bin/apps`, `/etc` y `/home/user`, y escribe `rootfs/etc/kernel.conf` y `rootfs/home/user/notas.txt` solo si no existen. Los binarios de userspace se copian con `strip` del host de forma best-effort: si `strip` no existe, se deja el binario sin stripear.

`init_submodules()` ejecuta `git submodule update --init --recursive` de forma best-effort; se omite si no hay directorio `.git`.

## 4. Imagen ISO

`task_iso` compone `target/iso_root/`:

```
target/iso_root/
├── EFI/BOOT/
│   ├── BOOTX64.EFI
│   └── limine.conf
├── limine.conf
├── limine.cfg
├── limine-bios-cd.bin
├── limine-uefi-cd.bin
├── limine-bios.sys
├── kernel.elf
└── initramfs.tar
```

- `initramfs.tar`: `tar -cvf target/iso_root/initramfs.tar -C rootfs .`
- Configuración de Limine: `timeout: 0`, `protocol: limine`, `kernel_path: boot():/kernel.elf`, `module_path: boot():/initramfs.tar`. Se escribe en las tres rutas anteriores.
- Los binarios de Limine se copian desde `/usr/share/limine`.
- Ensamblado con `xorriso -as mkisofs -R -r -J -b limine-bios-cd.bin -no-emul-boot -boot-load-size 4 -boot-info-table -hfsplus -apm-block-size 2048 --efi-boot limine-uefi-cd.bin -efi-boot-part --efi-boot-image --protective-msdos-label target/iso_root -o target/lixos.iso`.
- Post-proceso: `limine bios-install target/lixos.iso`.

Si `xorriso` no está disponible, `xtask` emite un aviso y continúa sin imagen.

## 5. Imagen de disco

`xtask nvme-image [PATH]` crea una imagen desechable GPT+ext4 para probar NVMe; por defecto usa `target/lixos_nvme.img` y nunca sobrescribe un archivo previo. `xtask run` reutiliza la imagen existente sin cambiarla:

```
cargo run -p xtask -- nvme-image
cargo run -p xtask -- nvme-image-4k
cargo run -p xtask -- test-nvme
```

El rootfs se copia con `mkfs.ext4 -d rootfs` dentro de una partición Linux GPT alineada a 1 MiB. Las dos imágenes prueban LBA lógico de 512 B y 4 KiB. Para regenerar una imagen, usa un PATH nuevo o mueve de forma deliberada la anterior; la tarea evita borrados implícitos. QEMU conecta el disco base como read-only y no puede persistir escrituras. Ejecuta `cargo run -p xtask -- run --headless --nvme-4k` para arrancar con namespace de 4 KiB.

`test-nvme` construye el ISO con `root=PARTUUID`, crea fixtures únicos temporales y arranca QEMU+OVMF con cada tamaño lógico; además crea imágenes instalables de 512 B y 4 KiB y las arranca sin CD. También arranca con un PARTUUID inexistente y exige que el kernel rechace la raíz. Requiere `qemu-system-x86_64`, QEMU image tools, `mkfs.ext4`, xorriso, Limine y OVMF code/vars. No escribe en dispositivos físicos ni reutiliza imágenes existentes.

Para generar una imagen UEFI con ESP FAT32 y raíz ext4 a partir de los artefactos del build:

```text
cargo run -p xtask -- iso
cargo run -p lix-installer -- image target/lixos_installed.img target/x86_64-unknown-none/debug/kernel target/iso_root/initramfs.tar /usr/share/limine/BOOTX64.EFI rootfs 10 512
```

`image` exige una ruta de salida inexistente, una carpeta destino privada y solo crea archivos regulares. Construye en un archivo staging oculto y publica el nombre final sin reemplazar un destino concurrente. El último parámetro puede ser `4096` para crear GPT+FAT32 para NVMe 4Kn. La integración `test-nvme` verifica ambos layouts arrancándolos desde NVMe en OVMF, sin CD; esto todavía no permite instalar en discos reales.

## 6. Ejecución en QEMU

| Parámetro | Valor |
| --- | --- |
| Máquina | `-M q35,accel=kvm:tcg` |
| Memoria | `-m 2G` |
| CD | `-cdrom target/lixos.iso` |
| Disco | `-drive id=disk,file=target/lixos_nvme.img,if=none,format=raw` |
| Controladora | `-device nvme,id=nvme0,serial=lixos-test` |
| Namespace | `-device nvme-ns,drive=disk,nsid=1` |
| Serie | `-serial stdio` |

**Firmware UEFI:** se sondean, en orden, `/usr/share/edk2/x64/OVMF.4m.fd`, `/usr/share/ovmf/x64/OVMF.4m.fd`, `/usr/share/edk2-ovmf/x64/OVMF_CODE.fd`, `/usr/share/OVMF/OVMF_CODE.fd`. Si ninguno existe, se arranca en modo BIOS híbrido.

**Modo de vídeo:** headless (`-display none`) si se pasa `--headless` / `-n`, o si no hay `DISPLAY` ni `WAYLAND_DISPLAY`.

## 7. Requisitos del host

| Herramienta | Uso |
| --- | --- |
| `qemu-system-x86_64` | Ejecución |
| `qemu-img` | Imagen de disco |
| `xorriso` | Ensamblado de ISO |
| `limine` | `bios-install` y binarios de arranque |
| `mkfs.ext4` | Imagen de disco |
| `strip` | Reducción de binarios (opcional) |
| Firmware OVMF/edk2 | Arranque UEFI |

## 8. Contenido de `rootfs/`

Lo que `stage_rootfs()` genera y lo que viaja en el initramfs:

```
rootfs/
├── bin/
│   ├── apps/
│   │   ├── edit.elf          copia de build (stripped)
│   │   ├── vanish.elf        copia de build (stripped)
│   │   └── hello.elf         binario C de prueba (heredado; se migra desde bin/hello.elf)
│   └── system/
│       ├── ltop.elf          copia de build (stripped)
│       └── lixinstall.elf    copia de build (stripped)
├── boot/
│   ├── kernel.elf            copia del kernel compilado
│   └── BOOTX64.EFI           copia de /usr/share/limine (si existe)
├── etc/
│   ├── keyboard.conf         disposición de teclado (KEYMAP=latam)
│   └── kernel.conf           HOSTNAME/SCHEDULER/LOGLEVEL/RING (si no existía)
├── home/user/
│   └── notas.txt             contenido inicial del directorio personal
├── lib/firmware/ath9k_htc/   firmware htc_9271-1.4.0.fw y su licencia
├── src/hello.c               fuente del binario de prueba
└── usr/share/licenses/       licencias empaquetadas
```

- `rootfs/etc/keyboard.conf` se lee en el arranque mediante `drivers::keyboard::load_config_from_vfs()` (ver [boot.md](../design/boot.md) §2 y [console.md](../design/console.md) §2.3). Documenta las 12 disposiciones disponibles y fija `KEYMAP=latam`.
- `rootfs/src/hello.c` es un binario de espacio de usuario mínimo que usa `int $0x80`; su salida estándar llama al número `1`, que **ya no** es `write` en el ABI actual (es `sys_handle_close`), mientras que `60` sí se resuelve a `sys_process_exit` vía alias. Ver [syscall-abi.md](../spec/syscall-abi.md) §8 y §10.6.
- Las copias heredadas `rootfs/*.elf`, `rootfs/bin/*.elf` y `rootfs/bin/dummy.txt` se eliminan en cada build.

Las extensiones `*.elf`, `*.iso`, `*.img` y el directorio `target/` están excluidos del repositorio por `.gitignore`.

## 9. Comprobación estática

`cargo xtask check` ejecuta `cargo check --workspace` con `RUSTC_BOOTSTRAP=1` y es la comprobación de tipos y sintaxis del árbol completo. No existe ninguna tarea de `fmt` o `clippy` en `xtask`: esos comandos se invocan directamente y viven en el pipeline de CI.

## 10. Integración continua

`.github/workflows/ci.yml` define el pipeline (detalle en [testing.md](testing.md)):

| Job | Comando | Propósito |
| --- | --- | --- |
| `fmt` | `cargo fmt --package <paquetes propios> -- --check` | formato (sin `libs/litui`: submódulo con licencia propia) |
| `clippy` | `RUSTC_BOOTSTRAP=1 cargo clippy --workspace --all-targets -- -D warnings` | lints y tipos del workspace |
| `codegen-drift` | `cargo xtask codegen` + `git diff --exit-code` | **impide que el ABI se desincronice** |
| `test` | `cargo test --workspace --locked` | pruebas de usuario y del kernel |
| `deps` | `cargo xtask deps` | dependencias de sistema + Limine en el runner |
| `smoke` | `cargo xtask smoke` | kernel + userland + ISO + arranque en QEMU con interacción por serie (`cargo xtask smoke --smp N`, `--no-build` para reutilizar la imagen) |

Todos los jobs usan `actions/checkout@v4` con `submodules: recursive` (necesario para `libs/litui`) y `dtolnay/rust-toolchain@stable` con `components: rust-src, rustfmt, clippy`.
