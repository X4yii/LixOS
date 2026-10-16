# Registro de decisiones (ADR)

> **Tipo:** registro de decisiones · **Estado:** vigente · **Alcance:** ADR-01 – ADR-25
> **Relacionado:** índice de ADR citados desde el resto de la documentación

Cada ADR documenta una decisión de arquitectura, su justificación y sus consecuencias. Las decisiones con carácter técnico detallado se desarrollan en el documento correspondiente y aquí se resume.

Prioridades, en orden: **1) estabilidad, 2) rendimiento bruto, 3) simplicidad, 4) funcionalidades.**
Regla general: si una decisión baja la estabilidad para ganar rendimiento sin una medición que lo respalde, se elige la opción estable. Después se optimiza con datos.

Estado: `ACEPTADA` = cerrada para v1.0. `DIFERIDA` = se decide con datos (ver «Decisiones diferidas» al final).

---

## ADR-01: Kernel monolítico con drivers en Ring 0 (ACEPTADA)

**Decisión:** El kernel es monolítico. Los drivers de arranque crítico (AHCI, NVMe, framebuffer, PS/2, xHCI-HID, timer, APIC) corren en Ring 0. No hay módulos de terceros en v1.0.

**Justificación:**
- Es la opción más rápida: no hay IPC ni cambios de contexto entre kernel y driver.
- Es la más simple. Un microkernel o drivers en userspace exigen IPC, scheduling y capabilities maduros antes de tener un disco funcionando.
- La seguridad de memoria de Rust cubre buena parte del riesgo de estabilidad que tiene un monolítico en C.
- `.kmod` de terceros en Ring 0 anula el modelo de capabilities, y Rust no tiene ABI estable (ver ADR-11).

**Consecuencias:** El documento actual lo llama "híbrido". Se renombra a **monolítico asíncrono**. Los drivers de terceros pasarán a userspace en una versión futura (ADR-11).

---

## ADR-02: Dos capas de ejecución: threads preemptivos + executor async interno (ACEPTADA)

**Decisión:**
- **Capa A (procesos de usuario):** threads preemptivos con runqueues **por CPU** y time slices. Se usa el APIC timer.
- **Capa B (interno del kernel):** executor `async` cooperativo, **solo** para I/O de drivers y filesystem.
- **Work-stealing:** solo entre runqueues de Capa A, y solo en configuraciones SMP. En un solo núcleo no se aplica.
- Las tareas de Capa B son `Send` y no guardan estado per-CPU entre `.await`.

**Justificación:**
- Un `Future` no se puede preemptar. Un proceso de usuario en bucle infinito congelaría el sistema, y eso es inaceptable para la estabilidad.
- El async es muy bueno para esperar I/O de disco sin bloquear un thread del kernel. Ahí queda confinado.
- Empezar con un solo núcleo elimina una clase entera de bugs (carreras, TLB shootdown, IPI) mientras se construye lo básico.

**Consecuencias:** El "M:N de tasks de espacio de usuario" del diseño original se elimina. Un runtime M:N en userspace podría construirse después sobre threads.

---

## ADR-03: Syscalls síncronas directas en v1.0; rings de I/O después (ACEPTADA)

**Decisión:**
- v1.0 usa `SYSCALL`/`SYSRET` con convención de registros (nº de syscall en `rax`, argumentos en `rdi, rsi, rdx, r10, r8, r9`).
- Cada syscall valida punteros y **copia** datos entre userspace y kernel (`copy_from_user` / `copy_to_user`).
- El ring SQ/CQ (estilo io_uring) se diseña en un documento aparte y se implementa **solo después** de medir cuánto cuestan las syscalls directas (ADR-DIFERIDA-A).
- `clock_gettime` y similares se resuelven con una página vDSO de solo lectura. Es opcional y se habilita por medición.

**Justificación:**
- Una syscall directa es predecible, fácil de depurar y fácil de fuzzear.
- Los rings compartidos zero-copy sufren TOCTOU, pinning de páginas y cancelación. Son la mayor fuente de CVEs de io_uring en Linux.
- Una syscall `SYSCALL` cuesta unos 100 ns. El beneficio del ring aparece solo con muchísimo I/O por segundo, volumen no alcanzable en hardware modesto.

**Consecuencias:** Se reescribe la sección 7. `SyscallRingCommand` se mueve a un apéndice "futuro".

---

## ADR-04: Procesos sin `fork`; creación con `spawn` (ACEPTADA)

**Decisión:** La creación de procesos es `proc_spawn(path, args, env, handles_heredados)`. No existe `fork()` en el kernel. `sys-libc` ofrece `posix_spawn`.

**Justificación:**
- `fork` + copy-on-write complica la gestión de memoria, y casi siempre va seguido de `exec`.
- `spawn` encaja de forma natural con capabilities, porque el padre elige explícitamente qué handles hereda el hijo.

**Consecuencias:** El kernel no tiene `fork`, pero `sys-libc` ofrece `posix_spawn` y `vfork` (hijo que comparte el espacio de direcciones hasta `exec`). Un `fork` completo con copy-on-write se **evalúa** según los resultados de `compat/` (ver ADR-22).

---

## ADR-05: Capabilities: handles con índice + generación y derechos atenuables (ACEPTADA)

