# Modelo de capabilities

> **Tipo:** especificación · **Estado:** borrador · **ADR:** 05, 23
> **Relacionado:** [ABI de syscalls](syscall-abi.md)

Define el modelo de autoridad de LixOS: handles opacos con derechos atenuables, namespaces compuestos y revocación activa, sin un usuario `root` omnipotente y sin permisos escalables.

Objetivo: **que un programa solo pueda tocar lo que se le dio explícitamente.**

## 1. Conceptos

- **Objeto del kernel:** algo con identidad (archivo, directorio, proceso, canal, pantalla, disco...).
- **Handle:** referencia de un proceso a un objeto, con una **máscara de derechos**. Vive solo en la tabla del kernel; el proceso ve un número opaco.
- **Derecho:** permiso concreto sobre ese objeto a través de ese handle.
- **Autoridad = poseer un handle.** No hay otra forma de actuar sobre un objeto.

```text
Proceso A                       Kernel
 handle 0x0000_0003_0000_0007 ──► [ idx 7, gen 3 ] ──► (objeto: Archivo X, derechos: READ|DUP)
```

## 2. Formato del Handle

`u64 = [generación:32 | índice:32]`

- El **índice** apunta a una entrada de la tabla del proceso. `índice 0` es siempre inválido.
- La **generación** sube cada vez que se reutiliza la entrada. Un handle viejo apunta a una generación que ya no existe y falla con `EBADF`. Previene use-after-free de handles.
- Tabla por proceso, crece en bloques, límite por defecto 4096 (ADR-20).
- Cada entrada: `(Arc<Objeto>, derechos u32, revoker Option<Arc<RevokeFlag>>)`.

## 3. Derechos

| Bit | Nombre | Sentido |
| --- | --- | --- |
| 0 | `READ` | leer datos / recibir mensajes |
| 1 | `WRITE` | escribir / enviar / modificar |
| 2 | `EXEC` | ejecutar como programa |
| 3 | `DUP` | duplicar el handle (con igual o menos derechos) |
| 4 | `TRANSFER` | enviarlo a otro proceso por un canal |
| 5 | `WAIT` | esperar sus señales |
| 6 | `SIGNAL` | enviar notificaciones o señales de objeto |
| 7 | `MAP` | mapear en memoria |
| 8 | `ENUMERATE` | listar un directorio |
| 9 | `TRAVERSE` | resolver nombres dentro de un directorio |
| 10 | `MANAGE` | administrar (matar procesos, ajustar límites, enlazar un namespace) |
| 11 | `INSPECT` | consultar información |
| 12 | `SPAWN` | crear procesos en un Job |
| 13 | `DISPLAY_OWN` | ser dueño de la pantalla |
| 14 | `INPUT_READ` | leer eventos de entrada |
| 15 | `POWER` | apagar o reiniciar |
| 16 | `DEVICES` | abrir dispositivos de bloque |
| 17 | `AUTH` | verificar credenciales (solo `authd`) |
| 18 | `LOG` | escribir en el log del kernel |
| 19 | `ELEVATE` | solicitar elevación a `authd` (ver 7) |
| 20 | `NET_READ` | consultar enlaces y redes visibles |
| 21 | `NET_ADMIN` | configurar interfaces y perfiles de red |
| 22 | `BLUETOOTH_READ` | enumerar controladores/dispositivos Bluetooth |
| 23 | `BLUETOOTH_ADMIN` | emparejar y administrar perfiles Bluetooth |
| 24 | `SECRET_USE` | usar un secreto de bóveda para una operación autorizada; no leer bytes de clave |
| 25 | `NET_CONNECT` | crear sockets de red para tráfico autorizado por el sistema |

Cada tipo de objeto define qué derechos tienen sentido. Los demás se rechazan al crear el handle.

## 4. Operaciones

| Operación | Regla |
| --- | --- |
| **Atenuar** (`dup`, `replace`) | El nuevo conjunto de derechos debe ser un **subconjunto**. Ampliar es imposible. |
| **Transferir** (`channel_send`) | El handle se **mueve** al receptor y se cierra en el emisor. Requiere `TRANSFER`. Si se quiere conservar, se hace `dup` primero. |
| **Heredar** (`spawn`) | El padre lista qué handles pasa y con qué derechos. No se hereda nada por defecto. |
| **Cerrar** | Libera la entrada. Si es el último handle, se libera el objeto (RAII, ADR-19). |
| **Revocar** | Ver 6. |

