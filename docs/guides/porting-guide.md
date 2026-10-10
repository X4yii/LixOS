# Guía de portabilidad

> **Tipo:** guía · **Estado:** borrador · **ADR:** 22
> **Relacionado:** [Portabilidad de software](../spec/porting-compat.md)

Describe cómo portar aplicaciones de terceros escritas en C/C++ a LixOS, y la estructura y el flujo de trabajo del directorio `ports/`.

## 1. Filosofía de Portabilidad

LixOS no es Linux. El objetivo es mantener el código nativo libre de legado y delegar toda la lógica de emulación POSIX a la biblioteca de C en espacio de usuario (`sys-libc`). LixOS provee un compilador cruzado (`x86_64-lix-gcc`) o un target de LLVM integrado.

## 2. Estructura del Directorio `ports/`

El directorio raíz de código fuente de LixOS cuenta con la carpeta `ports/`, que contiene scripts de descarga y aplicación de parches para librerías y aplicaciones esenciales:

```text
LixOS/
├── kernel/
├── sys-libc/
├── tools/
└── ports/
    ├── lua-5.4/
    │   ├── build.sh
    │   └── patches/
    │       └── 0001-lixos-compat-signal.patch
    ├── sqlite/
    ├── zlib/
    └── busybox-lite/
```

Cada proyecto dentro de `ports/` contiene:
1. `build.sh`: El script que descarga la fuente de un _tarball_ conocido, verifica su sha256, aplica los parches cruzados y corre `make` o `cmake` configurando el toolchain hacia Lix.
2. `patches/`: Directorio de archivos diff unificados para las partes del código que exigen adaptación.

**Meta T2 (ADR-22):** La mediana de tamaño de los parches por proyecto en la versión 1.0 debe ser ≤ 20 líneas.

## 3. Uso del Toolchain (Lix SDK)

Para compilar un programa para LixOS manualmente:

```bash
# Variables de entorno exportadas por el SDK
export CC="x86_64-lix-gcc"
export CXX="x86_64-lix-g++"
export AR="x86_64-lix-ar"
export CFLAGS="-O2 -static" # v1.0 requiere enlazado estático

./configure --host=x86_64-lix --prefix=/usr
make
```

### 3.1 Parchar el sistema de compilación
En los sistemas modernos tipo `autoconf`, LixOS probablemente no sea reconocido por `config.sub` ni `config.guess`.
La estrategia más sencilla es pasar `--host=x86_64-lix` y utilizar un archivo `config.sub` parcheado que identifique al OS.

## 4. Solución de Problemas Comunes (Gotchas)

Al portar software, es habitual toparse con las siguientes fricciones arquitectónicas de LixOS:

- **`fork()` puro:** Si la aplicación hace `fork()` y el hijo y el padre divergen asíncronamente haciendo trabajo intensivo de RAM: **Fallará**. Cambia la llamada a `posix_spawn()` o usa hilos (`pthreads`).
- **Punteros a Funciones en Memoria Compartida:** Las protecciones W^X (Write XOR Execute) de LixOS son estrictas. La asignación de memoria jamás permite páginas escribibles y ejecutables al mismo tiempo. Un JIT Compiler requerirá llamadas nativas de OS para hacer `mprotect` con mitigaciones.
- **Rutas "Hardcodeadas":** Software que lee `/etc/passwd` directamente funcionará, pero software que espera interactuar con `/sys` o `/proc` fallará porque LixOS no provee esos VFS por defecto.
- **Red Dinámica:** Cualquier programa que cargue plugins C compartidos `.so` vía `dlopen()` requerirá adaptación, ya que en la v1.0, LixOS impone el enlazado estático de los binarios ELF.

## 5. El Proceso de Aceptación a `PML`
Una vez que el proyecto compila, se empaqueta en el formato Zstd nativo:

```bash
# Se estructura el layout de instalación:
mkdir -p build_root/bin
cp mi_app build_root/bin/
# Se crea el paquete PML:
pml-pack build_root/ metadata.toml my_app-1.0-x86_64.pml
```
El paquete `.pml` resultante es compatible para instalación remota o local en cualquier entorno de LixOS.

---

## 6. Implementación actual

Comportamiento observado en la raíz del repositorio y en `pkg/pml`. Los apartados siguientes describen hechos del código; no modifican la especificación de las secciones anteriores.

| Elemento de las §2–§5 | Estado |
| --- | --- |
| Directorio `ports/` (§2) | no existe en la raíz |
| `build.sh` y `patches/` por proyecto (§2) | sin correspondencia |
| Toolchain «Lix SDK» (§3) | no hay binarios ni scripts de SDK en el árbol |
| Macros `__lix__`, `__unix__`, `_POSIX_VERSION` (§4) | no se definen en ningún `RUSTFLAGS` de `xtask`; `USER_RUSTFLAGS` solo fija `relocation-model`, `force-frame-pointers`, `link-arg=-Tuserland/linker.ld`, `nostdlib` y `zmax-page-size` |
| Aceptación en PML (§5) | `pkg/pml/src/main.rs` tiene 2 líneas: `println!("PML v0.0.1 (Program Manager Linx)")` |

Directorios presentes en la raíz: `abi/`, `docs/`, `kernel/`, `libs/`, `pkg/`, `rootfs/`, `services/`, `tools/`, `userland/`, `xtask/`.
