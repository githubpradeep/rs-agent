# MCP servers

rs-agent can attach **stdio MCP servers** configured in `~/.rs-agent/config.toml`
(or project `.rs-agent.toml`). Tools appear as `mcp__{server}__{tool}`.

```toml
[[mcp.servers]]
name = "filesystem"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
# enabled = false
# [mcp.servers.env]
# FOO = "bar"
```

On startup (TUI and `-p`), each enabled server is spawned, `initialize`d, and
`tools/list`ed. Mutating MCP tools still prompt for permission unless you use
`-a`/`--approve`. Tools with MCP `readOnlyHint: true` skip the permission prompt.
`--auto-mode` auto-approves built-in file tools (`write`/`edit` + reads) but still
prompts for bash/repl and non-readonly MCP tools.

HTTP/SSE MCP transports are not supported yet.
