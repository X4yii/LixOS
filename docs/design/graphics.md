# Gráficos, entrada y shell gráfica

> **Tipo:** diseño · **Estado:** consolidado · **ADR:** 15, 25
> **Relacionado:** [ABI de syscalls §4.7](../spec/syscall-abi.md) · [Diseño de interfaz](ui-design.md) · [Consola](console.md)

Define la arquitectura gráfica en capas: `lix-drm` en el kernel (KMS y VMOs), micro-compositor en userspace con presentación Zero-Copy, y shell gráfica.

Objetivo: **una interfaz gráfica fluida, determinista y de baja latencia con aislamiento total de fallos: el renderizado y la composición residen en userspace mediante buffers compartidos Zero-Copy, mientras el kernel gestiona el hardware de pantalla (KMS) y la sincronización.**

## 1. Capas Arquitectónicas

```text
┌───────────────────────────────────────────────────────────────┐
│  Apps (terminal, editor, gestor de archivos...)  lix-ui       │
├───────────────────────────────────────────────────────────────┤
│  lix-shell   (panel, lanzador, fondo, notificaciones)         │
├───────────────────────────────────────────────────────────────┤
│  lix-compositor  (Userspace, clase Interactive, Zero-Copy)    │
╞═══════════════════════════════════════════════════════════════╡
│  Kernel:  lix-drm (KMS, VMOs VRAM/RAM, Fences, Direct Scanout)│
│           Consola TTY de Rescate (GOP fallback)               │
└───────────────────────────────────────────────────────────────┘
```

- **Aislamiento de Fallos (Estabilidad Primero):** La lógica de ventanas, orden Z, clipping y foco reside en `lix-compositor` (userspace). Un error en una aplicación gráfica o en el compositor nunca cuelga el kernel.
- **Respaldo Inmediato (TTY):** La consola de texto virtual (TTY 1-5) corre en el kernel sobre GOP/framebuffer básico, garantizando operatividad ante cualquier eventualidad.

---

## 2. Kernel: Módulo `lix-drm` y Consola TTY

El subsistema `lix-drm` arbitra el acceso al hardware de pantalla y memoria gráfica sin intervenir en la geometría de las ventanas.

### 2.1 Mode Setting y Gestión de Buffers
- **KMS (Kernel Mode Setting):** Detección de pantallas conectadas, resoluciones nativas y tasas de refresco. En v1.0 se apoya en UEFI GOP y drivers base (`virtio-gpu` en QEMU y DRM genérico).
- **VMOs Gráficos:** Asigna buffers lineales en VRAM o RAM del sistema con alineación de scanout. Estos buffers se exponen a userspace como _handles_ con derechos atenuables (`MAP`, `DISPLAY_OWN`).
- **Direct Scanout (Optimización Pantalla Completa):** Si una ventana ocupa toda la pantalla sin transparencia (ej. un reproductor o juego), `lix-drm` puede programar el scanout directo de su VMO al controlador de pantalla, logrando rendimiento bruto de cero copias sin sobrecargar la CPU ni requerir overlays para ventanas regulares.

### 2.2 Sincronización y Presentación
- **Page Flipping y Vsync:** El compositor entrega el buffer final mediante `sys_display_present(display_h, vmo_h, &damage_rects, n)`. El flip se sincroniza con la interrupción de borrado vertical (VBlank) de la GPU o timer monotónico, eliminando el tearing.
- **Fences / Sync Objects:** Primitivas de sincronización asíncrona entre GPU y CPU para evitar lecturas de buffers antes de finalizar el dibujo.

### 2.3 Consola de Texto de Rescate (TTY 1-5)
- Renderizado de texto bitmap directamente sobre el framebuffer físico. El diseño completo de esta consola está en [console.md](console.md).
- Si el compositor colapsa o la GPU entra en *hang*, el kernel conmuta limpiamente a TTY 1 para diagnóstico y reinicio del servicio por parte de `init`.

---

## 3. Kernel: Entrada (Input Routing)

- Drivers de teclado/ratón (PS/2, xHCI-HID) producen eventos normalizados `InputEvent` (keycodes físicos, coordenadas relativas/absolutas, botones).
- `sys_input_open` entrega un handle de lectura únicamente al proceso de la sesión gráfica que posee la capability `INPUT_READ` (el compositor), impidiendo que aplicaciones en segundo plano capturen pulsaciones de teclado (*anti-keylogging* por diseño).
- El compositor mapea keycodes a caracteres según la distribución del teclado y despacha los eventos a la ventana activa mediante IPC (`sys_channel_send`).

---

## 4. Renderizado en Aplicaciones: `lix-ui`