## 5. Namespace compuesto (ADR-23)

Un proceso no tiene una raíz global ambiente. Tiene un **handle de namespace**, un directorio virtual compuesto de enlaces:

```text
Namespace de la sesión de "ana":
  /bin        → dir_h(/bin)          [READ|EXEC|TRAVERSE|ENUMERATE]
  /lib        → dir_h(/lib)          [READ|TRAVERSE|ENUMERATE]
  /etc        → dir_h(/etc-pub)      [READ|TRAVERSE|ENUMERATE]
  /home/ana   → dir_h(/home/ana)     [READ|WRITE|EXEC|TRAVERSE|ENUMERATE]
  /tmp        → dir_h(/tmp)          [READ|WRITE|TRAVERSE|ENUMERATE]
  /dev        → dir_h(/dev-user)     [READ|WRITE|TRAVERSE]
```

- Se construye con `sys_namespace_create` + `sys_namespace_bind`. Cada enlace lleva un máscara de derechos que **limita** lo que se puede hacer debajo.
- `open_at(ns, "/home/ana/x")` encuentra el enlace más específico y aplica sus derechos. Un `..` nunca sale del namespace.
- `sys_namespace_derive` crea un namespace **más pequeño** para confinar un programa (por ejemplo, un navegador con solo `/tmp` y un directorio de descargas).
- Lo que no está en el namespace **no existe** para ese proceso. Así `/var/lib/auth/passwords` simplemente no aparece para usuarios normales.
- El `cwd` lo mantiene la libc como una ruta dentro del namespace. Toda resolución parte del handle raíz (ADR-22).

## 6. Revocación Activa (Syscalls y Memoria Virtual)

> **Reservado, sin implementar.** Las syscalls de este apartado **no existen hoy en el ABI**: `sys_revoker_create` (`0x007`) y `sys_revoker_revoke` (`0x008`) fueron retiradas de `abi/syscalls.toml` porque no tienen handler en el kernel. Sus IDs quedan **reservados** y no se reutilizan (regla 5 de [syscall-abi.md](syscall-abi.md)). Los objetos de revocación, los *reverse mappings* y el TLB shootdown que se describen abajo son **diseño de referencia**, no comportamiento actual. Lo que sí existe hoy está documentado en la §13.7.
>
> En la misma situación se encuentran `sys_vmo_create` (`0x200`) y `sys_vm_map` (`0x202`): reservados, sin handler. Por eso el derecho `MAP` de la §3 está declarado pero todavía no se aplica en ninguna operación de mapeo.

- `sys_revoker_create` devuelve un objeto `Revoker` que controla un flag atómico `Arc<AtomicBool>` y una lista de mapeos activos (*reverse mappings*).
- `sys_handle_dup_revocable(h, rights, rev)` crea un handle ligado a ese `Revoker`.
- `sys_revoker_revoke(rev)` activa el flag y ejecuta la invalidación:
  1. **Syscalls:** A partir de ese instante, **todos** los handles derivados (incluidos los transferidos a otros procesos) fallan de inmediato con `ELIX_REVOKED`. Coste en syscalls: una comprobación atómica relajada sólo en handles revocables.
  2. **Memoria Virtual Mapeada (VMOs con derecho `MAP`):** El kernel mantiene un registro inverso de todas las VMAs creadas a partir de ese handle en cualquier proceso. Durante `sys_revoker_revoke`:
     - Se limpian las entradas de tablas de páginas (PTEs con bit *Present* a 0) en los procesos afectados.
     - Se emite una interrupción interprocesador (**TLB Shootdown**) para invalidar las líneas cacheadas en la TLB de todos los núcleos de CPU activos.
     - Cualquier acceso posterior de hardware detona un `#PF` controlado que termina la VMA o genera `EXIT_FAULT`.
- Los **handles y recursos de un proceso se pueden revocar sin matarlo**, esencial para las elevaciones transitorias de `vdo`.

