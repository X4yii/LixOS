# Documentación de LixOS

> **Tipo:** índice · **Estado:** vigente

Documentación técnica del sistema, organizada por tipo de documento. Cada archivo declara en su cabecera el tipo, el estado y las decisiones de arquitectura (ADR) de las que depende.

---

## 1. Estructura

| Directorio | Tipo | Contenido |
| --- | --- | --- |
| `spec/` | especificación | Contratos: ABI de syscalls, mapa de memoria, capabilities, portabilidad |
| `design/` | diseño | Descripción de subsistemas y de su implementación prevista |
| `adr/` | registro de decisiones | ADR-01 – ADR-26 con justificación y consecuencias |
| `guides/` | guía | Procedimientos operativos |

Estados en uso: `borrador` (sujeto a cambio), `consolidado` (revisado y estable) y `vigente`.

---

## 2. Especificaciones (`spec/`)

| Documento | Cubre |
| --- | --- |
| [syscall-abi.md](spec/syscall-abi.md) | Convención de llamada, códigos de error, tabla de las 35 syscalls (regenerada desde `abi/syscalls.toml`), alias de Linux e IDs reservados sin implementar |
| [memory-map.md](spec/memory-map.md) | Reparto del espacio virtual de 48 bits, reglas de paginación y seguridad |
| [capabilities.md](spec/capabilities.md) | Handles opacos, derechos atenuables, namespaces compuestos y revocación; qué existe hoy (`Process.rights`, enforcement del dispatcher, prefijos del VFS) y qué está reservado |
| [libc-compat.md](spec/libc-compat.md) | Capa de compatibilidad `sys-libc`: constantes, *layouts* y APIs emuladas |
| [porting-compat.md](spec/porting-compat.md) | Niveles de compatibilidad T0–T3, exclusiones y criterios de medición |

---

## 3. Diseño (`design/`)

| Documento | Cubre |
| --- | --- |
| [boot.md](design/boot.md) | Protocolo Limine, orden de inicialización, initrd y salto al espacio de usuario |
| [storage.md](design/storage.md) | Barrido PCI, controladora AHCI, abstracción de bloque, VFS y ext4 |
| [console.md](design/console.md) | Consola sobre framebuffer, entrada PS/2 y serie, registro `klog`, pánico y consola interactiva |
| [memory-management.md](design/memory-management.md) | Asignación del kernel, contabilidad estricta, presión de memoria y verificación de fugas |
| [process-scheduler.md](design/process-scheduler.md) | Modelo de ejecución, ciclo de vida, límites y política de planificación |
| [graphics.md](design/graphics.md) | Capas gráficas: módulo DRM/KMS, micro-compositor y shell gráfica |
| [ui-design.md](design/ui-design.md) | Identidad visual, paleta, tipografía, composición del escritorio y framework de aplicaciones |
| [installer.md](design/installer.md) | LiveISO, máquina de estados del instalador y herramientas de generación de imágenes |
| [connectivity.md](design/connectivity.md) | USB/xHCI, red, Wi‑Fi y Bluetooth: arquitectura, estado y etapas de soporte |
| [network-managers.md](design/network-managers.md) | Estados y límites de `netmgr` y `bluetoothd` |
| [security-auth.md](design/security-auth.md) | Identidad, autenticación, bóveda y elevación por capabilities |

---

## 4. Registro de decisiones (`adr/`)

| Documento | Cubre |
| --- | --- |
| [decisions.md](adr/decisions.md) | ADR-01 – ADR-26: decisión, justificación y consecuencias de cada una, más la tabla de decisiones diferidas con su criterio de resolución |

---

## 5. Guías (`guides/`)

| Documento | Cubre |
| --- | --- |
| [build.md](guides/build.md) | Workspace (21 miembros), estructura de módulos del kernel, toolchain, tareas de `xtask`, composición de imágenes y requisitos del host |
| [testing.md](guides/testing.md) | Categorías de prueba, dónde viven las pruebas hoy y el pipeline de CI de `.github/workflows/ci.yml` (7 jobs) |
| [porting-guide.md](guides/porting-guide.md) | Estructura y flujo de trabajo del directorio `ports/` |

---

## 6. Documentos de gobierno (raíz del repositorio)

| Documento | Cubre |
| --- | --- |
| [README](../README.md) | Qué es LixOS, tabla de estado honesta, build, estructura y licencia |
| [LICENSE](../LICENSE) | GNU General Public License versión 2 (texto íntegro) |
| [COPYING.EXCEPTIONS](../COPYING.EXCEPTIONS) | Excepción de syscalls y excepción de enlace de la libc; SPDX pendiente (ADR-16) |
| [TRADEMARK.md](../TRADEMARK.md) | Política de marca: *forks* permitidos, nombre reservado, sin distros oficiales |

---

## 7. Convenciones

### 7.1 Cabecera

Todo documento abre con `# Título` y un bloque de cita:

```markdown
> **Tipo:** especificación · **Estado:** borrador · **ADR:** 03, 05
> **Relacionado:** [Capabilities](capabilities.md)
```

### 7.2 Sincronización con el código

Se distinguen tres patrones según el origen del documento:

- **Documentos escritos a partir del código** (`design/boot.md`, `design/storage.md`, `design/console.md`, `guides/build.md`): su cuerpo describe directamente lo que el árbol implementa y cada dato cita su ruta.
- **Documentos de diseño** (el resto): el cuerpo describe el objetivo y el diseño de referencia, y los hechos observados en el código se añaden en una sección final `## Implementación actual`, con subsecciones numeradas y cita de ruta (`kernel/src/…`, `libs/…`, `xtask/…`, `userland/…`).
- **Tablas derivadas de una fuente única**: la tabla de syscalls de `spec/syscall-abi.md` se construye desde `abi/syscalls.toml`, la única fuente de verdad (ADR-24). Al cambiar el TOML hay que regenerar los artefactos con `cargo xtask codegen` y actualizar la tabla a mano; el job `codegen-drift` del CI cubre los artefactos, no este documento.

Las secciones `Implementación actual` no sustituyen a la especificación que las preceden: cuando el código y la especificación difieren, la divergencia queda documentada en ambos lugares para que sea visible.

### 7.3 Enlaces

Los enlaces son relativos al documento que los contiene. Las rutas de código citadas son relativas a la raíz del repositorio.

### 7.4 Alcance

La documentación cubre el estado del sistema y el diseño de referencia. Las decisiones de arquitectura se registran en `adr/decisions.md`; las cuestiones sin resolver se documentan en las secciones «Decisiones abiertas» de cada diseño, sin fecha asociada.