- Las aplicaciones cliente utilizan el toolkit oficial `lix-ui`.
- **Flujo Zero-Copy:**
  1. La aplicación solicita a `lix-drm` un buffer gráfico VMO con tamaño igual a su ventana.
  2. La aplicación mapea el VMO en su espacio virtual (`sys_vm_map`).
  3. Dibuja su interfaz (usando GPU shaders/Vulkan o el rasterizador 2D por software de `lix-ui`).
  4. Envía el handle del VMO y la lista de rectángulos sucios (*damage rects*) al compositor por el canal IPC de la ventana.

---

## 5. Micro-Compositor en Userspace (`lix-compositor`)

`lix-compositor` es un proceso no privilegiado que corre en la clase de prioridad `Interactive` del scheduler (quantum de 2 ms, preempción prioritaria):

1. **Gestión de Superficies:**
   - Mantiene el árbol de ventanas, orden Z, decoraciones, sombras y foco.
   - Recibe los handles de VMO de cada cliente sin necesidad de copiar píxeles a través de IPC.
2. **Motor de Mezclado Híbrido:**
   - **Vía Acelerada:** Utiliza la GPU para componer las capas en el backbuffer principal en VRAM si hay driver 3D disponible.
   - **Vía Software (Universal):** Mezcla las regiones sucias (*damage rects*) en CPU utilizando instrucciones vectoriales SIMD (AVX2 / SSE2) optimizadas con *write-combining* en RAM, garantizando 60 FPS en hardware estándar sin driver 3D propietario.
3. **Presentación Final:**
   - Invoca `sys_display_present` para intercambiar el buffer visible en el siguiente VBlank.

---

## 6. Shell Gráfica: `lix-shell`

La shell está formada por procesos cliente estándar comunicados con el compositor:
- **Panel / Barra de Tareas:** Reloj, bandeja del sistema, lista de ventanas y estado de energía.
- **Lanzador de Aplicaciones:** Búsqueda y ejecución en Jobs dedicados.
- **Notificaciones y Fondo de Pantalla:** Superficies gráficas independientes.
- Diseño visual, tipografía y paleta definidos en [ui-design.md](ui-design.md).

---

## 7. Objetivos de Rendimiento y Estabilidad

- **Latencia Input-to-Photon:** $\le 16\text{ ms}$ (un frame a 60 Hz).
- **Aislamiento de pánico:** si una aplicación gráfica o el compositor agota memoria o causa un fallo de segmentación, **el kernel no se ve afectado**.
- **Consumo de CPU:** $\le 1\%$ de CPU del compositor en reposo (composición guiada puramente por eventos y damage rects).
- **Recuperación Automática:** Supervisión del compositor por `init`; si el proceso termina con error, `init` restaura TTY 1 y reintenta el arranque del compositor.

---

## 8. Implementación actual

Comportamiento observado en `kernel/src/drivers/` y en los crates de espacio de usuario. Los apartados siguientes describen hechos del código; no modifican la especificación de las secciones anteriores.

### 8.1 Superficie de salida

El único camino de dibujo en el kernel es `kernel/src/drivers/framebuffer/` sobre el framebuffer GOP de Limine:

- No existe el módulo `lix-drm` ni ningún otro módulo DRM/KMS.
- No hay detección de pantallas, modo, tasa de refresco, *page flipping*, VSync, VBlank, *damage rects* ni *direct scanout*.
- No hay asignación de VMOs gráficos ni buffers de scanout.
- El software de consola ([console.md](console.md)) es el único consumidor actual del framebuffer.

### 8.2 Entrada

| Elemento | Estado |
| --- | --- |
| Teclado PS/2 | IRQ1, puerto `0x60`, con cambio de disposición en caliente ([console.md](console.md) §2.3) |
| Ratón o puntero | no hay driver; no se lee el puerto `0x64` ni se habilita el canal auxiliar |
| `InputEvent` normalizado | no existe |
| `sys_input_open` (`0x705`) | declarado en `lix-sys`, sin handler en el dispatcher |

### 8.3 Syscalls de pantalla

Los números `0x700` (`sys_display_info`), `0x701` (`sys_display_acquire`), `0x702` (`sys_display_present`) y `0x705` (`sys_input_open`) están declarados en `libs/lix-sys/src/lib.rs` pero **no tienen handler** en `kernel/src/task/syscall.rs`. Los handlers presentes en el rango `0x07xx` son los de consola de texto: `0x703`, `0x704`, `0x706`, `0x707` (ver [syscall-abi.md](../spec/syscall-abi.md) §10).

### 8.4 Procesos gráficos

| Crate | Estado |
| --- | --- |
| `services/lix-compositor` | stub: una sola llamada a `println!` |
| `userland/lix-shell` | stub: una sola llamada a `println!` |
| `libs/lix-ui` | crate vacío (`src/lib.rs` con solo `#![no_std]`) |
| `libs/litui` | submódulo git real (`litui-core`, `litui-widgets`, `litui`, 72 archivos `.rs`); se usa como toolkit TUI de terminal en `userland/ltop`, no como toolkit gráfico |

No hay IPC de ventanas, ni protocolo cliente/compositor, ni canal de presentación.