**Decisión:**
- `HandleID` = 64 bits: 32 de índice en la tabla del proceso + 32 de generación.
- Cada entrada guarda `(objeto_kernel, máscara_de_derechos)`.
- Operaciones: `handle_dup(h, derechos_menores)`, `handle_close(h)`, herencia en `spawn`. Los derechos solo pueden **reducirse**, nunca ampliarse.
- No hay autoridad ambiente: un proceso no puede abrir un archivo por ruta sin un handle de directorio que se lo permita.
- Los archivos ext4 conservan `owner/mode` en disco, pero el kernel los **trata como metadatos**. El control de acceso real son las capabilities del handle de directorio raíz que recibe cada sesión.
- La revocación de un handle compartido se hace con un objeto intermedio (`revoker`). Se define en el documento de capabilities.

**Justificación:**
- La generación detecta use-after-free de handles y evita que se reutilicen índices viejos.
- Atenuar y no ampliar es el principio que hace al modelo seguro de verdad.
- Mantener `mode/owner` evita que los discos ext4 dejen de ser compatibles con otras herramientas.

**Consecuencias:** `/etc/passwords` ya no es "legible solo por el kernel". Su protección es no entregar jamás un handle a esa ruta, salvo al servicio de autenticación.

**Namespace por proceso:** no hay una raíz global ambiente. Cada proceso hereda un **handle de directorio raíz** y la libc resuelve `open("/ruta")` como `openat(raiz, "ruta")`. Para el usuario normal la raíz abarca su vista habitual del sistema, y un proceso confinado recibe una raíz restringida. Así el código POSIX funciona sin cambios y el modelo de capabilities se mantiene (ADR-22).

---

## ADR-06: `sys-libc` propia desde cero, en capas, con compatibilidad creciente (ACEPTADA, revisada)

**Decisión:**
- `sys-libc` se escribe **desde cero**, en Rust `no_std` exportando ABI C, más cabeceras C propias. No se parte de musl ni relibc.
- Se construye en **dos capas**:
  1. `lix-sys`: crate Rust con la API **nativa** de Lix (syscalls, handles). Es la capa estable que usa el propio sistema (Vanish, PML, authd).
  2. `sys-libc`: capa de compatibilidad C/POSIX **encima** de `lix-sys`. Puede crecer o cambiar sin tocar el kernel.
- v1.0 soporta **solo ELF64 enlazado estáticamente**. Sin `.so` ni linker dinámico.
- Para acercarse a "cambios mínimos o nulos" con el tiempo: **se copian las convenciones de Linux x86_64 donde no cuesten nada**: valores de `errno`, números de `O_*`, `SEEK_*`, `S_IF*`, layout de `struct stat` y `struct timespec`, `sizeof` de los tipos. Eso evita que el código existente rompa por constantes distintas.
- Compatibilidad **medible**: un directorio `compat/` con programas C reales (`lua`, `sqlite`, `zlib`, `busybox`-subset) y una suite de tests (estilo `libc-test`). El porcentaje que compila sin parches es el indicador de progreso.
- Lo que se promete por nivel de compatibilidad (ver ADR-17 y ADR-22): T0 = C básico; T1 = POSIX de línea de comandos, señales básicas, `poll`/`select`, `epoll` emulado; T2 = hilos, `mmap` de archivo, PTY. `fork` real queda fuera de T2 y se evalúa por separado.
- Target de toolchain `x86_64-lix` con ABI System V, con parches mínimos a LLVM/clang para registrar el target.

**Justificación:**
- Es tu decisión y es válida: control total del ABI, sin heredar licencias ni supuestos de Linux, y coherente con el modelo de capabilities.
- Es el componente de mayor riesgo de tiempo, así que se reduce con la separación en capas: el sistema propio nunca depende de la libc C, solo de `lix-sys`. Si la libc se retrasa, el SO sigue avanzando.
- Copiar constantes y layouts de Linux es la forma más barata de lograr compatibilidad de código fuente sin copiar código.
- Lo estático hace los binarios autocontenidos y más rápidos al arrancar.

**Riesgos aceptados:** `printf`, `malloc` y `stdio` correctos y rápidos llevan meses; `pthread` y TLS dependen de decisiones del kernel (ADR-02). Mitigación: tests contra programas reales desde las primeras versiones, y priorizar el comportamiento por encima de cubrir toda la API.

**Licencia:** ver ADR-16. La libc **no puede ser GPLv2 pura**.

---

## ADR-07: Solo UEFI, abstracción BootInfo y bootloader agnóstico (ACEPTADA)

**Decisión:**
- v1.0 soporta **solo UEFI x86_64**. BIOS legacy/MBR se elimina de la lista de requisitos.
- El arranque usa **Limine** únicamente como shim provisional de carga. Un **`LixBoot` propio** es el cargador nativo oficial proyectado (ADR-18, Hito M8).
- El kernel es **estrictamente agnóstico al bootloader**: la comunicación se limita a un contrato de datos plano y genérico `BootInfo` (HHDM offset, mapa de memoria, GOP framebuffer, initrd y dirección física de ACPI RSDP).
- **Prohibición de protocolos SMP propietarios:** Se prohíbe delegar el multiprocesamiento a Limine (`limine::MpRequest`). El kernel asume la responsabilidad total de enumerar las CPUs vía hardware (ACPI MADT) y despertarlas mediante la secuencia x86 INIT-SIPI-SIPI.
- Multiboot de Linux y Windows se hace en LixBoot, no antes.

**Justificación:**
- UEFI da framebuffer GOP, mapa de memoria y ACPI sin trabajo extra.
- Aislar el cargador detrás de `BootInfo` y asumir el control nativo del hardware multiprocesador garantiza que sustituir Limine por `LixBoot` en el futuro requiera cero modificaciones en el kernel y en el planificador.
- Prioriza estabilidad y simplicidad: el kernel no depende de supuestos ni estados opacos de cargadores externos.