## 7. Privilegios sin `root`

No existe el usuario todopoderoso. El kernel crea `init` con **todas** las capabilities raíz (`root_job`, `root_namespace`, `POWER`, `DEVICES`, `AUTH`, `DISPLAY_OWN`, `INPUT_READ`). `init` las reparte:

| Servicio | Recibe | Por qué |
| --- | --- | --- |
| `authd` | `AUTH`, acceso al almacén de hashes, `ELEVATE`-grant | verifica contraseñas y concede elevaciones |
| Gestor de sesión (`login`) | `SPAWN` en el Job de cada sesión, derivación de namespaces | crea la sesión del usuario |
| Compositor | `DISPLAY_OWN`, `INPUT_READ` | única razón de tener acceso a la pantalla y al teclado |
| Gestor de energía | `POWER` | `shutdown` y `reboot` |
| Instalador | `DEVICES` (solo en LiveISO) | escribir en el disco elegido |

### `vdo` (reemplazo de `sudo`) y `su0`
1. El usuario ejecuta `vdo <comando>`; `sudo` puede ser un alias compatible. El proceso y su hijo permanecen en Ring 3.
2. `vdo` pide a `authd` una concesión limitada y presenta la credencial por un canal privado.
3. `authd` verifica con Argon2id (ADR-10), aplica política/grupo y crea el hijo con namespace ampliado y handles estrictamente necesarios, ligados a `dup_revocable`.
4. Al vencer el plazo, authd revoca los handles; la aplicación no adquiere autoridad por cambio de UID/CPL.
5. `su0` solicita una sesión administrativa aprobada por authd con un conjunto más amplio, explícito y auditable de capabilities. `su0` **no** ejecuta código en Ring 0.
6. Toda concesión registra identidad, programa, capacidades, duración y resultado; nunca registra credenciales.

x86 con paginación clasifica CPL 0, 1 y 2 como supervisor. Por eso Ring 1 no sirve como frontera segura para aplicaciones ni como “sudo”; Ring 0 pertenece al kernel. No existe `setuid` ni ELF de usuario privilegiado. Solo authd puede conceder capacidades elevadas y siempre en Ring 3.

## 8. Metadatos de archivo (compatibilidad)

`ext4` conserva `uid/gid/mode` (ADR-05) y `stat()` los devuelve. `chmod`/`chown` los modifican. **El kernel no los usa para autorizar.** La autorización es el handle y el namespace. El bit de ejecución sí lo verifica el loader como política adicional (no ejecutar archivos sin `x`), no como seguridad.

## 9. Base de confianza (TCB)

La seguridad depende de que sean correctos: el **kernel**, **`init`** y **`authd`**. Se mantienen pequeños, sin dependencias innecesarias, con la cobertura de pruebas más alta y revisión obligatoria de cada cambio. Todo lo demás (compositor, shell, PML) corre con las capabilities mínimas.

## 10. Auditoría

El kernel registra en klog (con rate-limit) cada: creación de namespace, `spawn` con handles de herencia, transferencia de handles sensibles (`POWER`, `DEVICES`, `AUTH`), revocación y fallos de derechos repetidos.

## 11. Pruebas

- Para cada syscall, un test con el handle al que le falta exactamente el derecho necesario.
- **Fuzzing de la tabla de handles:** secuencias aleatorias de `dup`, `close`, `send`, `revoke` y reutilización de generaciones.
- Intentos de escape de namespace: `..`, symlinks absolutos, rutas largas, rutas con `\0`, directorios renombrados durante la resolución.
- Prueba de que un handle atenuado jamás recupera derechos.
- Verificación formal ligera (con `loom` o `proptest`) de las invariantes: *los derechos solo disminuyen* y *un handle revocado nunca vuelve a funcionar*.

## 12. Decisiones abiertas

- Si los namespaces admiten **montar** otro namespace dentro de uno (composición recursiva) o solo directorios.
- Si hay una operación `sys_handle_restrict_all` para ocultar de golpe todos los handles de un proceso antes de ejecutar código no confiable.

## 13. Implementación actual

