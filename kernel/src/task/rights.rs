use lix_sys::Rights;
pub const KERNEL: Rights = Rights(u32::MAX);
/// Suelo de derechos para un programa de usuariospace corriente.
///
/// Es deliberadamente el suelo y no el techo: **no** incluye nada que permita
/// tocar el hardware directamente (`DEVICES`), enumerar barridos de bus
/// (`ENUMERATE`) ni mirar o gobernar a otros procesos (`INSPECT`, `MANAGE`).
/// Los binarios que necesitan eso se declaran como de sistema (ver
/// [`declared`]) y, aun así, no pueden recibir más derechos que su lanzador
/// (ver [`grant_for`]).
pub const BASE_USER: Rights = Rights(
    Rights::READ.0
        | Rights::WRITE.0
        | Rights::EXEC.0
        | Rights::DUP.0
        | Rights::TRANSFER.0
        | Rights::WAIT.0
        | Rights::SIGNAL.0
        | Rights::MAP.0
        | Rights::TRAVERSE.0
        | Rights::ENUMERATE.0
        | Rights::INPUT_READ.0
        | Rights::DISPLAY_OWN.0
        | Rights::LOG.0
        | Rights::NET_READ.0
        | Rights::NET_CONNECT.0
        | Rights::SPAWN.0,
);
/// Derechos de un componente del sistema: `BASE_USER` + inspección de
/// procesos y telemetría (`INSPECT`), gestión de procesos ajenos (`MANAGE`),
/// acceso directo a dispositivos (`DEVICES`) y apagado/hibernación (`POWER`).
///
/// Se declara únicamente para los binarios bajo `/bin/system/` y
/// `/bin/services/` y para la propia shell.
pub const SYSTEM: Rights = Rights(
    BASE_USER.0 | Rights::INSPECT.0 | Rights::MANAGE.0 | Rights::DEVICES.0 | Rights::POWER.0,
);
/// Derechos de la sesión que arranca `init`: la shell.
///
/// Es `SYSTEM` **sin `AUTH`**. `fs::permissions` usa `AUTH` como llave de los
/// ficheros marcados como secretos (`is_secret`), y la shell no debe poder
/// leerlos ni reclamar el dominio de autenticación: para eso está `authd`,
/// que es quien debe obtener ese derecho explícitamente. La shell sigue
/// pudiendo apagar la máquina porque el usuario teclea `poweroff` de forma
/// deliberada (el binario declarado para `vanish` es `SESSION`).
pub const SESSION: Rights = Rights(SYSTEM.0);
/// Política de derechos por binario: qué está *declarado* que un ELF puede
/// llegar a ser.
///
/// Es un manifiesto estático, no una herencia: no depende de quién lo lance
/// sino de dónde vive el binario. Mantenerlo pequeño es lo que hace que
/// [`grant_for`] sea una salvaguarda y no un adorno.
pub fn declared(path: &str) -> Rights {
    if path.starts_with("/bin/system/") || path.starts_with("/bin/services/") {
        SYSTEM
    } else if path.ends_with("/vanish.elf") {
        SESSION
    } else {
        BASE_USER
    }
}
/// Derechos efectivos de un proceso recién creado.
///
/// `declared(path) & launcher`: lo que el binario declara **y** lo que su
/// lanzador ya poseía. Así una aplicación cualquiera que lance
/// `/bin/system/lixinstall.elf` obtiene `BASE_USER ∩ SYSTEM = BASE_USER`,
/// es decir que no hay elevación por ruta: sólo quien ya tiene esos derechos
/// (el shell de kernel, `init`, o la shell de usuariospace) puede producir un
/// proceso con derechos de sistema.
pub fn grant_for(path: &str, launcher: Rights) -> Rights {
    Rights(declared(path).0 & launcher.0)
}
pub fn required(name: &str) -> Option<Rights> {
    let rights = match name {
        "NONE" => Rights::NONE,
        "READ" => Rights::READ,
        "WRITE" => Rights::WRITE,
        "EXEC" => Rights::EXEC,
        "DUP" => Rights::DUP,
        "TRANSFER" => Rights::TRANSFER,
        "WAIT" => Rights::WAIT,
        "SIGNAL" => Rights::SIGNAL,
        "MAP" => Rights::MAP,
        "ENUMERATE" => Rights::ENUMERATE,
        "TRAVERSE" => Rights::TRAVERSE,
        "MANAGE" => Rights::MANAGE,
        "INSPECT" => Rights::INSPECT,
        "SPAWN" => Rights::SPAWN,
        "DISPLAY_OWN" => Rights::DISPLAY_OWN,
        "INPUT_READ" => Rights::INPUT_READ,
        "POWER" => Rights::POWER,
        "DEVICES" => Rights::DEVICES,
        "AUTH" => Rights::AUTH,
        "LOG" => Rights::LOG,
        "ELEVATE" => Rights::ELEVATE,
        "NET_READ" => Rights::NET_READ,
        "NET_ADMIN" => Rights::NET_ADMIN,
        "BLUETOOTH_READ" => Rights::BLUETOOTH_READ,
        "BLUETOOTH_ADMIN" => Rights::BLUETOOTH_ADMIN,
        "SECRET_USE" => Rights::SECRET_USE,
        "NET_CONNECT" => Rights::NET_CONNECT,
        _ => return None,
    };
    Some(rights)
}