**Consecuencias:** "Revivir equipos antiguos" se redefine como "equipos x86_64 con UEFI (~2013+)".


---

## ADR-08: VFS extensible desde el día uno; ext4 es el único FS de usuario (ACEPTADA, revisada)

**Decisión:**
- Se crea una capa **VFS** basada en un trait `Filesystem` (montar, lookup, read, write, readdir, sync), diseñada para admitir más sistemas de archivos en el futuro **sin cambiar el kernel**.
- **ext4 es el sistema de archivos por defecto y el único soportado para datos de usuario.** Los demás se añaden solo si hay una necesidad real.
- Backends **internos** que no cuentan como "FS soportado": `tmpfs`, `devfs` y un **FAT32 mínimo** (solo para escribir la partición ESP en el instalador y actualizar LixBoot). Sin FAT32 el instalador no puede dejar el equipo arrancable.
- El LiveISO **no necesita driver ISO9660 en el kernel**: el bootloader UEFI lee `lix-root.img` desde el ISO y lo entrega en RAM (ver ADR-18).
- Orden de ext4: **solo lectura** en la primera etapa → **escritura con journaling** cuando el driver pase fuzzing y tests de corte de energía.
- Subconjunto ext4 soportado: `extents`, `flex_bg`, `64bit`, `metadata_csum`, `dir_index`. Todo lo demás se **rechaza al montar**.
- Hasta que el driver escribiendo pase fuzzing y tests de corte de energía, el sistema raíz se monta **solo lectura**, y las escrituras van a `tmpfs` o a una partición separada.

**Justificación:**
- El LiveISO ya necesita dos FS (tmpfs más medio de solo lectura), así que el VFS es obligatorio.
- Corromper datos es el peor fallo posible. Rechazar lo que no se entiende es más seguro que adivinar.
- Escribir ext4 sin un `journaling` replay correcto es la operación con mayor riesgo de corrupción de datos del sistema.

---

## ADR-09: Memoria: LZ4 por defecto, ZRAM y contabilidad estricta de commit (ACEPTADA, revisada)

**Decisión:**
- Asignador físico (bitmap o buddy), paginación de 4 niveles, heap del kernel (slab). Huge pages de 2 MB solo para el mapeo directo del kernel.
- **Contabilidad Estricta de Commit (*Strict Commit Charge Accounting*):** Las reservas `sys_vm_map` debitan de un contador global (`committed_bytes`). La paginación física es bajo demanda en `#PF`, pero `sys_vm_map` falla de inmediato con `ENOMEM` si `committed_bytes + size > RAM_física + ZRAM_max`. Cero overcommit impredecible.
- **ZRAM con LZ4 únicamente.** Pool ZRAM con límite del **25% de la RAM**.
- **Inmunidad a Deadlocks en ZRAM:** Buffers de compresión estáticos por CPU de 64 KB preasignados en el arranque (cero alocaciones dinámicas durante la compresión) y *Emergency Reserve Pool* en el Buddy Allocator para operaciones del kernel cuando la memoria física cruza la marca `min`.
- **No hay swap a disco en v1.0.** Si se agota ZRAM, actúa un OOM killer determinista.

**Justificación:**
- Paginación por demanda sin contabilidad previa es un overcommit implícito que causa OOMs aleatorios en `#PF`. La contabilidad estricta garantiza estabilidad matemática.
- Los buffers estáticos y la reserva de emergencia impiden que ZRAM se bloquee a sí mismo intentando alocar memoria en situaciones críticas.

---

## ADR-10: Autenticación: Argon2id en un servicio de userspace (ACEPTADA)

**Decisión:** El kernel **no** calcula hashes. Un servicio `authd` en userspace, con las capabilities mínimas, verifica contraseñas con Argon2id. Parámetros iniciales: `m=19 MiB, t=2, p=1`, ajustables por hardware. `vdo` pide a `authd` un handle elevado con tiempo de vida limitado.

**Justificación:** Un cálculo de cientos de ms dentro del kernel bloquea el sistema. En userspace es preemptable y su fallo no tumba el kernel.

---

## ADR-11: Módulos de terceros: fuera de v1.0 (ACEPTADA)

**Decisión:** v1.0 **no** carga `.kmod`. Cuando se necesite soporte de terceros, serán **drivers de userspace** con capabilities de MMIO, DMA e IRQ, no módulos en Ring 0.

**Justificación:** Rust no tiene ABI estable y un módulo en Ring 0 puede romper cualquier garantía. Un driver de userspace es coherente con el modelo de capabilities.

---

## ADR-12: Sin red ni `pml update` remoto en v1.0 (ACEPTADA)

**Decisión:** v1.0 no incluye stack de red. PML instala desde archivos `.pml` locales (USB, disco). La red va en v1.1+ con virtio-net primero.

**Justificación:** Es un subsistema enorme (drivers, TCP, DNS, TLS) y no es necesario para validar el núcleo.

---

## ADR-13: Observabilidad: serial, klog y panic con frame pointers (ACEPTADA)

**Decisión:**
- Puerto serie 16550 y `klog` con niveles desde el primer arranque.
- Se compila con `-C force-frame-pointers=yes` para backtraces.
- El `panic_handler` **no asigna memoria**, escribe primero a serial y después a pantalla, y detiene los demás núcleos con NMI/IPI cuando exista SMP.

**Justificación:** Sin logs no se puede depurar un kernel. La panic screen del diseño se conserva como interfaz, pero la fuente de verdad es el serial.

---

## ADR-14: Objetivos medibles (SLO) y política de `unsafe` (ACEPTADA)

**SLO iniciales (se ajustan tras la primera medición):**

