# rs-agent TODO — Features missing vs reference pi agent

## Priority: Foundation (start here)

- [ ] 1. Add `find` tool (glob-based file search)
- [ ] 2. Implement session persistence (save/load to disk)
- [ ] 10. Implement context compaction (auto + manual `/compact`)
- [ ] 12. Implement token counting and context overflow detection

## Session Management

- [ ] 3. Implement session resume (`-r` flag)
- [ ] 4. Implement session forking (`/fork`)
- [ ] 5. Implement session tree navigation (`/tree`)
- [ ] 6. Implement session naming (`--name`, `/name`)
- [ ] 7. Implement session export/import (HTML/JSONL)
- [ ] 8. Implement session sharing (GitHub gist)
- [ ] 9. Implement ephemeral mode (`--no-session`)

## Context & Compaction

- [ ] 11. Implement branch summarization

## Extension & Subagent System

- [ ] 13. Implement extension system (loader, tools, commands, UI, events)
- [ ] 14. Implement subagent spawning and orchestrator

## Interactive Features

- [ ] 15. Implement slash command system
- [ ] 16. Implement model switching UI and model cycling
- [ ] 17. Implement settings UI (`/settings`)
- [ ] 18. Implement OAuth login/logout
- [ ] 19. Implement prompt templates
- [ ] 20. Implement image paste support
- [ ] 21. Implement external editor (Ctrl+G)
- [ ] 22. Implement shell commands in editor (`!command`, `!!command`)
- [ ] 23. Implement file reference (`@` fuzzy search)
- [ ] 24. Implement message queue (steering/follow-up)

## Context Files

- [ ] 25. Implement context file discovery (AGENTS.md/CLAUDE.md)
- [ ] 26. Implement custom/append system prompt files

## Security

- [ ] 27. Implement project trust system
- [ ] 28. Implement sandbox mode for bash
- [ ] 29. Implement tool allowlist/denylist
- [ ] 30. Implement permission system

## Model Management

- [ ] 31. Implement model registry with pricing/context limits
- [ ] 32. Implement model resolver (pattern matching, provider prefix)
- [ ] 33. Implement thinking levels (off/minimal/low/medium/high/xhigh)

## TUI Enhancements

- [ ] 34. Implement TUI markdown rendering
- [ ] 35. Implement syntax highlighting in TUI
- [ ] 36. Implement image display in TUI
- [ ] 37. Implement dynamic border color (thinking level)
- [ ] 38. Implement footer with stats (tokens, cost, context, model)
- [ ] 39. Implement configurable keybindings
- [ ] 40. Implement theme system
- [ ] 41. Implement autocomplete (file/path/command)
- [ ] 42. Implement overlay UI system

## Output Modes

- [ ] 43. Implement JSON mode (`--mode json`)
- [ ] 44. Implement RPC mode (`--mode rpc`)

## Distribution & Operations

- [ ] 45. Implement binary distribution (Bun-like builds)
- [ ] 46. Implement self-update mechanism
- [ ] 47. Implement package manager for extensions
- [ ] 48. Implement telemetry system
