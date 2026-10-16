# LixOS

> **Proyecto experimental.** LixOS es un sistema operativo de hobby escrito en Rust: kernel `no_std` para x86_64, *userspace* propio y herramientas de build en el host. No está listo para uso diario, no tiene garantías de compatibilidad (ADR-17) y gran parte del hardware que detecta **no** funciona todavía. Usa bajo tu propio riesgo y lee la [tabla de estado](#2-estado-honesto-del-sistema) antes de creer nada de lo que ves documentado.

LixOS aplica una arquitectura de autoridad por *handles* y *capabilities* (sin `root` omnipotente), un ABI de syscalls definido en una única fuente (`abi/syscalls.toml`, ADR-24) y drivers internos en el kernel monolítico (ADR-01). La identidad, el diseño y el nombre del producto están controlados por el proyecto, pero el código es libre: los *forks* y la redistribución están permitidos (ADR-16).

---

## 1. Características breves

- **Kernel** `#![no_std]` en Rust, arranque con Limine (UEFI/BIOS), paginación, planificador MLFQ con SMP, VFS con ext4, NVMe/AHCI, consola sobre framebuffer y un ABI de ~35 syscalls.
- **Userspace** en Ring 3: `vanish` (shell), `ltop` (TUI de tareas), `edit`, `lixinstall`, más servicios `authd`, `netmgr`, `bluetoothd` y `lix-compositor` (varios aún son *stubs*).
- **Cadena de build** con `xtask`: compila kernel + userspace, genera el ISO y arranca QEMU con un comando.

## 2. Estado honesto del sistema

Leyenda de columnas:

- **detectado** — el subsistema o dispositivo se identifica en el arranque (barrido PCI, mapa de memoria, GOP, descriptores USB).
- **driver enlazado** — hay código de driver compilado y enlazado en el kernel para ello.
- **operativo** — funciona de extremo a extremo (al menos en QEMU), no solo «compila».
- **validado en hardware** — hay evidencia en el repositorio de que se probó en equipo físico. **Hoy la respuesta es «no» en todas las filas:** todo lo probado lo está en QEMU.

| Subsistema | detectado | driver enlazado | operativo | validado en hardware |
| --- | :-: | :-: | :-: | :-: |
| Arranque (Limine, UEFI + BIOS híbrido) | sí | sí | sí (QEMU) | no |
| Memoria (mapa, PMM/VMM, COW) | sí | sí | sí | no |
| Procesos y syscalls (dispatcher, scheduler) | n/a | sí | sí (35 syscalls enlazadas) | no |
| Almacenamiento (PCI → NVMe/AHCI, GPT, ext4) | sí | sí | parcial (lectura sí; ext4 en solo lectura + overlay RAM) | no |
| Red — virtio-net (`1af4:1000`) | sí | sí | sí (obtiene DHCP en QEMU) | no |
| Red — DHCP | n/a | sí | sí (4-way handshake) | no |
| Red — ARP/IPv4/UDP | n/a | sí | parcial (implementado, **sin validación end-to-end en QEMU**) | no |
| Red — TCP | n/a | no (solo parser en `lix-net`, sin wiring en el kernel) | no | no |
| Red — DNS | n/a | no (parser y constructor de consultas en `lix-net`, sin resolver) | no | no |
| Wi‑Fi (AR9271/AR7010 USB, AX201) | sí (AR9271 vía xHCI; AX201 PCI detectado **sin driver**) | sí (`drivers/wifi.rs` + `lix-wifi`) | no (falta transporte HTC/WMI, asociación y supplicant) | no |
| Bluetooth (HCI sobre USB) | parcial (descriptores HCI reconocidos en xHCI) | no (`lix-bluetooth` es crate en el workspace, **no** está enlazado al kernel) | no (sin transporte ni pairing) | no |
| Autenticación (`services/authd`) | n/a | n/a (servicio de userspace) | parcial (Argon2id, AEAD y rate-limit implementados; falta RNG, persistencia y bootstrap de IPC) | no |
| Gráficos (framebuffer GOP, consola TTY) | sí (framebuffer vía Limine; **sin** driver de GPU, no hay DRM) | sí (`drivers/framebuffer/`) | sí (consola de texto); compositor y shell gráfica, no | no |

Notas:

- «No validado en hardware» no significa «roto»: significa que **no hay evidencia** de ejecución en equipo físico en este repositorio. La única validación descrita es QEMU (p. ej. la entrada PS/2 con `sendkey` hasta `vanish`).
- Los derechos de un proceso se calculan como `declared(path) & launcher_rights`: `BASE_USER` es el suelo (incluye `NET_CONNECT`, `INPUT_READ`, `ENUMERATE`; **no** `DEVICES`, `MANAGE`, `INSPECT`, `POWER` ni `AUTH`), los binarios de `/bin/system/` pueden declarar `SYSTEM` y `vanish` declara `SESSION`; nadie puede elevar lo que su lanzador no tenía (ver [docs/spec/capabilities.md](docs/spec/capabilities.md) §13.7).
- Servicios `authd`, `netmgr`, `bluetoothd` y `lix-compositor` arrancan como *stubs*: no están conectados al init/IPC.
- El detalle por subsistema está en [`docs/design/connectivity.md`](docs/design/connectivity.md), [`docs/design/storage.md`](docs/design/storage.md) y [`docs/design/security-auth.md`](docs/design/security-auth.md).

## 3. Requisitos

| Componente | Uso |
| --- | --- |
| Rust estable (`rust-toolchain.toml`: canal `stable`, componentes `rust-src`, `llvm-tools-preview`, target `x86_64-unknown-none`) | compilación |
| `qemu-system-x86_64`, `qemu-img` | ejecución y pruebas |
| `xorriso`, `limine` (`/usr/share/limine`), `mtools` | imagen ISO |
| `mkfs.ext4`, `binutils`/`gcc`, `git` | imágenes de disco y ensamblado de `kernel/asm/*.S` |
| Firmware OVMF/edk2 | arranque UEFI en QEMU |

Todo ello se comprueba e instala con:

```sh
cargo xtask deps
```

## 4. Compilar y ejecutar

```sh
cargo xtask build               # kernel + userspace (x86_64-unknown-none, -Z build-std)
cargo xtask run                 # build + ISO + imagen + QEMU
cargo xtask run --headless      # idem sin ventana gráfica
cargo xtask iso                 # genera target/lixos.iso
cargo xtask check               # cargo check --workspace
cargo xtask codegen             # regenera los artefactos del ABI desde abi/syscalls.toml
cargo xtask help                # lista completa de tareas
```

Pruebas y calidad:

```sh
cargo test --workspace                                  # pruebas del workspace; el recuento y los fallos reales los manda el job `test` del CI
RUSTC_BOOTSTRAP=1 cargo clippy --workspace -- -D warnings
cargo fmt --all -- --check                              # hoy falla por diffs preexistentes (ver docs/guides/testing.md)
```

El ABI tiene una única fuente de verdad: editá `abi/syscalls.toml`, después `cargo xtask codegen` y commiteá los tres artefactos generados (`kernel/src/task/syscall_table.rs`, `include/lix/syscall.h`, `libs/lix-sys/src/syscalls.rs`). El job `codegen-drift` del CI falla si se olvida.

## 5. Estructura del repositorio

| Ruta | Contenido |
| --- | --- |
| `kernel/` | kernel `#![no_std]`: `arch/`, `boot/`, `drivers/`, `fs/`, `memory/`, `task/`, `shell/`, `klog/`, `sync/` y ensamblado en `kernel/asm/` |
| `libs/` | crates `no_std` reutilizables: `lix-sys` (ABI), `sys-libc`, `lix-net`, `lix-storage`, `lix-usb`, `lix-wifi`, `lix-bluetooth`, `lix-edit`; `litui` es submódulo |
| `services/` | demonios de userspace: `authd`, `netmgr`, `bluetoothd`, `lix-compositor` |
| `userland/` | programas: `vanish`, `ltop`, `edit`, `lixinstall` (+ `linker.ld`) |
| `tools/` | herramientas de build/instalación: `lix-installer`, `lix-mkiso` |
| `abi/` | **fuente única del ABI**: `syscalls.toml` |
| `include/` | cabeceras C generadas: `include/lix/syscall.h` |
| `docs/` | documentación: especificaciones, diseño, ADR y guías |
| `rootfs/` | contenido del initramfs (binarios, `etc/`, pruebas) |
| `xtask/` | driver de build: `build`, `iso`, `qemu`, `codegen`, `deps`, `image`, `bench`, … |
| `pkg/` | `pml`, gestor de paquetes |

## 6. Documentación

Índice completo en [`docs/README.md`](docs/README.md). En corto:

| Documento | Cubre |
| --- | --- |
| [`docs/spec/syscall-abi.md`](docs/spec/syscall-abi.md) | convención de llamada, tabla completa de syscalls, errores y alias de Linux |
| [`docs/spec/capabilities.md`](docs/spec/capabilities.md) | handles, derechos, namespaces y revocación (qué existe hoy y qué está reservado) |
| [`docs/spec/memory-map.md`](docs/spec/memory-map.md) | reparto del espacio virtual y reglas de paginación |
| [`docs/spec/libc-compat.md`](docs/spec/libc-compat.md), [`docs/spec/porting-compat.md`](docs/spec/porting-compat.md) | compatibilidad C/POSIX y niveles T0–T3 |
| [`docs/design/`](docs/design/) | subsistemas: arranque, memoria, scheduler, almacenamiento, consola, gráficos, conectividad, instalador, red y seguridad |
| [`docs/adr/decisions.md`](docs/adr/decisions.md) | ADR-01 – ADR-26 y decisiones diferidas |
| [`docs/guides/build.md`](docs/guides/build.md), [`docs/guides/testing.md`](docs/guides/testing.md) | build, imágenes y pruebas/CI |

## 7. Licencia

El kernel, la shell, las utilidades y el sistema base se publican bajo la **GNU General Public License versión 2** (texto completo en [`LICENSE`](LICENSE)).

Hay **dos excepciones** documentadas en [`COPYING.EXCEPTIONS`](COPYING.EXCEPTIONS):

1. **Excepción de syscalls** (estilo Linux): usar la interfaz de syscalls de LixOS no sujeta tu programa a la GPL.
2. **Excepción de enlace** para `sys-libc`, `lix-sys` y `include/lix/*`: enlazarlos no impone la GPL a tu programa.

La marca «LixOS» tiene su propia política en [`TRADEMARK.md`](TRADEMARK.md): los *forks* están permitidos, el nombre de producto está reservado y no hay «distros» oficiales. El requisito de cabeceras SPDX por archivo (ADR-16) está **pendiente de aplicación**.

## 8. Advertencia

LixOS es un **proyecto experimental y en desarrollo activo**:

- Puede corromper datos, romperse entre commits y cambiar de ABI sin aviso hasta `1.0.0` (ADR-17).
- No ejecutes en hardware que te importe ni como sistema de producción; pruébalo en QEMU o en equipo de desecho.
- No hay soporte, garantía ni responsabilidad: el software se distribuye «tal cual» (GPLv2, secciones 11–12).
- Nada de esto es asesoría legal: revisá `LICENSE`, `COPYING.EXCEPTIONS` y `TRADEMARK.md` antes de redistribuir.