| Métrica | Objetivo |
| --- | --- |
| Arranque hasta prompt (QEMU) | ≤ 1 s |
| Arranque hasta prompt (HDD real) | ≤ 8 s |
| RAM del kernel en reposo | ≤ 64 MB |
| Syscall trivial (`getpid`) | ≤ 300 ns |
| Latencia tecla → pantalla | ≤ 16 ms |
| Cambio de contexto | ≤ 2 µs |

**Política de `unsafe`:**
- Se concentra en módulos pequeños y revisados (`arch/`, `mm/`, `drivers/mmio`). Cada bloque lleva un comentario `// SAFETY:`.
- Lo que sea lógica pura (parsers, estructuras, capabilities) va en crates `no_std` sin `unsafe` y se testea en el host con `cargo test` y Miri.
- Parser ELF, ext4 y PML pasan por **fuzzing** (`cargo-fuzz`) antes de darse por terminados.

---

## ADR-15: Gráficos: micro-compositor en userspace con buffers VMO zero-copy (CONSOLIDADA CON ADR-25)

**Decisión:**
- El kernel provee un módulo **`lix-drm`** propio (ModeSetting, asignación de VMOs y sincronización Vsync).
- La **composición y gestión de ventanas** (`lix-compositor`) reside en **userspace** en la clase de prioridad `Interactive`. Las aplicaciones comparten buffers gráficos mediante Handles de VMO con política Zero-Copy.
- **Direct Scanout:** Optimización en Ring 0 para ventanas a pantalla completa.
- **Fallback Seguro:** Si no hay aceleración GPU o falla el driver 3D, el compositor mezcla en CPU mediante SIMD (AVX2/SSE2) y *damage tracking*, con la consola TTY de texto siempre disponible en el kernel como red de seguridad.

**Justificación:**
- Colocar la composición en Ring 0 viola la estabilidad y depende de planos de hardware que son escasos en hardware real (<4 overlays).
- Mantener el compositor en userspace garantiza aislamiento total de fallos: si la GUI crashea, el kernel no muere y `init` restaura el servicio o conmuta a TTY 1.

---

## ADR-16: Licencia y modelo de distribución (ACEPTADA, con advertencias)

**Decisión:**
- Kernel, shell, utilidades y el resto del sistema base: **GPLv2**.
- **`sys-libc`, `lix-sys` y las cabeceras**: **GPLv2 con excepción de enlace** (estilo *GCC Runtime Library Exception*) o **LGPLv2 + enlace estático permitido**. Se decide cuál antes del primer release público.
- Se añade una **excepción explícita de syscalls** (como la de Linux): los programas de usuario que solo usan la interfaz de syscalls no quedan sujetos a GPL.
- El proyecto es **de desarrollo cerrado en control** (un único mantenedor decide diseño, componentes y ABI), pero con **código abierto**. No hay "distros" oficiales: LixOS es un producto integrado con un conjunto fijo de componentes. Marca y nombre quedan reservados.

**Justificación y advertencias:**
1. **Enlace estático + libc GPLv2 pura = todo programa compilado para Lix sería GPLv2.** Como v1.0 solo enlaza estático (ADR-06), esto es un problema real. Por eso la libc necesita una excepción; es el punto central de este ADR.
2. GPLv2 incluye la cláusula **anti-tivoización** (sección 6): si exiges firmas criptográficas para módulos o paquetes (el diseño original lo pide), el **dueño del equipo debe poder instalar su propia clave o desactivar la verificación** en dispositivos de consumo. En un PC es fácil de cumplir: la clave raíz de confianza debe ser configurable por el usuario.
3. GPLv2 no impide un modelo "cerrado como Windows" en el sentido de **control del diseño**, pero **sí impide** impedir que otros hagan forks. No se puede prohibir redistribuir ni modificar. Lo que sí se puede controlar es la **marca**.
4. Los drivers propietarios de terceros en Ring 0 son legalmente conflictivos con GPLv2. Esto refuerza el ADR-11: los drivers de terceros serán de **userspace**, que no tienen ese problema.
5. Esto no es asesoría legal. Antes del primer release conviene una revisión de un abogado.

**Consecuencias:** cada crate lleva `SPDX-License-Identifier` en cada archivo. Se añade `LICENSE`, `COPYING.EXCEPTIONS` y una política de marca (`TRADEMARK.md`).

---

## ADR-17: Versionado pre-1.0 (ACEPTADA)

**Decisión:** Versionado `MAYOR.MENOR.PARCHE`. Antes de 1.0, **no hay garantía de compatibilidad de ABI** entre versiones `0.x`.

- `0.0.x`: prototipo del kernel, sin userspace. Cada `0.0.N` es un incremento pequeño y verificable.
- `0.MENOR.0`: incremento de funcionalidad con cambios de ABI.
- `0.MENOR.PARCHE`: correcciones y mejoras sin cambio de ABI.
- `0.9.x`: candidatos a estable (solo corrección de errores y estabilidad, sin funciones nuevas).
- `1.0.0`: estable. A partir de aquí el **ABI de syscalls y el formato `.pml` se congelan** y los cambios incompatibles requieren `2.0`.

Además:
- El **ABI de syscalls tiene su propio número de versión**, expuesto por el kernel y verificado por `lix-sys`.
- Cada versión se etiqueta en git (`v0.1.0`), con un `CHANGELOG.md`.
- La versión se muestra en LixBoot, en la panic screen y en el login.

**Justificación:** `0.0.x` da margen para muchos incrementos pequeños al inicio, sin quemar versiones. Congelar el ABI en `1.0` da un punto claro a partir del cual otros pueden programar para Lix.

