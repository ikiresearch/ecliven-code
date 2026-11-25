# Ecliven Code

Agente de codigo con IA en terminal construido en Rust.

## Caracteristicas base

- **Multi-proveedor unificado (`LlmProvider`)**:
  - Anthropic (Claude Messages API con streaming SSE y `tool_use`)
  - OpenAI y compatibles (OpenAI, OpenRouter, DeepSeek, Groq)
  - Ollama local (`/api/chat` con soporte de herramientas)
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
irm https://github.com/ikiresearch/ecliven-code/releases/latest/download/install.ps1 | iex
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
$env:ECLIVEN_UNINSTALL = "1"; irm https://github.com/ikiresearch/ecliven-code/releases/latest/download/install.ps1 | iex
```

### macOS / Linux

```sh
curl -fsSL https://github.com/ikiresearch/ecliven-code/releases/latest/download/install.sh | sh
```

### Dominio propio (opcional)

Si quieres distribuirlo con tu propio dominio (ej. `irm https://tu-dominio.com/install.ps1 | iex`), crea una redireccion `302` o sirve `install.ps1` apuntando a:

```text
https://github.com/ikiresearch/ecliven-code/releases/latest/download/install.ps1
```

`install.ps1` ya descarga los binarios compilados y valida su `SHA256SUMS.txt` automaticamente desde GitHub Releases (`ikiresearch/ecliven-code`).

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
| `F1` | Mostrar/ocultar ayuda |
| `Ctrl+Q` | Salir |

## Comandos slash en el chat

- `/swarm` (abre el Swarm Hub)
- `/swarm code` / `/swarm security` / `/swarm off`
- `/provider <anthropic|openai|openrouter|ollama>`
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
