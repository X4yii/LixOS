# Identidad, autenticación y elevación

> **Estado:** diseño implementado parcialmente · **ADR:** 05, 10, 23
> **Relacionado:** [Capabilities](../spec/capabilities.md) · [ABI](../spec/syscall-abi.md) · [Conectividad](connectivity.md)

## Modelo de privilegio

LixOS usa dos niveles efectivos de ejecución: kernel Ring 0 y procesos no confiables Ring 3. No se ejecuta código de aplicación en Ring 1 para “sudo”: con paginación x86, CPL 0, 1 y 2 pertenecen al modo supervisor y pueden acceder a páginas supervisoras. IOPL también da acceso a instrucciones de puerto y no constituye una frontera de seguridad. Ningún ELF de usuario puede elegir CPL o IOPL.

`sudo`/`vdo` significa **elevación de autoridad**: authd crea un proceso hijo que continúa en Ring 3, hereda un conjunto explícito y mínimo de handles/capabilities con revocación y plazo, y registra la concesión. `su0` es una sesión administrativa con un conjunto más amplio de capabilities en Ring 3; no es una sesión Ring 0 ni un usuario universal del kernel. Las operaciones exclusivas del kernel siguen siendo syscalls comprobadas.

Esto conserva ADR-01 (drivers internos en Ring 0), ADR-05 (autoridad por handles), ADR-10 (hash en userspace) y ADR-23 (sin setuid/root ambiente). El usuario conserva la propiedad de sus datos; la administración se modela como capacidades, no como un número UID privilegiado.

## Servicios

- `sessiond`: autentica una sesión y crea el namespace del usuario con permisos mínimos.
- `authd`: único dueño del almacén de verificadores; aplica Argon2id, limitación de intentos, comprobaciones de grupo/política y auditoría. Nunca entrega el almacén a un proceso iniciado por usuario.
- `vdo`: solicita a authd una elevación para un comando concreto. No interpreta reglas ni crea handles por sí mismo.
- `netmgr`: gestiona interfaces, perfiles Wi‑Fi, DHCP y selección de rutas mediante la API de red.
- `bluetoothd`: gestiona controladores HCI, dispositivos emparejados y perfiles permitidos.

Los servicios requieren init, canales IPC, namespaces y concesión de handles en `spawn`; como esas piezas del ABI están incompletas, todavía no se consideran servicios arrancables ni confiables.

## Contraseñas y secretos de red

Los verificadores de acceso usan Argon2id con sal aleatoria única y parámetros calibrados según ADR-10. El fichero de política guarda identificador, grupos y verificador, nunca contraseñas. El primer administrador se crea durante el alta del sistema mediante consola local; no hay contraseña predeterminada.

Las claves WPA-PSK y las claves de bonding Bluetooth no se guardan en texto plano. El almacén cifra cada secreto con AEAD, nonce único y versión de formato; una clave de bóveda aleatoria se envuelve con una clave derivada de la contraseña mediante Argon2id con separación de dominio. El login desbloquea la bóveda; al cerrar sesión se borran las copias en RAM. La recuperación requiere una clave de recuperación explícita generada durante el alta. Si el sistema no dispone de RNG criptográfico y primitivas mantenidas/validadas, no persiste secretos ni simula cifrado.

La negociación Wi‑Fi se delega a un supplicant en userspace una vez exista la API de red. La etapa inicial permite WPA2-Personal CCMP/AES; WPA3-SAE y protección de tramas de gestión solo se anuncian cuando su handshake completo está implementado y validado. WEP, TKIP y redes abiertas no se presentan como seguras.

## Implementación actual y pendientes