---

## ADR-18: LiveISO: carga a RAM desde el bootloader (ACEPTADA)

**Decisión:**
- El ISO es **híbrido UEFI** (El Torito + ESP). LixBoot/Limine lee `lix-kernel.elf` y `lix-root.img` (Zstd) desde el medio y los carga a RAM. El kernel descomprime el rootfs en `tmpfs`.
- Una vez cargado, el medio **se puede retirar**.
- El kernel **no** necesita drivers de ISO9660 ni de USB para el arranque, porque la lectura la hace el firmware UEFI.
- Se limita el tamaño de `lix-root.img`: ≤ 25% de la RAM mínima (4 GB), es decir **≤ 1 GB descomprimido**, para dejar memoria al sistema y a ZRAM.
- `lix-installer` copia el rootfs a ext4, escribe la ESP (FAT32 mínimo, ADR-08) e instala el bootloader.

**Presupuesto de tamaño (el ISO debe ser pequeño):**
- Objetivo: **ISO ≤ 150 MB**, con **rootfs descomprimido ≤ 400 MB** (meta de v1.0). El tope duro de 1 GB queda solo como límite de seguridad.
- Rootfs comprimido con **Zstd nivel alto (19+) y diccionario**. Se paga solo al generar el ISO. Descomprimir es rápido.
- Binarios con `strip`, `opt-level="s"` o `"z"` en lo que no es crítico para el rendimiento, y LTO. Enlace estático con `--gc-sections` para que solo entre lo usado.
- Contenido mínimo en el LiveISO: kernel, Vanish, PML, `lix-installer`, `authd` y utilidades base. Nada de documentación extensa, locales extra ni fuentes que no se usen.
- El CI **falla** si el ISO supera el presupuesto, igual que un benchmark que regresiona.
- El enlace estático (ADR-06) duplica código común entre binarios. Si el tamaño se vuelve un problema, se evalúa un binario único tipo *multicall* (estilo BusyBox) para las utilidades base.
- Diseño completo del ISO y del instalador en [installer.md](../design/installer.md).

**Justificación:** es el mismo modelo que Linux (*copytoram*) y evita el trabajo de drivers de almacenamiento extraíble para poder arrancar. Reduce mucho la superficie de código del primer arranque.

---

## ADR-19: Disciplina de memoria: RAII, asignaciones falibles y verificación de fugas (ACEPTADA)

Diseño completo en [memory-management.md](../design/memory-management.md).

**Decisión:**
- Todo recurso es un tipo con `Drop`. Se prohíben `mem::forget` y `Box::leak` fuera del arranque.
- **Toda asignación del kernel es falible** (`Err(NoMem)`). El kernel no hace `panic` por falta de memoria fuera del arranque. Se usan colecciones propias con API `try_*`.
- Asignador de páginas **buddy** con cachés por CPU, heap **slab** con desperdicio interno ≤ 12,5%, paginación por demanda con zero-page compartida, W^X, NX, SMEP y SMAP.
- Orden de recuperación bajo presión: page cache limpia → slabs libres → writeback → ZRAM → OOM killer determinista.
- **Verificación obligatoria en CI:** línea base de frames y slabs tras cada test, ciclo de 10.000 procesos sin variación, asignador en modo depuración (poisoning, redzones, cuarentena), fuzzing y soak de 24 h en hardware real.

**Justificación:** Rust evita errores de memoria a nivel de lenguaje, pero no evita fugas lógicas ni agotamientos. La falta de memoria es un caso normal que el kernel debe poder manejar, no un fallo fatal. Sin verificación automática, estas reglas se erosionan con el tiempo.

---

## ADR-20: Procesos con Jobs, sin zombies; MLFQ O(1) y preempción voluntaria condicional (ACEPTADA, revisada)

Diseño completo en [process-scheduler.md](../design/process-scheduler.md).

**Decisión:**
- Jerarquía **Job → Process → Thread**. Todo proceso pertenece a un Job, y terminar un Job termina todo su árbol.
- El estado de salida vive en el objeto `Process`, que existe mientras alguien tenga un handle. **No hay tabla de PIDs ni zombies que recoger.**
- Secuencia de terminación **ordenada y obligatoria**: parar hilos → cancelar I/O en vuelo → cerrar handles → desmapear memoria → devolver cuota → notificar → liberar stacks por el *reaper*.
- Límites por defecto de memoria, hilos y handles en procesos y Jobs.
- **Scheduler MLFQ de 32 niveles con colas por CPU (SMP):**
  - Arreglo estático de runqueues por CPU (`RUNQUEUES: [IrqSafeSpinlock<PerCpuRunqueue>; MAX_CPUS]`) con selección $O(1)$ (`31 - active_bitmap.leading_zeros()`).
  - **Cero contención en camino rápido:** Cada núcleo adquiere exclusivamente su candado local al planificar o ceder tiempo; los demás núcleos ejecutan sin interferencias.
  - **Cero alocaciones de memoria en ruta de planificación:** El cambio de contexto no invoca al heap (`no alloc`).
  - **Descubrimiento y arranque nativo de APs:** Enumeración hardware vía tabla ACPI MADT (`'APIC'`) y arranque por secuencia canónica INIT-SIPI-SIPI con trampolín de 16 bits en `0x8000`, sin delegar en protocolos de bootloader.
  - **Balanceo por Work-Stealing determinista:** Activado solo con runqueue local vacía; adquisición canónica de candados $\min(\text{cpu}_A, \text{cpu}_B) \to \max(\text{cpu}_A, \text{cpu}_B)$ que elimina matemáticamente los interbloqueos.
  - Quantum: 2 ms para `Interactive`, 4 ms para `Normal`, 8 ms para `Batch`, anti-inanición cada 1 s y herencia de prioridad.
