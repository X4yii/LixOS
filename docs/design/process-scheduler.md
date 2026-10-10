# Procesos, hilos y scheduler

> **Tipo:** diseño · **Estado:** borrador · **ADR:** 20
> **Complementa:** ADR-02, 04, 05 · **Relacionado:** [Gestión de memoria](memory-management.md) · [Capabilities](../spec/capabilities.md)

Define el modelo de ejecución (Job → proceso → hilo), el ciclo de vida sin zombies, los límites por defecto y la política de planificación.

Objetivo: **que ningún proceso deje recursos sin liberar al terminar, que no se acumulen procesos zombie, y que el sistema responda de forma estable en hardware de gama baja.**

## 1. Entidades

```text
Job (grupo, cuota, política de OOM)
 └── Process (espacio de direcciones + tabla de handles + límites)
      └── Thread (stack de kernel, contexto, estado, prioridad)
```

| Entidad | Contiene | Se destruye cuando |
| --- | --- | --- |
| **Job** | cuota de memoria/hilos/handles, lista de procesos | no tiene procesos y no queda ningún handle a él |
| **Process** | `AddressSpace`, `HandleTable`, `ExitStatus`, hilos | todos sus hilos terminaron **y** se cerró el último handle a su objeto |
| **Thread** | stack de kernel de 16 KB, contexto de CPU (XSAVE), estado, prioridad, afinidad | termina y el *reaper* libera su stack |

- Todo proceso nace dentro de un **Job**. El Job de sesión (el de cada login) permite **terminar de golpe a todos sus procesos**, sin huérfanos y sin necesidad de rastrearlos uno por uno.
- `spawn` (ADR-04) crea un proceso nuevo con los handles que el padre elige heredar.

## 2. Ciclo de vida y terminación

**Sin zombies por diseño.** El estado de salida vive en el *objeto* `Process`, que persiste mientras alguien tenga un handle a él. El padre espera con `wait(handle)` y, cuando cierra el handle, el objeto se libera. No existe una tabla global de PIDs que alguien tenga que "recoger".

### Estados de un hilo

```text
New ──► Ready ◄──► Running
          ▲           │
          │           ▼
        Blocked ◄─────┤
                      ▼
                   Exiting ──► Dead (stack liberado por el reaper)
```

### Secuencia de terminación de un proceso (orden obligatorio)

1. Se marca el proceso como `Exiting`. Ningún hilo nuevo puede crearse en él.
2. Se **detienen todos sus hilos**. En SMP, con IPI a los núcleos que los ejecuten.
3. Se **cancelan las operaciones async pendientes** (I/O de disco, esperas), esperando a que confirmen. Así ningún driver escribe en memoria que ya no es del proceso.
4. Se **cierran todos los handles** en orden inverso al de creación. Cada objeto se libera por su `Drop` (archivos, buffers compartidos, display, etc.).
5. Se **desmapean las VMAs** y se liberan sus frames y las tablas de páginas.
6. Se **devuelve la cuota** al Job y se libera la contabilidad.
7. Se guarda el `ExitStatus` en el objeto y se **despierta a quien lo esperaba**.
8. Los stacks de kernel de sus hilos los libera el **reaper** (un hilo del kernel). Un hilo no puede liberar el stack sobre el que corre.

> [!IMPORTANT]
> El paso 3 debe preceder al 5. Liberar la memoria con I/O/DMA en vuelo es una fuente clásica de corrupción. La verificación está en los tests de ciclo de `memory-management.md`.

### Muerte anormal
- Fallo de página irrecuperable, instrucción ilegal o violación de capability → el proceso termina por la misma secuencia con un código de causa (`EXIT_FAULT`, `EXIT_KILLED`, `EXIT_OOM`). **Nunca** tumba el kernel.
- `init` es especial: si muere, el kernel hace `panic` con un mensaje claro, porque el sistema ya no es utilizable.

## 3. Límites por defecto

| Recurso | Por proceso | Por Job de sesión |
| --- | --- | --- |
| Memoria | 50% de la RAM | 75% de la RAM |
| Hilos | 256 | 1024 |
| Handles | 4096 | 32768 |

Si se supera un límite, la operación devuelve error (`Err(LimitExceeded)`) y el sistema sigue sano.

## 4. Scheduler

### 4.1 Política (v1.0)

**MLFQ simplificada**, con colas por prioridad y cola por CPU.

- **32 niveles** de prioridad agrupados en clases:

