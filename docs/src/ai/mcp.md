# Model Context Protocol

Zed uses the [Model Context Protocol](https://modelcontextprotocol.io/) to interact with context servers.

> The Model Context Protocol (MCP) is an open protocol that enables seamless integration between LLM applications and external data sources and tools. Whether you're building an AI-powered IDE, enhancing a chat interface, or creating custom AI workflows, MCP provides a standardized way to connect LLMs with the context they need.

Check out the [Anthropic news post](https://www.anthropic.com/news/model-context-protocol) and the [Zed blog post](https://zed.dev/blog/mcp) for an introduction to MCP.

## MCP Servers as Extensions

Zed supports exposing MCP servers as extensions.
You can check which servers are currently available in a few ways: through [the Zed website](https://zed.dev/extensions?filter=context-servers) or directly through the app by running the `zed: extensions` action or by going to the Agent Panel's top-right menu and looking for "View Server Extensions".

In any case, here are some of the ones available:

- [Postgres](https://github.com/zed-extensions/postgres-context-server)
- [GitHub](https://github.com/LoamStudios/zed-mcp-server-github)
- [Puppeteer](https://github.com/zed-extensions/mcp-server-puppeteer)
- [BrowserTools](https://github.com/mirageN1349/browser-tools-context-server)
- [Brave Search](https://github.com/zed-extensions/mcp-server-brave-search)
- [Prisma](https://github.com/aqrln/prisma-mcp-zed)
- [Framelink Figma](https://github.com/LoamStudios/zed-mcp-server-figma)
- [Linear](https://github.com/LoamStudios/zed-mcp-server-linear)

If there's an existing MCP server you'd like to bring to Zed, check out the [context server extension docs](../extensions/context-servers.md) for how to make it available as an extension.

## Bring your own MCP server

You can bring your own MCP server by adding its configuration to your `settings.json` file (accessible via `zed: settings open json`). The structure for configuring context servers is under the `context_servers` key.

Here's an example demonstrating various configuration options:

```json
{
  "context_servers": {
    "my_calculator_server": {
      "command": {
        "path": "/path/to/my/calculator_server_executable",
        "args": ["--port", "8080"],
        "env": {"API_KEY_FOR_CALC": "secret_key"}
      },
      "settings": { // Server-specific settings passed to the MCP server
        "precision": "high"
      },
      // Zed-specific settings for this MCP server
      "zed_tool_confirmation": {
        // Optional: Default confirmation behavior for all tools from this server.
        // Defaults to true (requires confirmation) if this whole block or this specific key is omitted.
        "default_needs_confirmation": false,
        // Optional: Specific overrides for tool confirmation.
        "tools": {
          "add": false, // "add" tool from this server will not require confirmation
          "execute_complex_calculation": true // This tool will require confirmation
        }
      }
    },
    "another_server_default_confirm": {
      "command": { // Minimal command example
        "path": "another_mcp_server"
        // "args" and "env" are optional
      },
      "settings": {
        // Optional server-specific settings
      },
      "zed_tool_confirmation": {
        "default_needs_confirmation": true // All tools here will need confirmation unless overridden in the "tools" map below.
        // "tools": {} // No specific overrides in this example
      }
    }
  }
}
```

If you are interested in building your own MCP server, check out the [Model Context Protocol docs](https://modelcontextprotocol.io/introduction#get-started-with-mcp) to get started.

### Configuring Tool Confirmation

By default, when a Large Language Model (LLM) attempts to use a tool from a custom MCP server you've configured, Zed will prompt you for confirmation before the tool is executed. This is a security measure to ensure you're aware of the actions being performed.

You can customize this behavior using the `zed_tool_confirmation` object within your server's configuration block in `settings.json`. This object can contain two optional fields:

1.  **`default_needs_confirmation`** (boolean):
    *   If set to `true`, all tools provided by this MCP server will require user confirmation before execution, unless a specific tool is overridden in the `tools` map.
    *   If set to `false`, all tools from this server will *not* require confirmation by default, again, unless overridden for a specific tool.
    *   If this field is omitted entirely, or if the entire `zed_tool_confirmation` block is omitted, it defaults to `true` (requiring confirmation for all tools from that server).

2.  **`tools`** (map):
    *   This is an optional map where each key is a tool name (a string, exactly as the MCP server exposes it) and the value is a boolean.
    *   Set the value to `true` if that specific tool should require confirmation.
    *   Set the value to `false` if that specific tool should *not* require confirmation.
    *   This map allows you to override the behavior set by `default_needs_confirmation` (or the implicit default) on a per-tool basis.

**Global Override**:
Please note that all these confirmation settings are subject to a global Zed setting: `agent.always_allow_tool_actions`. If you set `agent.always_allow_tool_actions` to `true` in your main Zed `settings.json`, it will bypass *all* tool confirmations, regardless of the `zed_tool_confirmation` settings for individual MCP servers.

### Tool Availability

Once your MCP server is configured in Zed's settings and Zed successfully connects to it, any tools your server exposes via the Model Context Protocol's standard tool discovery mechanism will be registered with Zed's agent. If the agent is using an LLM that supports tool calling (like a properly configured Ollama model, or other integrated models), these tools can then be invoked by the LLM during its interactions.