- **Tickless idle**, FPU con `XSAVEOPT` eager, PCID cuando exista.
- **Preempción Voluntaria Condicionada (`PREEMPT_VOLUNTARY`):** Kernel no preemptible en su estructura general, pero con macros `cond_resched()` en bucles $O(N)$ (ext4, liberación masiva de VMAs/frames) que verifican el flag `need_resched`.

**Justificación:** Previene que una syscall pesada secuestre la CPU e impida atender al compositor o eventos de teclado, salvaguardando el SLO de latencia $\le 16\text{ ms}$ sin la complejidad de un kernel totalmente preemptible. La arquitectura multinúcleo prioriza estabilidad y eficiencia: el aislamiento de runqueues elimina la contención en el bus y el arranque nativo garantiza independencia absoluta ante el futuro cargador `LixBoot`.

---

## ADR-21: Instalador: transaccional, sin tocar datos ajenos, probado en CI (ACEPTADA)

Diseño completo en [installer.md](../design/installer.md).

**Decisión:**
- Dos modos en v1.0: **disco completo** y **junto a otro SO usando solo espacio libre**. **No se redimensionan particiones.** No hay particionado manual.
- La **entrada de arranque se crea al final**. Si algo falla antes, el equipo arranca como antes y el instalador limpia lo creado.
- Se reutiliza la ESP existente sin formatearla y no se modifica ningún cargador de otro SO.
- Verificación por manifiesto firmado con hashes, antes y después de copiar.
- Herramientas propias: `lix-part` (GPT), `lix-mkfs` (ext4 y FAT32 mínimo, validados con `e2fsck` y `fsck.fat`), `lix-bootctl`, `lix-mkiso`.
- **Instalación desatendida** (`install.toml`) para poder probar todo en CI con QEMU, incluyendo cortes de energía simulados en cada fase.

**Justificación:** el instalador es el único componente que puede destruir datos de un usuario. Por eso se limita deliberadamente el alcance (sin resize), se ordena el trabajo para que lo irreversible vaya al final y se exige cobertura automática de fallos antes de publicarlo.

---

## ADR-22: Portabilidad de código fuente como objetivo de 1.0 (ACEPTADA)

Diseño completo en [portabilidad de software](../spec/porting-compat.md).

**Decisión:**
- La meta es **compatibilidad de código fuente**, no de binarios. No habrá capa de compatibilidad de binarios de Linux.
- La compatibilidad vive en **`sys-libc`**, que traduce POSIX al modelo nativo. El kernel conserva su diseño y solo incorpora primitivas mínimas (`wait_many`, notificaciones asíncronas para señales, raíz por proceso).
- Se copian de Linux x86_64 las **convenciones** (LP64, `errno`, constantes, layouts, FHS). **No se define `__linux__`**; sí `__lix__` y `__unix__`.
- **Niveles T0 a T3.** `1.0` garantiza **T2**: C estándar, POSIX CLI, hilos, señales, `mmap` de archivo, PTY y `AF_UNIX`. T3 (`fork` real, `dlopen`, red) llega en `1.x`.
- **Se revisan decisiones previas:** `vfork`/`posix_spawn`, `poll`/`select` y `termios` en T1; subconjunto de señales y `epoll` emulado en T2; `ET_DYN`/PIE desde el inicio; shell `sh` POSIX incluida.
- **El ABI se congela en `1.0`** (ADR-17), y el `sysroot` se versiona.
- Se mide con `compat/` y `ports/`: T0 y T1 con **0 parches**, T2 con mediana **≤ 20 líneas**. Un parche repetido en 3 ports se convierte en corrección de la libc.

**Justificación:** deformar el kernel para imitar a Linux sacrificaría la estabilidad y el modelo de seguridad que son el objetivo. Concentrar la traducción en la libc aísla el riesgo, deja el kernel pequeño y permite mejorar la compatibilidad después sin cambiar el ABI. Copiar convenciones de Linux es barato y evita la mayoría de las roturas por constantes distintas.

**Consecuencias:** más superficie que mantener en `sys-libc` (ver el detalle en el documento) y un mantenimiento continuo de parches de toolchain (LLVM, `config.sub`, `std` de Rust).

---

## ADR-23: Namespaces compuestos, privilegios sin `root` y revocación real de memoria (ACEPTADA, revisada)

Diseño completo en [capabilities](../spec/capabilities.md).

**Decisión:**
- Cada proceso tiene un **namespace compuesto**: un directorio virtual con enlaces (`/bin`, `/home/ana`, ...) y una máscara de derechos por enlace. Lo que no está enlazado no existe para el proceso.
- Se puede **derivar** un namespace más pequeño para confinar programas. Los derechos solo disminuyen.
- **No existe el usuario `root` ni `setuid`.** `init` recibe todas las capabilities raíz y las reparte a servicios mínimos (`authd`, sesión, compositor, energía, instalador).
- **`vdo`** pide a `authd` la elevación. `authd` crea el proceso hijo con namespace y handles ampliados, hechos con `dup_revocable`, y los **revoca** al vencer el plazo, sin matar al proceso.
- **Revocación Real (Syscalls + Memoria Virtual):** 
  - Un flag `AtomicBool` para invalidar llamadas a syscalls subsiguientes.
  - Para memoria mapeada (VMOs con derecho `MAP`), el kernel mantiene un *reverse mapping* hacia las VMAs. Al revocar, desmapea forzosamente las entradas de tabla de páginas (PTEs) y dispara un **TLB shootdown (IPI)** a todos los núcleos que ejecuten hilos de esos procesos.
