# rs-agent

Minimalist AI agent toolkit. Unified LLM API, agent loop, TUI, and coding agent CLI.

## Features

- **Multiple LLM providers**: OpenAI, Anthropic, opencode.ai, and opencode CLI bridge
- **Agent loop**: Iterative tool-use execution with streaming responses
- **TUI**: Terminal UI with insert/normal modes, word-wrapped message display
- **Tools**: `read`, `write`, `edit`, `bash`, `grep`, `ls`
- **opencode CLI bridge**: Uses `opencode run` as a model backend with `<tool_call>` JSON protocol

## Usage

### TUI mode (default)

```sh
cargo run -- --provider openai
cargo run -- --provider anthropic
cargo run -- --provider opencode-cli
```

### One-shot mode

```sh
cargo run -- --provider opencode-cli -p "make a snake game in python"
```

### Options

| Flag | Description |
|---|---|
| `--provider` | LLM provider: `openai`, `anthropic`, `opencode`, `opencode-cli` |
| `--model` | Model name override |
| `--api-key` | API key |
| `--api-key-env` | Environment variable name for API key |
| `--timeout` | Request timeout in seconds (default: 300) |
| `-p, --prompt` | One-shot prompt (non-interactive) |
| `--list-models` | List available models for the provider |

## Providers

### opencode-cli bridge

Uses the `opencode` CLI as a model backend. The bridge:
- Creates an agent config that denies all opencode-native tools
- Communicates tool calls via `<tool_call>` JSON blocks in the text stream
- Passes tool results back to the model for subsequent turns
- Falls back to bare JSON `{"name":"...","arguments":{...}}` detection when `<tool_call>` tags are omitted

```sh
cargo run -- --provider opencode-cli
```

Requires `opencode` CLI on `PATH`.