Comportamiento observado en `kernel/src/task/handle.rs`. Los apartados siguientes describen hechos del código; no modifican la especificación de las secciones anteriores. La carga de ELF se fuerza a Ring 3; init/bootstrap y la concesión real de capabilities aún no existen.

### 13.1 Tipos de objeto

```rust
pub enum KernelObject {
    StdIn,
    StdOut,
    StdErr,
    File { path: String, cursor: u64, is_write: bool },
    Process(ProcessId),
    InputDevice { class: u32 },
    BlockDevice(Arc<BlockDeviceCapability>),
}
```

No existen todavía objetos de canal, Job, revoker, VMO ni namespace.

### 13.2 Entrada de tabla

Cada entrada es un `HandleEntry { object: KernelObject, rights: Rights, generation: u32 }`. El objeto se posee directamente, sin `Arc`, y no hay campo de revoker.

### 13.3 Reservas en los slots 0, 1 y 2

`HandleTable::new()` rellena los tres primeros slots con `insert_fixed`:

| Slot | Objeto | Derechos iniciales |
| --- | --- | --- |
| 0 | `StdIn` | `READ \| WAIT \| DUP` |
| 1 | `StdOut` | `WRITE \| DUP` |
| 2 | `StdErr` | `WRITE \| DUP` |

### 13.4 Generaciones

`Handle::new(index, generation)` empaqueta `(generation << 32) | index` en el `u64` del handle; `Handle::INVALID` es `Handle(0)`.

- Un slot nuevo arranca con `generations[idx] = 1`; los reutilizados toman el valor que quedó almacenado.
- `close` libera el slot y hace `generations[idx] = generations[idx].wrapping_add(1).max(1)`, de modo que el handle anterior queda invalidado.
- `dup` construye una entrada temporal con `generation: 0`, pero la pasa por `insert`, que la sustituye por la generación del slot libre que resulte; el `0` no llega a almacenarse.

La comprobación no es idéntica en los tres métodos:

| Método | Regla cuando `handle.generation() == 0` |
| --- | --- |
| `get` | el atajo solo se aplica a los slots 0–2 (`idx < 3`) y únicamente valida los derechos (`EACCES` si faltan); para cualquier otro índice con generación `0` la comparación es estricta y devuelve `EBADF` |
| `close` | omite la comparación y cierra el slot |
| `dup` | omite la comparación y continúa con el chequeo del derecho `DUP` |

### 13.5 Límite de la tabla

La tabla se expande en bloques a partir de una capacidad inicial de 16 entradas. Si `slots.len() >= 4096`, `install` devuelve `EMFILE`.

### 13.6 Atenuación en `dup`

- Se exige el derecho `DUP` en la entrada original, si no se devuelve `EPERM`.
- Un índice fuera de rango o una generación no coincidente devuelven `EBADF`.
- `requested_rights == 0` conserva el conjunto completo; en otro caso se aplica la intersección con el conjunto original.
- Los slots se reutilizan antes de crecer la tabla.

### 13.7 Autoridad efectiva hoy: `Process.rights` y el enforcement del dispatcher

Lo que sí está implementado (y es verificable en el árbol) es una capa de autoridad a **dos niveles**, distinta del modelo completo de las §5–§7:

**(a) Derechos del proceso.** Cada `Process` lleva `rights: lix_sys::Rights` (`kernel/src/task/process.rs`), calculado en el arranque del ELF por **declaración ∩ lanzador** (`kernel/src/task/rights.rs::grant_for`, aplicado en `kernel/src/task/elf.rs::load_elf`): `rights = declared(path) & launcher_rights`. Ni la ruta por sí sola eleva (un binario de sistema lanzado por una aplicación corriente sólo obtiene lo que la aplicación ya tenía), ni un lanzador sin derechos puede otorgarlos.

| Máscara | Valor | Quién la recibe |
| --- | --- | --- |
| `KERNEL` | `Rights(u32::MAX)` | los hilos de kernel creados en `kernel/src/task/scheduler/spawn.rs` (y por tanto `init`, que es quien lanza `vanish`) |
| `SYSTEM` | `BASE_USER \| INSPECT \| MANAGE \| DEVICES \| POWER` | los ELF bajo `/bin/system/` y `/bin/services/` **sólo si su lanzador ya los poseía** (`rights::declared`) |
| `SESSION` | `SYSTEM` (sin `AUTH`) | `vanish`, cuyo binario declara `SESSION` y que es lanzado por `init` o por la propia shell |
| `BASE_USER` | derechos ordinarios (ver abajo) | cualquier otro binario: es el suelo que recibe todo hijo de `sys_process_spawn` |

