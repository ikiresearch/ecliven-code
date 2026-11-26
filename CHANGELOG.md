# Changelog

Formato basado en [Keep a Changelog](https://keepachangelog.com/es-ES/1.1.0/). El proyecto sigue [versionado semántico](https://semver.org/lang/es/).

## [Sin publicar]

### Corregido

- Linux: nuevo binario estático (`x86_64-unknown-linux-musl`) que funciona en cualquier distribución, sin depender de la versión de glibc. `install.sh` lo usa por defecto y, en releases anteriores, vuelve al binario glibc. Este último se compila ahora en Ubuntu 22.04 (glibc 2.35).

## [0.4.0] - 2026-11-26

### Añadido

- Instrucciones de proyecto (`AGENTS.md` global, de la raíz y de subcarpetas).
- Sesiones guardadas en JSONL: `ecliven sessions`, `ecliven resume`, `/sessions`, `/resume`.
- Compactación de contexto automática y con `/compact`.
- Checkpoints persistentes antes de cada edición, con `/undo`.
- Herramienta `apply_patch` para cambios en varios archivos.
- Cliente MCP (stdio y HTTP).
- Sandbox de comandos: Landlock en Linux y Seatbelt en macOS, con los modos `read-only`, `workspace-write` y `danger-full-access`.
- Swarm con modelos de distintos proveedores, model router por dificultad, coste y especialidad, y modo ahorro.
- Memoria por capas y conocimiento del proyecto: índice incremental con SQLite y FTS5, símbolos y relaciones, búsqueda semántica opcional, historial de tareas y un resumen de contexto con presupuesto de tokens. Comandos `/memory`, `/index`, `/tasks`, `/goal` y `/workflow`.
- Workspace Trust: un `.ecliven.toml` de un repositorio desconocido no activa sus ajustes ejecutables hasta que lo autorices (`/trust`, `ecliven trust`, `--trust-workspace`).
- Nuevos modos de Swarm: Competitive (candidatos aislados y ganador elegido por tests), Collaborative (un agente por área) y Red Eclipse (equipo rojo contra equipo azul, con rondas verificadas).
- Prompt caching para los proveedores por API (`prompt_cache = "5m" | "1h" | "off"`). `/status` muestra los tokens servidos desde la caché.
- Licencia dual MIT OR Apache-2.0, `SECURITY.md`, guía de contribución, código de conducta y plantillas.

### Cambiado

- Política de seguridad central: `read-only` bloquea todas las herramientas de escritura y los MCP no confiables. En Windows, sin sandbox del sistema, cada comando pide aprobación.
- Escrituras seguras frente a symlinks: se resuelven los ancestros, se rechazan los symlinks rotos y las escrituras en `.git`, y se escribe de forma atómica.
- Los backends CLI externos tienen timeout (`cli_timeout_secs`), se termina todo su árbol de procesos y sus llamadas a herramientas se recuperan de forma estructurada.
- Swarm Competitive parte de una instantánea inmutable, solo aplica un ganador que pasó la verificación y protege los tests frente a los candidatos.
- CI con acciones fijadas por SHA, permisos mínimos, `cargo deny`, revisión de dependencias y atestaciones de procedencia en los releases.

### Seguridad

- `rusqlite` 0.41: el SQLite incluido corrige RUSTSEC-2026-0459.
- Las sesiones guardadas informan los errores de escritura en lugar de ignorarlos.
- Corregido el truncado de texto UTF-8 en la salida de herramientas.

[0.4.0]: https://github.com/ikiresearch/ecliven-code/compare/v0.3.0...v0.4.0