| Clase | Prioridades | Uso |
| --- | --- | --- |
| `Realtime` | 24-31 | hilos del kernel críticos, IRQ threads, audio |
| `Interactive` | 16-23 | shell, compositor, input |
| `Normal` | 8-15 | programas de usuario por defecto |
| `Batch` | 1-7 | tareas en segundo plano |
| `Idle` | 0 | un hilo por CPU, ejecuta `HLT`/`MWAIT` |

- Selección en **O(1)**: un bitmap de colas no vacías y `find_first_set`.
- **Quantum:** 4 ms en `Normal`, 8 ms en `Batch`, 2 ms en `Interactive`. Los hilos que se bloquean pronto (I/O) mantienen o suben prioridad. Los que consumen todo su quantum bajan un nivel (dentro de su clase).
- **Anti-inanición:** cada 1 s, los hilos que llevan esperando sin ejecutar se suben de nivel.
- **Herencia de prioridad** en los locks que duermen, para evitar inversión de prioridad.
- La **shell gráfica y el compositor** corren como `Interactive`. Así la interfaz responde aunque haya trabajo pesado.

**Por qué no un scheduler tipo CFS/EEVDF:** son más justos pero mucho más complejos y difíciles de depurar. Una MLFQ O(1) es simple, predecible y rápida. Se reevalúa por medición.

### 4.2 Tiempo
- **Sin tick periódico en reposo (tickless idle).** El APIC timer se programa en modo one-shot o TSC-deadline, y solo se despierta cuando hace falta. Menos energía y menos interrupciones inútiles.
- Idle con `HLT` o `MWAIT`. Se detecta el soporte en CPUID.

### 4.3 Cambio de contexto
- Se guarda el estado FPU/SIMD con **`XSAVEOPT` de forma eager** (la FPU perezosa tiene vulnerabilidades conocidas y en CPUs modernas no ahorra nada).
- Se cambia solo `CR3` si el proceso es distinto. Si es otro hilo del mismo proceso, no se toca la TLB.
- PCID cuando exista, para evitar flushes completos de TLB.
- Objetivo: **≤ 2 µs** (ADR-14).

### 4.4 Preempción del kernel: Preempción Voluntaria Condicionada (`PREEMPT_VOLUNTARY`)
- **Estructura base:** Kernel no preemptible arbitrariamente en v1.0 para minimizar bugs de concurrencia y simplificar invariantes en spinlocks.
- **Mecanismo `cond_resched()`:**
  - Cada CPU mantiene un flag atómico en su bloque local: `cpu_local::need_resched`.
  - El APIC timer activa `need_resched = true` si expira el quantum del hilo actual o si un hilo de clase `Interactive` pasa al estado `Ready`.
  - Las operaciones $O(N)$ y bucles extensos del kernel (sync/journaling en ext4, recorridos de VMAs, desmapeos masivos de memoria) insertan llamadas a:
    ```rust
    #[inline(always)]
    pub fn cond_resched() {
        if cpu_local::need_resched() {
            scheduler::reschedule();
        }
    }
    ```
  - **Garantía:** Cero sobrecoste en el camino rápido de ejecución, pero garantiza que ninguna syscall pesada degrade la latencia interactiva ($\le 16\text{ ms}$).

### 4.5 SMP y work-stealing
- Runqueue **por CPU**, con su propio lock. Un hilo nuevo se coloca en la CPU menos cargada de su afinidad.
- Una CPU que queda ociosa **roba la mitad** de los hilos listos de la cola más cargada, evitando mover hilos con caché caliente (los que corrieron hace menos de ~1 ms).
- Reglas de locks: orden global documentado, nunca dos locks de runqueue a la vez sin orden fijo, sin asignar memoria bajo spinlock.
- Un **watchdog** detecta un núcleo que no ha pasado por el scheduler en 5 s (soft lockup) y lo vuelca por serial.

### 4.6 Executor async interno
- Corre dentro de hilos del kernel (`Realtime` bajo, `Normal` según el caso) con un **presupuesto de polls** por turno, para que ninguna tarea monopolice la CPU.
- Las tareas async que pertenecen a un proceso se cancelan en el paso 3 de la terminación.

## 5. Primitivas de sincronización para userspace
- `wait(handle)` / `signal` sobre objetos del kernel.
- **`futex`-like:** `wait_on(addr, expected)` / `wake(addr, n)`, base de `pthread_mutex` y `pthread_cond` en `sys-libc`.
- Timers: `sleep`, `timer_create` como objeto con handle.
- No hay señales Unix en v1.0 (D-H). Los eventos asíncronos se entregan como mensajes o se leen desde un handle.

## 6. Verificación