`BASE_USER` incluye `READ`, `WRITE`, `EXEC`, `DUP`, `TRANSFER`, `WAIT`, `SIGNAL`, `MAP`, `ENUMERATE`, `TRAVERSE`, `SPAWN`, `DISPLAY_OWN`, `INPUT_READ`, `LOG`, `NET_READ` y `NET_CONNECT`; **no** incluye `MANAGE`, `INSPECT`, `DEVICES`, `POWER`, `AUTH`, `ELEVATE` ni `NET_ADMIN`. `AUTH` queda fuera también de `SESSION`: es la llave de los ficheros marcados como secretos en `fs::permissions` y la shell no debe poder leerlos.

Dos controles que antes vivían en la tabla y ahora viven en el kernel, porque en la tabla eran inalcanzables o demasiado amplios:

- `sys_process_kill` y `sys_waitpid` se declaran `rights = "NONE"` y el handler comprueba «hijo propio, uno mismo, o `MANAGE`» (`kernel/src/task/syscall/proc.rs`). Exigir `MANAGE` en la tabla habría obligado a repartir `MANAGE` a todo programa que quiera recoger a un hijo; el progenitor se guarda en `Process.parent_pid`.
- `sys_block_open` deja de mirar `privilege_level <= 1` (imposible de satisfacer: `spawn_user_process` exige `privilege == 3`) y exige el derecho `DEVICES` (`kernel/src/task/syscall/block.rs::installer_privilege`).

**(a′) La consola tiene un solo dueño.** `sys_read`/`sys_read_nonblocking` sobre `fd == 0` sólo sirven al proceso en primer plano (`FOREGROUND_TASK`) o, cuando no lo hay, a la shell; el resto recibe `EAGAIN` (`kernel/src/task/syscall/io.rs::owns_console`). Así ningún proceso en segundo plano puede leer lo que el usuario teclea para la shell.

**(b) Comprobación por syscall.** El dispatcher (`kernel/src/task/syscall/mod.rs`) traduce el nombre `required_rights` de la fila generada (`kernel/src/task/syscall_table.rs`, origen `abi/syscalls.toml`) con `rights::required()` y, **antes** de ejecutar el handler, exige que `Process.rights` contenga ese derecho: si falta, devuelve `-EACCES`; si el nombre no corresponde a un derecho conocido, también `-EACCES`. Una syscall sin fila registrada devuelve `-ENOSYS`.

**(c) Prefijos protegidos del VFS.** `kernel/src/fs/permissions.rs` añade una autorización por **ruta** independiente de los handles, aplicada en las syscalls con argumento de ruta de `kernel/src/task/syscall/io.rs` (`sys_list_dir`, `sys_read_file`, `sys_write_file`, `sys_remove_file`, `sys_mkdir`):

| Regla | Comportamiento |
| --- | --- |
| `PROTECTED_PREFIXES` | `/bin`, `/boot`, `/etc`, `/lib`, `/usr`, `/root` (la propia y cualquier ruta por debajo) |
| Ruta relativa | `EINVAL` (toda ruta debe empezar por `/`) |
| Componente `..` | `EACCES` |
| Escritura en `/` | `EACCES` |
| Escritura bajo un prefijo protegido | exige el derecho `AUTH`; sin él, `EACCES` |
| Escritura ordinaria | exige `WRITE`; lectura, exige `READ` (`kernel/src/fs/permissions.rs::authorize_read/write/remove`) |

Los errores de fichero se traducen a `errno` en `kernel/src/fs/error.rs` (`FsError → ErrorCode`).

**Límites de lo anterior.** No sustituye al modelo de las §5–§7: siguen sin existir namespaces de proceso, revokers, transferencia de handles por IPC y concesión inicial por parte de `init`. La autoridad efectiva hoy es «derechos globales del proceso + prefijos de ruta del VFS», no «handles como única autoridad».

