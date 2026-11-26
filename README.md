# Ecliven Code

Agente de codigo con IA en terminal construido en Rust.

## Caracteristicas base

- **Multi-proveedor unificado (`LlmProvider`)**:
  - APIs: Anthropic (Claude), OpenAI y compatibles (OpenRouter, DeepSeek, Groq), Ollama local
  - **CLIs externos conectados (`/connect`)**: `codex`, `agy`, `claude`, `opencode`, `gemini` y cualquier CLI personalizado (`/connect custom <cmd>`) sin necesitar API keys adicionales
- **Herramientas de codigo con sandbox al workspace**:
  - `read_file`, `write_file`, `edit_file` (con preview de diff unificado antes de escribir)
  - `list_dir`, `search_files` (glob), `grep_content` (regex)
  - `apply_patch` (edita, crea, mueve y borra varios archivos en un solo parche con un formato propio de bloques `*** Begin Patch`)
  - `run_command` (ejecucion de comandos con timeout configurable y sandbox del sistema operativo)
- **Instrucciones de proyecto (`AGENTS.md`)**: se cargan `~/.config/ecliven-code/AGENTS.md` y un `AGENTS.md` (o `AGENTS.override.md` / `CLAUDE.md`) por carpeta, desde la raiz del repo hasta el workspace (max. 32 KiB)
- **Sesiones guardadas (JSONL)** en `<data dir>/ecliven-code/sessions/` (o `ECLIVEN_SESSIONS_DIR`): `ecliven sessions`, `ecliven resume [id]`, `--resume <id|last>`, `/sessions`, `/resume`
- **Compactacion de contexto**: automatica al llegar a `auto_compact_percent` (80% por defecto) y manual con `/compact`
- **Checkpoints**: antes de cada edicion se guarda el estado de los archivos; `/undo` revierte el ultimo turno y `/checkpoints` los lista
- **Cliente MCP** (stdio y HTTP streamable): las herramientas aparecen como `mcp__<servidor>__<tool>`; `ecliven mcp` o `/mcp` las lista
- **Politicas de aprobacion**: `ask` (default), `auto-edit`, `auto-all`
- **Modo Swarm con dos apartados aislados**:
  - **Swarm: Code (Best-of-3 -> CHIEF)** (`--swarm code` / `/swarm code`): 3 codificadores independientes (`[CLEAN]`, `[PERF]`, `[ROBUST]`) resuelven la misma tarea en paralelo y un Agente Mayor (`[CHIEF]`) evalua y combina lo mejor de cada uno en una sola solucion optima.
  - **Swarm: Security (2 modos defensivos)** (`--swarm security-defense` / `--swarm security-threat`):
    - **Defensa (Codigo/Carpeta)** (`/swarm security defense [carpeta]`): audita una carpeta o codigo fuente con `[SURFACE]` + `[AUDIT]` (OWASP/CWE) + `[PATCH]` -> `[DEFENSE LEAD]`.
    - **Threat Model & Hardening** (`/swarm security threat [carpeta]`): modelado `[STRIDE]` + `[CONFIG]` (dependencias/CI) + `[BLUETEAM]` -> `[SEC ARCHITECT]`.
  - Pestanas separadas (`F2`), selector **Swarm Hub** (`F3` o `/swarm`), cambio de sub-modo de seguridad (`F4`) y foco por carpeta (`/scope <carpeta>` o `--scope <carpeta>`).
- **TUI en Ratatui con atajos estandar (`Ctrl+tecla`)** y modo headless (`--prompt`)

## Instalacion rapida

### Windows (PowerShell)

```powershell
irm https://ecliven.com/code/install.ps1 | iex
```

Una vez instalado, solo escribe en cualquier terminal:

```powershell
ecliven
```

Para actualizar rapidamente a la ultima version:

```powershell
ecliven update
```

*(Tambien te avisa automaticamente al abrir la TUI cuando hay una nueva version y puedes ejecutar `/update` desde el chat.)*

Para desinstalar:

```powershell
$env:ECLIVEN_UNINSTALL = "1"; irm https://ecliven.com/code/install.ps1 | iex
```

### macOS / Linux

```sh
curl -fsSL https://ecliven.com/code/install.sh | sh
```

### Como funciona

`https://ecliven.com/code/install.ps1` y `https://ecliven.com/code/install.sh` redirigen al instalador del ultimo release en GitHub. `https://ecliven.com/code/install` elige solo: PowerShell recibe `install.ps1` y `curl`/`wget` reciben `install.sh`.

