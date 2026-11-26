# Ecliven Code

Agente de codigo con IA en terminal construido en Rust.

## Caracteristicas base

- **Multi-proveedor unificado (`LlmProvider`)**:
  - APIs: Anthropic (Claude), OpenAI y compatibles (OpenRouter, DeepSeek, Groq), Ollama local
  - **CLIs externos conectados (`/connect`)**: `codex`, `agy`, `claude`, `opencode`, `gemini` y cualquier CLI personalizado (`/connect custom <cmd>`) sin necesitar API keys adicionales
- **Herramientas de codigo con sandbox al workspace**:
  - `read_file`, `write_file`, `edit_file` (con preview de diff unificado antes de escribir)
  - `list_dir`, `search_files` (glob), `grep_content` (regex)
  - `run_command` (ejecucion de comandos con timeout configurable)
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
- `/status`
- `/clear`
- `/help`
- `/quit`

## Verificacion

```bash
cargo test
cargo clippy --all-targets
```
