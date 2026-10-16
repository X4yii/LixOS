# Conectividad y buses de dispositivos

> **Tipo:** diseño · **Estado:** en implementación · **ADR:** 01, 05, 11, 12
> **Relacionado:** [Capabilities](../spec/capabilities.md) · [Almacenamiento](storage.md)

Define la ruta de integración para USB, redes Wi‑Fi/Ethernet y Bluetooth sobre x86_64 UEFI. La compatibilidad se declara por controlador y chipset; no existe un driver Wi‑Fi genérico que cubra hardware arbitrario.

## Objetivo

- USB: xHCI como controlador inicial; USB HID para teclado/ratón y almacenamiento masivo.
- Red: virtio-net como backend QEMU reproducible, seguido de chipsets Ethernet y Wi‑Fi con soporte documentado. La ruta usable requiere Ethernet, IPv4, ARP, DHCP, DNS, UDP y TCP.
- Bluetooth: HCI sobre USB y perfil HID para teclado/ratón en la primera etapa. Audio y BLE GATT quedan fuera de esa etapa.
- Hardware de validación disponible: Intel Wi‑Fi 6 AX201 (`8086:a0f0`, interfaz CNVio2 en Tiger Lake) y Bluetooth Intel AX201 (`8087:0026`, HCI USB). El AX201 no debe tratarse como un adaptador PCIe genérico; el soporte requiere la ruta CNVio2 de la plataforma y firmware compatible.
- Los drivers internos siguen el modelo monolítico aceptado en ADR‑01. La separación en userspace requiere primero implementar las capabilities de MMIO, DMA e IRQ especificadas en ADR‑11.

## Capas

1. PCI/ACPI identifica funciones, BARs, interrupciones y requisitos DMA.
2. Drivers de bus (xHCI y virtio) gestionan colas, DMA y eventos de hardware.
3. Drivers de clase/adaptador convierten transferencias en entrada, almacenamiento o tramas Ethernet.
4. Parsers y protocolos independientes del hardware residen en crates `no_std` probables desde host.
5. `netmgr`, `bluetoothd` y `authd` administran enlaces, perfiles y secretos en userspace. El ABI de sockets/canales se define antes de exponer la red a aplicaciones; ver [Gestores de red](network-managers.md) y [Autenticación](security-auth.md).

## Estado de implementación

