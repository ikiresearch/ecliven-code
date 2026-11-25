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
- **TUI en Ratatui con atajos estandar (`Ctrl+tecla`)** y modo headless (`--prompt`)

## Instalacion rapida

### Windows (PowerShell)

```powershell
irm https://ecliven.com/install.ps1 | iex
```

Una vez instalado, solo escribe en cualquier terminal:

```powershell
ecliven
```

Para desinstalar:

```powershell
$env:ECLIVEN_UNINSTALL = "1"; irm https://ecliven.com/install.ps1 | iex
```

### macOS / Linux

```sh
curl -fsSL https://ecliven.com/install.sh | sh
```

### Como funciona

`https://ecliven.com/install.ps1` y `https://ecliven.com/install.sh` redirigen al instalador del ultimo release en GitHub. `https://ecliven.com/install` elige solo: PowerShell recibe `install.ps1` y `curl`/`wget` reciben `install.sh`.

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
| `F1` | Mostrar/ocultar ayuda |
| `Ctrl+Q` | Salir |

## Comandos slash en el chat

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
