# Gestores de Wi‑Fi y Bluetooth

> **Estado:** diseño · **Relacionado:** [Conectividad](connectivity.md) · [Autenticación](security-auth.md)

## Frontera entre drivers y servicios

Los drivers xHCI, Wi‑Fi y HCI son internos al kernel según ADR-01. Exponen objetos de dispositivo y transferencias acotadas; no interpretan contraseñas ni deciden políticas de usuario. Los servicios `netmgr` y `bluetoothd` corren en Ring 3 y reciben handles concretos del bootstrap. Hasta que existan IPC/canales y capacidades de dispositivo, no se conectan directamente a procesos de aplicación.

### `netmgr`

Administra varias interfaces y perfiles. Mantiene estados `Down → Scanning → Associating → Authenticating → Configuring → Online`, además de `Failed` y `Disconnecting`. Cada operación lleva un identificador para cancelar/reemplazar scans y asociaciones anteriores. La capa de enlace proporciona scan, asociación, desasociación y envío/recepción Ethernet; la política elige perfil por orden explícito, seguridad mínima, prioridad del usuario y estado del enlace.

El gestor no implementa WPA dentro del driver. Un supplicant verifica RSN/AKM/cipher negociados y obtiene credenciales de la bóveda solo al conectar. El camino IP común reutiliza Ethernet, DHCP, ARP, DNS, IPv4 y sockets sin distinguir si el enlace es Ethernet o Wi‑Fi.

El primer adaptador físico objetivo es Intel AX201 (`8086:a0f0`) en Tiger Lake. Requiere soporte de plataforma CNVio2 y carga de firmware; detectar la función PCI no cuenta como interfaz Wi‑Fi operativa. La primera seguridad implementada será WPA2-Personal/CCMP. WPA3-Personal debe permanecer rechazado hasta completar SAE y protección de tramas de gestión.

### `bluetoothd`

Administra controladores HCI USB, inicialización/reset, comandos, eventos y ACL; registra dispositivos con dirección aleatoria/identidad protegida y la política de bonding. HCI USB usa control para comandos, interrupt-IN para eventos y bulk-IN/OUT para ACL; el transporte SCO queda fuera del primer hito. El host controla el pairing y guarda LTK/IRK en la bóveda; los drivers solo transportan paquetes.

El primer perfil es HID de teclado/ratón. Sus eventos alimentan `drivers::input`; el perfil no escribe directamente en TTY. La traducción a caracteres sigue en la capa de consola/userspace. GATT, audio y otros perfiles quedan deshabilitados hasta completar su política y pruebas.

## Reglas de seguridad y operación

- SSID, nombres y direcciones descubiertas son datos no confiables y se validan con longitudes máximas.
- Nunca escribir PSK, PMK, LTK, IRK o contraseñas en logs, klog, argumentos de proceso ni variables de entorno.
- Toda conexión/reconexión, rechazo de seguridad y desconexión produce eventos sin secretos.
- La eliminación de perfil revoca credenciales y cancela conexiones asociadas.
- No auto-conectar a una red abierta ni rebajar WPA2/WPA3 para satisfacer un perfil seguro.
- Los drivers declaran IDs PCI/USB exactos, firmware, DMA, interrupciones, energía y limitaciones antes de figurar como compatibles.
