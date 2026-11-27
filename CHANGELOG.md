# Changelog

Formato basado en [Keep a Changelog](https://keepachangelog.com/es-ES/1.1.0/). El proyecto sigue [versionado semántico](https://semver.org/lang/es/).

## [0.4.2] - 2026-11-27

### Añadido

- Herramienta `ask_user`: el agente puede preguntar cuando tiene dudas con opciones de selección única o múltiple y un campo de respuesta libre. En CLI pregunta por consola; en Swarm autónomo continúa sin bloquearse.
- Cola de mensajes de seguimiento: puedes escribir y pulsar `Enter` mientras el agente trabaja para encolar mensajes que se ejecutan en orden al terminar el turno, con barra de cola, `[Clear queue]` y `/queue [clear|pop]`.
- Soporte de mouse en la terminal: clic en opciones de preguntas, botones de aprobación (`Approve` / `Deny`), pestañas de la barra superior, selector de modelos, Swarm Hub, asistente inicial y `[Clear queue]`, además de rueda del mouse en listas e historial.

## [0.4.1] - 2026-11-27

### Añadido

- Referencias `conector/modelo` en `/model` (por ejemplo `codex/test-model` u `opencode/test-model`), que cambian conector y modelo a la vez. Los modelos se muestran como `conector/modelo` en confirmaciones, `/status` y Swarm.
- Router adaptativo: aprende la tasa de éxito, la latencia, los tokens y el costo reales de cada modelo, y salta los modelos en enfriamiento, poco fiables o lentos (`max_latency_ms`).
- Fallback ante 429, errores 5xx o fallos de red, y escalamiento a un modelo mayor si se excede el contexto. La cadena es: modelo elegido, tiers superiores, `fallbacks` y modelo activo.
- Presupuestos `max_session_usd` y `max_daily_usd` con precios en `[router.prices]`: al 80 % baja un tier y al 100 % solo usa modelos sin precio.
- `/router` muestra estadísticas por modelo, gasto y enfriamientos; `/router reset` borra lo aprendido.

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

[0.4.2]: https://github.com/ikiresearch/ecliven-code/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/ikiresearch/ecliven-code/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/ikiresearch/ecliven-code/compare/v0.3.0...v0.4.0