Si el dominio no responde, usa GitHub directamente:

```text
https://github.com/ikiresearch/ecliven-code/releases/latest/download/install.ps1
https://github.com/ikiresearch/ecliven-code/releases/latest/download/install.sh
```

`install.ps1` descarga los binarios compilados y valida su `SHA256SUMS.txt` automaticamente desde GitHub Releases (`ikiresearch/ecliven-code`).

## Configuracion rapida (desarrollo local)

1. Copia `.env.example` a `.env` y agrega tu API key (o usa Ollama local):

```bash
cp .env.example .env
```

2. Ejecuta en modo interactivo (TUI):

```bash
cargo run
```

3. O en modo de un solo turno (headless):

```bash
cargo run -- --provider ollama --model qwen2.5-coder:14b --prompt "Resume la estructura del proyecto"
```

## Atajos de teclado

| Atajo | Accion |
|---|---|
| `Enter` | Enviar prompt |
| `Ctrl+J` / `Shift+Enter` | Insertar salto de linea |
| `Ctrl+U` | Limpiar caja de entrada |
| `Ctrl+L` | Limpiar historial de conversacion |
| `PgUp` / `PgDn` / Rueda | Hacer scroll en la conversacion |
| `Up` / `Down` | Navegar historial de prompts |
| `F2` | Cambiar de pestana (`Chat` -> `Swarm: Code` -> `Swarm: Security`) |
| `F3` | Abrir Swarm Hub (selector separado de Codigo y Ciberseguridad) |
| `F4` | Alternar sub-modo de Seguridad (`Defensa` <-> `Threat Model`) |
| `F5` | Instalar actualizacion disponible inmediatamente |
| `F6` | Abrir selector de carpeta de trabajo |
| `Ctrl+P` / `F7` | Abrir selector interactivo de modelos con buscador |
| `F1` | Mostrar/ocultar ayuda |
| `Ctrl+Q` | Salir |

## Comandos slash en el chat

- `/connect` | `/connect codex|agy|claude|opencode|gemini|custom <cmd>`
- `/models` / `/model` (abre el selector automatico de modelos)
- `/swarm` | `/swarm code [carpeta]` | `/swarm security defense|threat [carpeta]` | `/swarm off`
- `/scope <carpeta|off>`
- `/setup` | `/folder` | `/telemetry <on|off>`
- `/update [check]`
- `/provider <nombre>`
- `/model <nombre-del-modelo>`
- `/approval <ask|auto-edit|auto-all>`
- `/sandbox [read-only|workspace-write|danger-full-access]`
- `/compact`
- `/undo` (alias `/revert`) | `/checkpoints`
- `/sessions` | `/resume [id|last]`
- `/mcp`
- `/router [on|off]` | `/saver [on|off]` | `/swarm-models`
- `/memory [search <q>|add [--user] <texto>|forget <id>|confirm <id>|stale]`
- `/index [rebuild]` | `/tasks` | `/goal [<texto>|done|clear]` | `/workflow [on|off]`
- `/status`
- `/clear`
- `/help`
- `/quit`

## Sandbox de comandos

`run_command` corre dentro de un sandbox del sistema operativo:

| Modo | Escritura permitida |
|---|---|
| `workspace-write` (defecto) | workspace, `/tmp`, `/dev` y `sandbox_writable_roots` |
| `read-only` | solo `/tmp` y `/dev` |
| `danger-full-access` | todo (sin sandbox) |

- Linux: Landlock (kernel 5.13+; si el kernel no lo soporta, avisa y corre sin sandbox). `sandbox_network = false` bloquea TCP (kernel 6.7+).
- macOS: Seatbelt (`sandbox-exec`).
- Windows: aun sin sandbox; el resultado del comando lo indica.

Se cambia con `--sandbox <modo>`, `/sandbox <modo>` o en `.ecliven.toml`.

## Swarm con varios laboratorios, model router y modo ahorro

Los modelos se indican como `proveedor/modelo` (por ejemplo `anthropic/test-model`, `openrouter/vendor/test-model`) y usan las API keys ya configuradas.