- `services/authd/src/vault.rs` implementa el derivador Argon2id (m=19 MiB, t=2, p=1), comparación constante del verificador y cifrado AEAD AES-256-GCM-SIV con datos asociados. `UserDatabase` y la política `AuthService` tienen interfaces implementadas; el binario no es todavía un daemon operativo (`services/authd/src/main.rs` es un *stub* de cinco líneas que informa de que faltan el bootstrap de capacidades y el IPC).
- **Rate-limit de intentos: implementado.** `services/authd/src/lib.rs` declara `MAX_FAILED_ATTEMPTS: u32 = 5` y `MAX_LOCKOUT_SECONDS: u64 = 300`. `AuthService<V, B, A, C>` lleva el tiempo como parámetro mediante el trait `Clock` (inyectado, no se llama a un reloj global): tras el quinto intento fallido se aplica un *backoff* que crece por intento y nunca supera los 300 segundos (`entry.2 = now.saturating_add(backoff.min(MAX_LOCKOUT_SECONDS))`). El límite es por entrada de usuario y no depende de la plataforma, así que es testeable en host.
- `EncryptedSecret` define un sobre versión 1 con nonce de 96 bits y ciphertext/tag. `wrap_vault_key`/`unlock_vault_key` separan la clave aleatoria de datos de la clave de envoltura derivada de contraseña; el llamador aporta clave aleatoria, sal independiente, nonce y AAD. Faltan la fuente de entropía, almacenamiento persistente atómico y migraciones del formato. La contraseña exige al menos 8 bytes y el módulo limita entradas a 1024 bytes.
- **Autoridad por proceso: implementada.** Cada `Process` lleva `rights: lix_sys::Rights` (`kernel/src/task/process.rs`), con las máscaras de `kernel/src/task/rights.rs`:
  - `BASE_USER` — derechos de un proceso normal (`READ`, `WRITE`, `EXEC`, `DUP`, `TRANSFER`, `WAIT`, `SIGNAL`, `MAP`, `ENUMERATE`, `TRAVERSE`, `MANAGE`, `INSPECT`, `SPAWN`, `DISPLAY_OWN`, `INPUT_READ`, `DEVICES`, `LOG`, `NET_READ`, `NET_CONNECT`). **No** incluye `POWER`, `AUTH`, `ELEVATE`, `NET_ADMIN` ni los de Bluetooth.
  - `SESSION` = `BASE_USER | POWER | AUTH` — sesión administrativa; es lo que reciben `vanish` y los ELF lanzados desde la consola (`kernel/src/main.rs`, `kernel/src/shell/mod.rs`).
  - `KERNEL` = `u32::MAX` — solo el proceso inicial creado por el propio kernel (`kernel/src/task/scheduler/spawn.rs`).
  - `sys_process_spawn` otorga `BASE_USER` al hijo (`kernel/src/task/syscall/proc.rs`), no `SESSION`.
  - El dispatcher comprueba `required_rights` **antes** de ejecutar el handler: lee el derecho del registro generado (`kernel/src/task/syscall_table.rs`, origen `abi/syscalls.toml`), lo traduce con `rights::required()` y devuelve `EACCES` si `Process.rights` no lo contiene (`kernel/src/task/syscall/mod.rs`). Este enforcement es efectivo, pero **solo cubre el paso previo al handler**: la revocación activa, los namespaces y la concesión inicial por parte de `init` siguen sin existir.
- Faltan RNG de sistema enlazado, persistencia/serialización de cuentas y bóveda, `init`, `sessiond`, canales, revokers y ABI funcional de elevación. Sin RNG/persistencia no se crean ni desbloquean perfiles guardados en el sistema real.
- La shell tenía prefijos `sudo`/`su0` que elegían un nivel de privilegio al cargar ELF. Se retiró esa elevación directa: ahora fallan cerrados hasta que authd pueda crear un hijo con handles limitados.
- La carga de ELF se limita a Ring 3 e IOPL 0. Los procesos de usuario no reciben los handles raíz descritos en capabilities.md.
- No hay almacén persistente de contraseñas/perfiles, ni desbloqueo conectado al login, RNG criptográfico enlazado o rutinas WPA/WPA3/Bluetooth pairing integradas.

## Hitos

1. Implementar init/sessiond y la creación/herencia explícita de handles en `spawn`.
2. Implementar canales, revokers y ABI versionado, incluidos plazos y auditoría.
3. Enlazar RNG criptográfico y persistencia atómica con las primitivas existentes; añadir recuperación/rotación y revisión de formato versionado.
4. Implementar authd y alta local sin credenciales por defecto.
5. Implementar `vdo`/`su0` como clientes de authd; probar que expiran/revocan capacidades y nunca cambian Ring 3.
6. Conectar bóveda y perfiles de red mediante APIs versionadas de netmgr y bluetoothd.