1. Test de ciclo: crear y destruir 10.000 procesos y comprobar la línea base de memoria y de objetos (`Process`, `Thread`, `Handle`).
2. Matar procesos en cada estado posible, incluso en mitad de un `spawn`, de un `wait` y de una operación de disco.
3. Matar un Job completo con árboles profundos de procesos.
4. `loom` sobre runqueues, wait queues y la tabla de handles.
5. Medición continua: latencia de cambio de contexto, de wakeup y de scheduling bajo carga, con regresión en CI.
6. Prueba de *fork bomb* equivalente (`spawn` recursivo): los límites deben detenerla sin afectar al login.

## 7. Decisiones abiertas

- Si se añade afinidad e isolation de CPU para tareas en tiempo real.
- Si se reemplaza MLFQ por un esquema basado en *deadline*.

## 8. Implementación actual

Comportamiento implementado en `kernel/src/task/{process,scheduler,syscall}.rs` y `kernel/src/arch/x86_64/percpu.rs`.

### 8.1 Estructura del planificador MLFQ $O(1)$

```rust
pub struct Scheduler {
    pub queues: [VecDeque<Process>; 32],
    pub active_bitmap: u32,
    pub sleeping: VecDeque<Process>,
    pub zombies: VecDeque<Process>,
    pub current: Option<Process>,
    pub total_ticks: u64,
    pub started: bool,
}
```

- **32 niveles de prioridad agrupados por clases:**
  * Realtime / Drivers / Kernel Threads: prioridades 24–31 (quantum: 2 ticks = 20 ms).
  * Interactivo / Vanish / TUI / Eventos: prioridades 16–23 (quantum: 4 ticks = 40 ms).
  * Normal / Desktop: prioridades 8–15 (quantum: 8 ticks = 80 ms).
  * Batch / Tareas en segundo plano: prioridades 0–7 (quantum: 16 ticks = 160 ms).
- **Selección $O(1)$:** `active_bitmap` contiene los bits de colas no vacías. La selección se realiza con `31 - active_bitmap.leading_zeros()`.
- **Anti-inanición (*Priority Boost*):** Cada 1000 ticks (10 s), los procesos en colas 0–15 se ascienden de prioridad a colas superiores para prevenir inanición de tareas de fondo.

### 8.2 Estructura y campos de `Process`

```rust
pub struct Process {
    pub id: ProcessId,
    pub name: String,
    pub state: ProcessState,
    pub stack_pointer: u64,
    pub cr3: u64,
    pub cpu_ticks: u64,
    pub ram_bytes_used: u64,
    pub gpu_context: u64,
    pub pending_signals: u64,
    pub waiting_for: Option<ProcessId>,
    pub kernel_stack_ptr: u64,
    pub pml4_frame: usize,
    pub privilege_level: u8,
    pub handles: HandleTable,
    pub priority: u8,
    pub quantum_remaining: u8,
    pub affinity_mask: u64,
}
```

### 8.3 Estados de Proceso

```rust
pub enum ProcessState { Ready, Running, Blocked, Sleeping(u64), Zombie, Dead }
```

- `Sleeping(target_tick)`: almacena el tick en el cual el temporizador debe despertar el proceso.
- `Zombie`: encolado en `zombies` y consumido por `reaper_thread` para liberar la pila de kernel (`dealloc`) y el espacio virtual (`destroy_userspace_pml4`).

### 8.4 Área Local por CPU (`PerCpuArea`) y `GS_BASE`

