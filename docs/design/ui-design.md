# Diseño de interfaz de usuario y tema

> **Tipo:** diseño · **Estado:** borrador · **ADR:** 15, 25
> **Relacionado:** [Gráficos](graphics.md) · [Construcción y ejecución](../guides/build.md)

Define la identidad visual del escritorio: paleta, tipografía, composición del panel y del launcher, y el framework gráfico de las aplicaciones.

LixOS emplea un entorno gráfico propio con aceleración por hardware. Al tratarse de un sistema construido desde cero, la interfaz no arrastra decisiones de diseño heredadas: el criterio es latencia baja, densidad de información y ausencia de ornamentación innecesaria.

## 1. Criterios de diseño

- **Latencia de percepción:** todo elemento visual se actualiza de forma inmediata. Las transiciones se limitan a 100-150 ms; no se introducen animaciones más largas.
- **Estabilidad de composición:** la arquitectura Zero-Copy con doble y triple buffering en VRAM elimina el parpadeo de los elementos.
- **Glassmorfismo funcional:** materiales opacos con desenfoque de GPU para las barras y los menús modales contextuales.

## 2. Paleta de Colores por Defecto (Lix Dark Theme)

Para facilitar la composición y priorizar las pantallas modernas y el ahorro energético (OLED), el sistema utiliza un **modo oscuro nativo**.

- **Fondo General (Background):** Gris espacial muy oscuro `#121212`.
- **Superficies (Paneles, Ventanas Activas):** `#1E1E1E` con ligeras transparencias (blur backend).
- **Acento Primario (Lix Blue):** `#007ACC` o un tono cyan neón sutil `#00A3FF` para botones activos, focos y barras de progreso.
- **Acento Secundario (Errores/Crítico):** `#E53935`.
- **Texto Primario:** `#F5F5F5` (Blanco suave, no #FFFFFF puro para reducir contraste agresivo).
- **Texto Secundario:** `#A0A0A0`.

## 3. Composición del Escritorio (`lix-shell`)

### 3.1 Panel Único
- **Posición:** Borde inferior de la pantalla.
- **Tamaño:** Compacto (ej. 40px de altura).
- **Lado Izquierdo:** Botón del Menú (Ícono de Lix), accesos rápidos fijados.
- **Centro:** Lista de ventanas abiertas con indicadores minimalistas (un punto debajo del ícono activo).
- **Lado Derecho (Tray):** Conexiones (cuando se implemente red), batería (ACPI), sonido, reloj.

### 3.2 Menú Lanzador (Launcher)
- Estilo "Overlay" que se despliega desde la parte inferior izquierda.
- Integra un buscador unificado muy veloz para invocar comandos locales o buscar paquetes de `PML`.
- Categorías automáticas generadas a partir de la instalación de paquetes.

## 4. Tipografía

- Fuente sin serif, moderna y altamente legible para la interfaz gráfica: **Inter** o **Roboto**.
- Para la terminal (`Vanish`) y los bloques de código: **JetBrains Mono** o **Fira Code**.
- Textos antialiased con _Subpixel Rendering_ asistido por GPU.

## 5. El Framework Gráfico Oficial: `LUIX`

Las aplicaciones para LixOS no usan GTK, Qt ni abstracciones genéricas multiplataforma. Utilizan **LUIX**, el framework oficial mantenido por el proyecto.

### 5.1 Arquitectura de LUIX (derivado de `iced`)
LUIX se deriva de **`iced`** y de sus dependencias (`taffy` para flexbox, `cosmic-text` para tipografía), reescrito para este sistema.
- **Sin código multiplataforma:** se elimina el código heredado de `winit`, Wayland, X11, macOS y Windows.
- **Integración directa con el kernel:** LUIX se comunica con el subsistema gráfico del kernel para el registro de ventanas y la asignación de planos de hardware, con Zero-Copy desde los _widgets_ hasta la VRAM y sin consumo de RAM para la composición.
- **Modelo reactivo:** arquitectura Elm (Estado, Vista, Actualización), con aplicaciones seguras, predecibles y testables.

### 5.2 Integración en el árbol del sistema
- **Motor de temas (Theme Engine)** basado en *tokens* (colores, espaciados, opacidad): las aplicaciones adoptan la identidad visual por defecto sin valores codificados, lo que permite cambiar de tema (claro/oscuro) en tiempo de ejecución.
- El binario final de las aplicaciones es mínimo, ya que el framework se optimiza junto al sistema y se enlaza con la _toolchain_ de LixOS.

## 6. Transiciones de Estado y Manejo de Errores Gráficos

- Si un proceso de usuario entra en pánico, la ventana se congela con un tinte rojo leve o un aviso ("_No responde_").
- El subsistema de ventanas del kernel opera con independencia de la carga o de los bloqueos en userspace; el cierre forzoso se realiza con `Ctrl + Shift + Q`.
- Transición de consola a GUI: el cambio entre TTY1 (texto) y TTY6 (GUI) se realiza sin destellos; el kernel prepara los búferes GOP en segundo plano antes del `_flip` final.

---

## 7. Implementación actual

Comportamiento observado en `libs/` y en los binarios de espacio de usuario. Los apartados siguientes describen hechos del código; no modifican la especificación de las secciones anteriores.

### 7.1 Estado de LUIX

`libs/lix-ui` **no existe en el árbol**: el crate fue retirado del repositorio y de la lista de miembros del workspace. En las versiones anteriores era un crate vacío (solo la atribución `#![no_std]`) y tampoco entonces había código de *widgets*, motor de temas ni integración con un subsistema gráfico.

- No es miembro del workspace y ningún `Cargo.toml` lo declara como dependencia de ruta.
- Ningún archivo `.rs` del árbol contiene `use lix_ui` ni `lix_ui::`.

### 7.2 `litui`

`libs/litui` es un submódulo git (`https://github.com/X4yii/Litui.git`) con tres crates: `litui-core` (0.1.2), `litui-widgets` (0.3.2) y `litui` (0.30.2), declarados como dependencias de ruta en el `Cargo.toml` de la raíz.

- `libs/litui/Cargo.toml` lo describe como «recorte del fork litui para LixOS: solo lo necesario para el TUI de userspace (ltop)».
- Su único consumidor es `userland/ltop` (`main.rs` y `tui_backend.rs`, 413 líneas en total), que usa `litui::Terminal`, los *widgets* `Block` / `Borders` / `Paragraph` / `Table`, y los módulos `layout`, `style` y `backend`.
- Es una librería de interfaz de **terminal**, no un toolkit gráfico; no dibuja sobre framebuffer ni sobre VRAM.

### 7.3 Escritorio

| Binario | Estado |
| --- | --- |
| `userland/lix-shell` | **retirado del árbol**: el crate ya no existe |
| `services/lix-compositor` | stub: una sola llamada a `println!` |

No existen el panel, el lanzador, el gestor de ventanas ni las transiciones descritas en las §3 y §6.