- La **TCB** de seguridad es el kernel, `init` y `authd`, y se mantienen pequeños y con la máxima cobertura de pruebas.

**Justificación:** Evita que un proceso retenga acceso directo por hardware (MMU) a memoria concedida temporalmente una vez expirado el plazo de `vdo`.

**Consecuencias:** `/var/lib/auth` (hashes) vive fuera de cualquier namespace de usuario. El VFS necesita un backend de namespace (ver ADR-08).

---

## ADR-24: ABI definido en una única fuente, `abi/syscalls.toml` (ACEPTADA)

**Decisión:** Números, nombres, argumentos, tipos, derechos y errores de cada syscall se definen en **`abi/syscalls.toml`**. De ese archivo se generan: la tabla del kernel, `lix-sys` (Rust), `<lix/syscall.h>` (C), la documentación del ABI y el fuzzer de syscalls.

**Justificación:** el ABI se escribe a mano en cinco sitios en casi todos los SO, y es una fuente permanente de incoherencias. Con una fuente única, el ABI no puede divergir entre kernel, librerías y pruebas, y congelarlo en `1.0` se convierte en congelar un archivo.

---

## ADR-25: Aceleración gráfica desacoplada y fallback seguro (REVISADA Y CONSOLIDADA)

**Decisión:**
- LixOS ofrece soporte para aceleración gráfica por hardware delegando la gestión de modos, VMOs y Vsync al subsistema `lix-drm` en Ring 0.
- La **composición de ventanas** se ejecuta en userspace (`lix-compositor`), usando aceleración GPU si hay driver disponible o fallback vectorizado SIMD por CPU con damage tracking.
- Se descarta la composición obligatoria en Ring 0 mediante planos de hardware directos, pues los controladores físicos tienen un límite estricto de overlays (1 a 4). En su lugar, el kernel soporta *Direct Scanout* solo como optimización para ventanas completas sin ventanas superpuestas.
- La consola TTY de texto permanece en el kernel sobre GOP como red de seguridad incondicional.

**Justificación:**
- Preserva la jerarquía **Estabilidad > Rendimiento > Escalabilidad**: si el entorno gráfico o el driver 3D colapsan, el kernel no entra en pánico y la sesión se recupera sin pérdida de datos.
- Permite que el sistema operativo funcione en hardware virtualizado estándar (`virtio-gpu`) y equipos sin controladores 3D propietarios.

## ADR-26: Conectividad integrada por etapas (ACEPTADA)

**Decisión:** Se amplía el alcance de conectividad previsto en ADR-12 con una entrega progresiva: xHCI como primer bus USB; virtio-net como adaptador de red reproducible en QEMU; Bluetooth HID para teclado/ratón; y soporte Wi‑Fi por familias de chipsets con una matriz pública de modelos y firmware. La primera ruta de red usable debe incluir IPv4, ARP, DHCP, DNS, UDP y TCP.

**Arquitectura:** Los drivers incluidos siguen siendo internos al kernel monolítico (ADR-01). Los parsers y protocolos independientes del hardware se mantienen en crates `no_std` host-testables. Drivers externos en userspace siguen dependiendo de las capabilities de MMIO, DMA e IRQ de ADR-11, aún no implementadas.

**Justificación:** QEMU aporta una ruta repetible para validar controladores y protocolos, mientras la compatibilidad física puede crecer sin prometer un driver universal inexistente para Wi‑Fi.

**Consecuencias:** ADR-12 queda reemplazada en cuanto al alcance de conectividad. Audio Bluetooth y BLE GATT no forman parte del primer hito. La arquitectura y el estado observado se mantienen en [Conectividad](../design/connectivity.md).

---

## Decisiones diferidas

| ID | Tema | Criterio de decisión |
| --- | --- | --- |
| D-A | Ring SQ/CQ para I/O | Medición del coste de las syscalls directas (ADR-03, ADR-14) |
| D-B | Protocolo cliente/compositor y widgets (`design/graphics.md`, `design/ui-design.md`) | Definirse junto al protocolo de presentación |
| D-C | Drivers de GPU / aceleración 3D nativa | Disponibilidad de hardware de destino; el fallback por software es el camino garantizado (ADR-25) |
| D-D | Linker dinámico `ld-lix` y `dlopen` (el loader ya soporta PIE) | Requiere el ABI congelado y `compat/` con datos suficientes |
| D-E | Suspend / hibernate | Requiere ACPI y gestión de energía en el kernel |
| D-F | Cadena de confianza y Secure Boot (con clave configurable por el usuario, ver ADR-16) | Antes de PML con repositorios remotos |
| D-G | Elección final de licencia de la libc (excepción GPL o LGPL) | Antes del primer release público |
| D-H | `fork` real con copy-on-write | Según los resultados de `compat/` (ADR-22) |


---

## Estado de implementación de los ADR

Comportamiento observado en el árbol de fuentes. Las secciones siguientes describen hechos del código; no modifican la formulación de las decisiones anteriores. La referencia «§ *n*» apunta a la sección `Implementación actual` del documento indicado.