En [`kernel/src/arch/x86_64/percpu.rs`](file:///home/x4/Documents/LixOS/kernel/src/arch/x86_64/percpu.rs), cada procesador dispone de su propia estructura alineada a 64 bytes referenciada por `IA32_GS_BASE`:

```rust
#[repr(C, align(64))]
pub struct PerCpuArea {
    pub kernel_rsp: u64,       // gs:[0] - Leído en swapgs
    pub user_rsp: u64,         // gs:[8] - Leído en swapgs
    pub self_ptr: u64,         // gs:[16] - Puntero a sí mismo
    pub cpu_id: u32,           // gs:[24]
    pub lapic_id: u32,         // gs:[28]
    pub current_pid: u64,      // gs:[32]
    pub tss: TaskStateSegment, // gs:[40]
}
```

El cambio de contexto actualiza la pila de interrupciones del TSS local y el registro `kernel_rsp` de la CPU actual en $O(1)$ sin recurrir a variables estáticas globales compartidas.

---

## 9. Arquitectura Multinúcleo (SMP) y Colas por CPU

Para habilitar la ejecución concurrente en múltiples núcleos garantizando aislamiento de fallos, mínima contención de cerraduras y absoluta independencia del bootloader (preparado para `LixBoot`, Hito M5):

### 9.1 Principio de Diseño: Estabilidad y Eficiencia sobre "Elegancia"

Los modelos de planificación puramente teóricos o "elegantes" (como deques lock-free de robo tipo Chase-Lev, árboles rojinegros balanceados bajo interrupción de reloj tipo CFS/EEVDF, o colas globales lockless con bucles CAS intensivos) introducen fragilidad extrema en bare metal x86_64:
- Son vulnerables a problemas sutiles de ordenamiento de memoria y barreras hardware (`MFENCE`/`LFENCE`).
- Provocan degradación por rebote de líneas de caché (*cache-line bouncing* / false sharing).
- Requieren gestión de memoria compleja o alocaciones dinámicas incompatibles con interrupciones críticas.

**Enfoque de LixOS:**
1. **Aislamiento Radical:** Arreglo estático de runqueues por CPU (`RUNQUEUES`) protegidas individualmente por `IrqSafeSpinlock`.
2. **Cero Contención en Camino Rápido:** En cada tick de temporizador (100 Hz) o llamada a `sys_yield`, la CPU $N$ bloquea **única y exclusivamente** su propia cola `RUNQUEUES[N]`. Las restantes $N-1$ CPUs continúan ejecutando tareas a máxima velocidad sin contención en el bus.
3. **Cero Alocación Dinámica:** Las transiciones de estado, selecciones de tarea y cambios de contexto se realizan sin invocar al heap (`no alloc`).
4. **Determinismo Libre de Interbloqueos (*Deadlock-Free*):** El robo de trabajo solo se activa cuando una CPU está completamente ociosa, y la adquisición de cerraduras sigue una jerarquía canónica estricta.

### 9.2 Independencia del Bootloader: Descubrimiento Nativo por Hardware (ACPI MADT)

El kernel **no depende de ningún protocolo multiprocesador de bootloaders externos** (se descarta y prohíbe `limine::MpRequest`). En el futuro, el cargador nativo `LixBoot` solo transferirá el control al BSP entregando el puntero físico RSDP en la estructura `BootInfo`.

El descubrimiento y enumeración de núcleos secundarios se ejecuta 100% en el kernel mediante hardware estándar:
1. **Localización de Tablas ACPI:** El BSP toma `BootInfo.rsdp_addr`, valida la signatura RSDP (`"RSD PTR "`) y accede a la tabla RSDT (punteros de 32 bits) o XSDT (punteros de 64 bits).
2. **Parser de la Tabla MADT (`'APIC'`):**
   - Extrae la dirección base de MMIO del Local APIC (por defecto `0xFEE0_0000` o entrada Tipo 5 *Local APIC Address Override*).
   - Recorre las entradas de tipo 0 (*Processor Local APIC*):
     ```rust
     #[repr(C, packed)]
     pub struct MadtLocalApicEntry {
         pub entry_type: u8, // 0
         pub length: u8,     // 8
         pub acpi_proc_id: u8,
         pub apic_id: u8,
         pub flags: u32,     // Bit 0: Enabled, Bit 1: Online Capable
     }
     ```
   - Registra todos los núcleos habilitados (`flags & 1 != 0`) en la tabla estática de topología `CPUS[MAX_CPUS]`.

### 9.3 Secuencia Nativa de Arranque de APs (Trampolín INIT-SIPI-SIPI)

Los procesadores secundarios (*Application Processors* o APs) arrancan por hardware en Modo Real de 16 bits tras el encendido. El BSP los levanta mediante el protocolo estándar de la especificación MP de Intel/AMD:

1. **Instalación del Trampolín en Memoria Baja:**
   - Se copia una rutina binaria compacta de bootstrap en una página física debajo de 1 MiB (`0x8000`, página 8).
   - La rutina de 16 bits:
     * Carga una GDT temporal básica de 32 bits.
     * Activa el bit `PE` en `CR0` para pasar a Modo Protegido de 32 bits.
     * Habilita `PAE` en `CR4`.
     * Carga el registro `CR3` con la tabla de páginas PML4 del kernel.
     * Activa el bit `LME` en el MSR `IA32_EFER` (`0xC000_0080`).
     * Activa la paginación (`CR0.PG = 1`) para entrar en Long Mode de 64 bits.
     * Salta a la rutina de 64 bits `ap_entry_point`.

2. **Protocolo de Disparo vía Local APIC ICR (*Interrupt Command Register*):**
   Para cada AP detectado en la MADT:
   - **INIT IPI:** Se escribe en el registro ICR (`0x300` / `0x310`) con modo de entrega INIT (`0x0000_4500` con el APIC ID de destino).
   - **Retardo de estabilización:** Espera calibrada de 10 ms (vía PIT o TSC).
   - **Startup IPI (SIPI):** Se envía el vector `0x08` (dirección `0x08 << 12 = 0x8000`).
   - **Retardo de 200 µs:** Si el núcleo no ha respondido en su bandera de sincronización atómica, se emite un segundo SIPI idéntico.
   - **Handshake Determinista con Timeout:** El BSP espera que el AP incremente la variable atómica `AP_ONLINE_COUNT` con un límite estricto de 100 ms. Si la CPU no responde en ese plazo, se marca como `DISABLED_UNRESPONSIVE` y el sistema continúa de forma segura sin congelarse.

3. **Inicialización Local del AP (`ap_entry_point`):**
   Al tomar el control en 64 bits, cada AP:
   - Establece su pila de kernel preasignada (`kernel_rsp`).
   - Carga su GDT y TSS local configurada en su `PerCpuArea`.
   - Carga el puntero `IA32_GS_BASE` hacia su estructura `PerCpuArea` específica.
   - Carga el registro IDTR con la tabla IDT compartida del kernel.
   - Programa los MSRs `STAR`, `LSTAR`, `FMASK` para `SYSCALL`/`SYSRET`.
   - Inicializa y habilita su Local APIC y programa su temporizador periódico a 100 Hz (Vector 32).
   - Notifica `AP_ONLINE_COUNT` y entra en el bucle del planificador local.

### 9.4 Estructura `PerCpuRunqueue` y Topología

Cada CPU física/lógica posee su propia cola de planificación aislada:

```rust
pub const MAX_CPUS: usize = 32;

pub struct PerCpuRunqueue {
    pub cpu_id: u32,
    pub lapic_id: u32,
    pub queues: [VecDeque<Process>; 32],
    pub active_bitmap: u32,
    pub current: Option<Process>,
    pub total_ticks: u64,
    pub quantum_remaining: u8,
    pub idle_ticks: u64,
}

pub static RUNQUEUES: [IrqSafeSpinlock<PerCpuRunqueue>; MAX_CPUS] = ...;
pub static ONLINE_CPUS: AtomicU32 = AtomicU32::new(1);
```

### 9.5 Asignación y Balanceo de Carga (*Work-Stealing*)

1. **Inserción Inicial (`spawn`):**
   Al crear un proceso, se evalúa su `affinity_mask`. El proceso se encola directamente en la CPU con la menor cantidad de procesos en cola:
   $$\text{target\_cpu} = \arg\min_{c \in \text{affinity}} \sum_{p=0}^{31} \text{len}(\text{queues}_c[p])$$

2. **Robo de Trabajo (*Work-Stealing*):**
   Cuando una CPU agota todas sus tareas listas (`active_bitmap == 0`):
   - Escanea las runqueues de las demás CPUs activas para identificar a la CPU víctima con mayor carga.
   - **Prevención Matemática de Deadlocks:** La adquisición simultánea de cerraduras entre dos CPUs siempre respeta el orden canónico estricto de índices:
     $$\text{lock}(\min(\text{cpu}_A, \text{cpu}_B)) \longrightarrow \text{lock}(\max(\text{cpu}_A, \text{cpu}_B))$$
   - **Filtro de Afinidad y Clases:** Solo se roban tareas si la CPU ladrona está presente en el `proc.affinity_mask`, y solo de clases generales (`Normal` o `Batch`, prioridades < 24). Las tareas `Realtime` e `Interactive` nunca se mueven para preservar localidad de caché y tiempos de respuesta.
   - Si ninguna CPU contiene tareas transferibles, la CPU libera los locks y pasa a su hilo `idle` local ejecutando `hlt`.

### 9.6 Interrupciones Inter-Procesador (IPI)

El Local APIC gestiona dos vectores de interrupción fijos:
* **Vector 34 (`IPI_RESCHEDULE`):** Disparado cuando una tarea interactiva o de alta prioridad se encola en una CPU que se encuentra en reposo (`idle`), forzando su despertar inmediato sin esperar al siguiente tick.
* **Vector 35 (`IPI_TLB_SHOOTDOWN`):** Disparado durante desmapeos o modificaciones de páginas virtuales compartidas para invalidar entradas de TLB (`invlpg`) en todos los núcleos asociados al espacio de direcciones.

### 9.7 Watchdog por Núcleo

Cada CPU actualiza un contador atómico local `last_tick` en cada interrupción de temporizador. Si un núcleo no avanza en 5 segundos (posible soft-lockup o bucle infinito en Ring 0), el BSP emite una traza diagnóstica por el puerto serie sin comprometer la ejecución de los núcleos restantes.