- `kernel/src/drivers/pci.rs` detecta funciones PCI xHCI/red y enlaza virtio-net transitional (`1af4:1000`) cuando está presente. `kernel/src/drivers/virtio_net.rs` configura colas DMA legacy, hace polling RX/TX y completa DHCPDISCOVER/OFFER/REQUEST/ACK.
- `libs/lix-usb` valida límites de descriptores USB y clasifica interfaces HID y almacenamiento.
- `libs/lix-usb` también parsea descriptores de dispositivo/endpoints, localiza endpoints HID boot y decodifica informes boot de teclado/ratón a eventos físicos. `kernel/src/drivers/input.rs` ofrece una cola acotada; PS/2 publica keycodes HID físicos además de conservar la ruta TTY.
- `sys_input_open` (0x705) y `sys_read` exponen esa cola a procesos con un handle que posea `INPUT_READ`. `sys_usb_enumerate` (0x825) expone descriptores USB ya leídos a procesos con `ENUMERATE`. El ABI y la comprobación de rights existen, pero aún no hay un broker/bootstrap que conceda esas capabilities a procesos de userspace.
- `libs/lix-net` parsea Ethernet, ARP, IPv4, UDP, TCP y respuestas DNS A con validación de límites/checksums; también construye respuestas ARP y tramas IPv4/UDP con checksums. El kernel ahora procesa ARP/IPv4/UDP sobre la interfaz registrada, aprende vecinos ARP, responde solicitudes para su IPv4 y demultiplexa datagramas a sockets UDP.
- `libs/lix-wifi` parsea beacons/RSN y contiene primitivas propias de ath9k_htc: matriz USB, selección de endpoints HIF (incluidos configuración, interfaz y alternate setting), framing de streams y descarga de firmware por bloques. `kernel/src/drivers/wifi.rs` enlaza la detección xHCI con AR9271/AR7010 y registra endpoints. Rootfs incluye `htc_9271-1.4.0.fw` y su licencia; xHCI selecciona la configuración/interfaz HIF y envía el firmware al AR9271 por control OUT, seguido del comando de finalización. Si esa secuencia acaba, el estado pasa a `HtcStarting`; aún faltan transfer rings xHCI bulk/interrupt WLAN, HTC/WMI, asociación, supplicant y publicación de interfaz de red. AR7010 todavía no tiene firmware ni carga soportada. `netmgr` tiene backend inyectable, pero no backend Wi‑Fi enlazado. El AX201 CNVio2 (`8086:a0f0`) sigue sin driver.
- `libs/lix-bluetooth` valida y codifica comandos HCI, además de parsear eventos Command Complete/Status y datos ACL. `libs/lix-usb` reconoce endpoints USB de interfaz HCI (interrupt Event y bulk ACL), según la [especificación USB Transport del Bluetooth SIG](https://www.bluetooth.com/wp-content/uploads/Files/Specification/HTML/Core_v6.3/out/en/host-controller-interface/usb-transport-layer.html). xHCI ahora inspecciona los descriptores y registra la geometría de los endpoints encontrados, pero aún no configura sus transfer rings; no existe transporte de clase xHCI, máquina HCI conectada, pairing ni perfiles.
- Las decisiones de perfiles, claves guardadas y permisos se documentan en `docs/design/network-managers.md` y `docs/design/security-auth.md`; `netmgr`, `bluetoothd` y `authd` todavía no están conectados al arranque/IPC.
- `kernel/src/drivers/net.rs` registra interfaces, transmite por la primera interfaz, conserva hasta 32 tramas RX y datagramas por socket, aprende hasta 64 vecinos ARP y selecciona gateway/subred para TX. La caché ARP se aprende de tráfico entrante; si falta una entrada, se emite una solicitud y `sendto` devuelve `EAGAIN` para que el proceso reintente. `sys_socket_open/bind/sendto/recvfrom` (0x830–0x833) y wrappers Rust están definidos para UDP IPv4 no bloqueante. Abrir sockets exige `NET_CONNECT`; el handle resultante se atenúa con `READ/WRITE`. El bootstrap todavía no concede esta capability y no hay paso de handles por IPC, así que el acceso queda cerrado hasta integrar esa concesión explícita. No hay todavía TCP, IPv6, fragmentación, ICMP/ping general, DNS integrado como resolver, DHCP lease renewal ni semántica POSIX completa; tampoco se validó el flujo con QEMU.
- xHCI resetea puertos y ejecuta Enable Slot/Address Device. La validación QEMU actual detecta el teclado USB, pero falla al completar `GET_DESCRIPTOR(Device)` por EP0; por ello no están validados configuración, HID ni `sys_usb_enumerate` sobre hardware. Se corrigieron TRT, dirección e IOC del Status Stage (sin IOC el driver no recibía el Transfer Event); el arreglo aún debe validarse en QEMU/hardware.
- xHCI también sondea puertos raíz durante el loop de servicio e intenta enumerar conexiones/desconexiones. La enumeración está incompleta en QEMU como se describe arriba; no hay soporte para hubs, alternativas de interfaz, USB MSC, recuperación completa ni HID fuera del protocolo boot.
- virtio-net cubre la ruta QEMU legacy y DHCP hasta obtener IPv4. La pila IPv4/UDP y los syscalls de datagramas están implementados pero requieren validación QEMU end-to-end; TCP, resolver DNS y ABI POSIX siguen pendientes.
- No hay aún un driver Wi‑Fi de chipset (AX201 es el objetivo detectado, no compatibilidad declarada), supplicant WPA2/WPA3 ni cifrado persistente integrado en los perfiles. Tampoco hay transporte xHCI para Bluetooth HCI, pairing o perfil HID Bluetooth. Los crates de protocolo y el clasificador USB son solo cimientos de software, no drivers utilizables.

## Matriz Linux de familias Wi‑Fi

Esta tabla es un inventario de familias mantenidas en Linux para guiar la compatibilidad de LixOS; **no declara que LixOS soporte esos dispositivos**. Los IDs exactos deben importarse a la matriz de hardware desde las tablas PCI/USB/SDIO de cada familia y validarse uno por uno.

| Familia Linux | Bus / tipo MAC | Ejemplos representativos | Firmware |
| --- | --- | --- | --- |
| `iwlwifi` | PCIe y plataforma CNVio/CNVio2; firmware MAC | 3165/3168, 7260/7265, 8260/8265, 9260/9560, AX200/201/210/211/411, BE200/201/202 | Sí; AX201 requiere CNVio2 además del firmware |
| `ath5k`, `ath9k` | PCI/PCIe/AHB; SoftMAC | AR5xxx, AR9285/9287, AR9462, AR9565 y familias AR900x | Generalmente no para el MAC base |
| `ath9k_htc` | USB; radio con firmware | AR9271 y puentes AR7010 | Sí (`htc_9271.fw`, `htc_7010.fw`) |
| `ath10k`, `ath11k`, `ath12k` | Principalmente PCIe/AHB; firmware MAC | QCA988x/QCA6174; QCA6390/WCN6855/QCN9074; generaciones Wi‑Fi 7 | Sí |
| `mt76`, `mt7601u` | PCIe, USB, SDIO y SoC; mezcla SoftMAC/firmware | MT7601U, MT7610U/7612U, MT7663, MT7915/16, MT7921/22/25, MT7992/96 | Sí en varias familias |
| `rtl8xxxu`, `rtw88`, `rtw89` | USB y PCIe; generaciones N/AC/AX/BE | RTL8188/8192/8723 USB; RTL8821/8822 PCIe/USB; RTL8852 PCIe | Sí en varios modelos |
| `brcmfmac`, `brcmsmac`, `b43` | PCIe, USB, SDIO, SoC; FullMAC y SoftMAC | BCM4313/432xx/435x/43602; generaciones Broadcom antiguas | Normalmente sí; algunos requieren NVRAM por modelo/plataforma |
| `rt2x00` y legado (`ipw*`, `iwlegacy`) | PCI/PCIe/USB/PCMCIA | Ralink RT28xx/RT30xx; Intel PRO/Wireless 2100/2200 y 3945/4965 | Depende del chipset |

Referencias primarias de soporte Linux: [índice de drivers](https://wireless.docs.kernel.org/en/latest/en/users/drivers.html), [iwlwifi](https://wireless.docs.kernel.org/en/latest/en/users/drivers/iwlwifi.html), [Atheros](https://wireless.docs.kernel.org/en/latest/en/users/drivers/ath9k.html), [ath10k](https://wireless.docs.kernel.org/en/latest/en/users/drivers/ath10k.html), [ath11k](https://wireless.docs.kernel.org/en/latest/en/users/drivers/ath11k.html), [MediaTek](https://wireless.docs.kernel.org/en/latest/en/users/drivers/mediatek.html), [Realtek](https://wireless.docs.kernel.org/en/latest/en/users/drivers/rtl819x.html) y [Broadcom](https://wireless.docs.kernel.org/en/latest/en/users/drivers/brcm80211.html).

La implementación se organizará por transporte y familia, no por marca comercial. Antes de añadir un modelo a “compatible” se registrarán ID de bus, revisión, firmware/NVRAM y evidencia de scan, asociación WPA2 y tráfico IP. Los drivers Linux sirven como mapa de hardware y referencia de comportamiento; el código no se copia al kernel LixOS.

## Secuencia y aceptación

1. Validar la corrección IOC de `GET_DESCRIPTOR(Device)` en QEMU; después verificar enumeración, conexión/desconexión y transferencias HID boot antes de declarar soporte USB estable. La entrada PS/2 sí fue verificada con `sendkey` de QEMU hasta Vanish.
2. Validar en QEMU ARP/IPv4/UDP y `NET_CONNECT`; después integrar DNS resolver, TCP y wrappers POSIX según `docs/spec/porting-compat.md`.
3. Completar el sistema de handles/IPC y el bootstrap antes de arrancar `netmgr`, `bluetoothd` o `authd`.
4. Añadir transporte Bluetooth HCI sobre USB y perfil HID reutilizando la cola de entrada; después pairing/bóveda y GATT según política.
5. Implementar Wi‑Fi por chipset para AX201/CNVio2, primero inicialización de plataforma y firmware, luego TX/RX y scan/association; añadir WPA2-Personal CCMP. WPA3 requiere SAE y PMF completos. Si la documentación/protocolo CNVio2 no permite un driver mantenible, conservar la interfaz de chipset y declarar el AX201 no soportado hasta elegir un adaptador con protocolo documentado.
6. Completar API de red con DNS/TCP y routing avanzado; `netmgr` controla links y no implementa una pila distinta.
7. Validar cada familia de hardware y seguridad en QEMU/físico; publicar IDs, firmware, licencias y limitaciones. Para Bluetooth, validar específicamente `8087:0026` sobre xHCI.

Una función cuenta como soportada solo cuando pasa sus pruebas QEMU o físicas asociadas; detección PCI por sí sola no cuenta como soporte funcional.