- **Swarm multi-modelo:** `[swarm].workers` asigna un modelo a cada especialista por turnos, y `lead` el modelo que sintetiza. Cada agente muestra su modelo en la Swarm Hub. Sin configuracion se usa el modelo activo.
- **Model router:** con `[router].enabled = true`, cada turno se clasifica sin llamar a ningun modelo (dificultad `simple`/`standard`/`complex` y especialidad: security, frontend, backend, data, tests, docs, devops). Se usa el modelo de la especialidad si existe; si no, `cheap`, `standard` o `strong` segun la dificultad. El chat muestra que modelo eligio y por que.
- **Modo ahorro** (`--saver`, `/saver on` o `saver_mode = true`): baja un nivel de modelo (o usa `cheap` si el router esta apagado), limita la respuesta a 2048 tokens, recorta a 4000 caracteres la salida de herramientas que queda en el historial, compacta al 60 % y pide al modelo respuestas cortas y lecturas minimas. En Swarm, los especialistas hacen como maximo 4 iteraciones y usan `cheap` si no hay `[swarm].workers`.

## Memoria y conocimiento del proyecto

Ecliven guarda el conocimiento de cada proyecto en una base SQLite fuera del repositorio
(`<datos-del-usuario>/ecliven-code/knowledge/`, o `ECLIVEN_KNOWLEDGE_DIR`). Las memorias del usuario
van en `user.db` y se comparten entre proyectos.

**Capas:** instrucciones (`AGENTS.md`), trabajo activo (objetivo y plan), sesiones y tareas
(un registro por turno, sin llamar al modelo), conocimiento del proyecto (memorias con procedencia)
e indice del codigo.

**Vistas del repositorio:**
- indice textual (FTS5 con ranking BM25) por fragmentos de codigo;
- simbolos y relaciones (definiciones e imports) con un analizador ligero por lenguaje;
- busqueda semantica opcional (apagada por defecto y nunca en modo ahorro);
- estado de Git (rama y archivos modificados).

**Frescura:** cada archivo indexado guarda ruta, hash, lenguaje y version del analizador. Antes de cada
consulta se comparan tamano y fecha, y si cambian, el hash; solo se reanaliza lo que cambio. Las memorias
que citan un archivo modificado o borrado quedan marcadas `POSSIBLY STALE`; los renombrados se detectan por
hash y conservan sus memorias. Los embeddings se guardan por hash del fragmento, asi que solo se generan
para contenido nuevo.

**Contexto barato:** cada mensaje lleva un bloque `<project_context>` de 1500 tokens como maximo (600 en
modo ahorro) con la tarea activa, Git, memorias relevantes y referencias `ruta:lineas`, nunca archivos enteros.

**Herramientas del agente:** `code_search`, `find_symbol`, `memory_search`, `memory_save` y `update_plan`
(sin aprobacion: no modifican el repositorio). En la CLI: `ecliven index [--rebuild]`.

**Workflow opcional** (`/workflow on`): recuperar contexto, inspeccionar archivos, planificar, aplicar
cambios, verificar, revisar el diff, informar y guardar memoria util.

## Configuracion nueva en `.ecliven.toml`

```toml
sandbox_mode = "workspace-write"   # read-only | workspace-write | danger-full-access
sandbox_network = true
sandbox_writable_roots = ["/home/me/.cargo"]
auto_compact_percent = 80          # 0 desactiva la compactacion automatica
# context_window = 128000          # sobrescribe la ventana del proveedor
saver_mode = false

[swarm]
workers = ["anthropic/test-model-a", "openai/test-model-b", "openrouter/vendor/test-model-c"]
lead = "anthropic/test-model-lead"

[router]
enabled = true
cheap = "ollama/test-small"
standard = "openai/test-medium"
strong = "anthropic/test-large"

[router.specialties]
security = "anthropic/test-large"
frontend = "openrouter/vendor/test-frontend"

[mcp_servers.docs]
command = "npx"
args = ["-y", "@example/mcp-server"]
env = { EXAMPLE_TOKEN = "..." }

[knowledge]
enabled = true                     # brief por turno y herramientas de conocimiento
brief_tokens = 1500                # en modo ahorro el maximo es 600
workflow = false
# embeddings = "ollama/nomic-embed-text"   # opcional: busqueda semantica

[mcp_servers.remoto]
url = "https://example.com/mcp"
headers = { Authorization = "Bearer ..." }
trusted = true                     # sin pedir aprobacion (por defecto se pide como run_command)
```

## Verificacion

```bash
cargo test
cargo clippy --all-targets
```