| ADR | Estado | Evidencia |
| --- | --- | --- |
| ADR-01 | parcial | drivers en `kernel/src/drivers/`, todo en Ring 0; xHCI implementa enumeración/HID boot y polling de conexión/desconexión en puertos raíz, aún sin validación ni hubs |
| ADR-02 | parcial | hay planificación y preempción; **no hay executor `async` ni tipos `Future` en el kernel** |
| ADR-03 | en uso | `syscall_dispatcher` y `init_syscall_msrs` (`kernel/src/task/syscall/mod.rs`); vía primaria `SYSCALL`, secundaria `int 0x80` ([syscall-abi.md](../spec/syscall-abi.md) §1) |
| ADR-04 | en uso | `sys_process_spawn` (`0x100`); no existe `fork` |
| ADR-05 | en uso | `HandleTable` con `slots` + `generations`, `Handle { index, generation }` y `Rights` (`kernel/src/task/handle.rs`) |
| ADR-06 | parcial | `libs/sys-libc` expone 18 envoltorios `sys_*`; sin API POSIX ni `errno` ([libc-compat.md](../spec/libc-compat.md) §5) |
| ADR-07 | en uso | peticiones Limine en `kernel/src/main.rs` ([boot.md](../design/boot.md) §1) |
| ADR-08 | parcial | `kernel/src/drivers/{nvme,gpt,block}.rs` + `libs/lix-storage`: NVMe/GPT 512 B y 4 KiB; ext4 por partición solo lectura + overlay RAM, sin journaling/writeback ([storage.md](../design/storage.md) §3–§6) |
| ADR-09 | sin implementar | no hay LZ4, ZRAM, contabilidad de commit ni OOM killer; sí hay contadores de uso de PMM y de E/S expuestos por `sys_monitor_metrics` ([memory-management.md](../design/memory-management.md) §9.5) |
| ADR-10 | parcial | Argon2id y cifrado AEAD están implementados como núcleo en `services/authd`; faltan RNG, persistencia, rate-limit, arranque y grants integrados ([security-auth.md](../design/security-auth.md)) |
| ADR-11 | en uso | no hay infraestructura de módulos en el árbol |
| ADR-12 / ADR-26 | parcial | QEMU virtio-net obtiene DHCP; xHCI aún falla en GET_DESCRIPTOR de teclado USB. `lix-wifi`/`lix-bluetooth` aportan parsers, pero faltan drivers de radio/HCI, supplicant, pairing, sockets e integración en servicios ([connectivity.md](../design/connectivity.md)) |
| ADR-13 | en uso | `kernel/src/klog/`, `kernel/src/panic.rs`, `force-frame-pointers=yes` en `KERNEL_RUSTFLAGS` ([console.md](../design/console.md) §3–§4) |
| ADR-14 | sin instrumentación | no hay medición de latencia de `SYSCALL` ni tests de `unsafe` |
| ADR-15 / ADR-25 | sin implementar | no existe el módulo `lix-drm` ni ningún DRM/KMS; `services/lix-compositor` sigue siendo un *stub* y los crates `userland/lix-shell` y `libs/lix-ui` fueron retirados del árbol ([graphics.md](../design/graphics.md) §8) |
| ADR-16 | decisión de política | no afecta al código |
| ADR-17 | parcial | `sys_abi_version` devuelve `0x00000001`; `abi/syscalls.toml` declara `0.1.0`; el workspace declara `0.0.1` |
| ADR-18 | en uso | `xtask iso` genera `target/lixos.iso` y el initrd se carga como módulo ([boot.md](../design/boot.md) §3) |
| ADR-19 | parcial | RAII presente (`AddressSpace::Drop` libera los marcos); sin línea base de frames ni tests de fugas ([memory-management.md](../design/memory-management.md) §9.5) |
| ADR-20 | en consolidación | MLFQ $O(1)$ de 32 prioridades con `PerCpuArea`, `GS_BASE`, colas por CPU y arquitectura multinúcleo SMP documentada ([process-scheduler.md](../design/process-scheduler.md) §8–§9) |
| ADR-21 | parcial | `lix-storage` comparte validación GPT; `lix-installer` inspecciona imágenes y calcula planes de solo lectura, pero no aplica cambios, no tiene diario/rollback, FAT32 ni registro UEFI ([installer.md](../design/installer.md) §9) |
| ADR-22 | sin implementar | sin `ports/`, sin `compat/`, sin toolchain `x86_64-lix` ([porting-compat.md](../spec/porting-compat.md) §11) |
| ADR-23 | parcial | no hay namespaces ni capabilities raíz/bootstrap; ELF queda forzado a Ring 3/IOPL 0. `sudo`, `vdo` y `su0` fallan cerrados mientras authd no implemente grants; ver `docs/design/security-auth.md` |
| ADR-24 | en uso | `abi/syscalls.toml` (35 entradas) es la fuente única; `cargo xtask codegen` genera `kernel/src/task/syscall_table.rs`, `include/lix/syscall.h` y `libs/lix-sys/src/syscalls.rs` (antes escrito a mano). La documentación del ABI y el fuzzer de la decisión siguen **sin** generarse: la tabla de [syscall-abi.md](../spec/syscall-abi.md) se mantiene a mano y debe regenerarse desde el TOML; el job `codegen-drift` de CI falla ante deriva |

### Observaciones sobre la §2 y la §10

- La fila «Enlace dinámico / `dlopen`» de [porting-compat.md](../spec/porting-compat.md) §2 afirma que el loader soporta `ET_DYN` y PIE desde el inicio. `kernel/src/task/elf.rs` rechaza `ET_DYN`; ver [memory-map.md](../spec/memory-map.md) §5.6.
- La decisión D-D repite «el loader ya soporta PIE» con la misma discrepancia.
- `adr/decisions.md` ADR-15 y ADR-25 describen un módulo `lix-drm` en Ring 0; ese módulo no existe. La convención de nombre usada en [graphics.md](../design/graphics.md) es `lix-drm` en todo el documento.
