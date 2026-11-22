# oc-observe

Inspect [OpenCode](https://opencode.ai) sessions in your browser.

`oc-observe` calls `opencode session export <id>`, turns the JSON into a
self-contained HTML page (styles and scripts embedded), and serves it on
`127.0.0.1`. With no arguments it opens an interactive picker with the sessions
of the current project.

## Install

```sh
cargo install --path .
```

## Usage

```sh
oc-observe                  # interactive picker (↑/↓ or j/k, Enter, a, q)
oc-observe <session-id>     # serve the session and open the browser
oc-observe <session-id> -o report.html   # write a standalone HTML file only
```

### Modes

| Command | Behaviour |
| --- | --- |
| `oc-observe` | TUI with the project's sessions: id, title and date. `a` toggles between this project and all projects, `Enter` serves the selected one. |
| `oc-observe <id>` | Export, render, start a local server and open the browser. |
| `oc-observe <id> -o file.html` | Write `file.html`. No server, no browser. |
| `oc-observe export.json` | Same as an id, using a previously exported JSON file. |
| `opencode session export <id> \| oc-observe -` | Same as an id, reading the export from stdin. |

### Options

```
-o, --output <FILE>  Write a standalone HTML file instead of starting the server
    --no-open        Don't open the page in the default browser
    --port <PORT>    Port for the local server (0 = random free port)
    --stdout         Print the HTML to stdout instead of serving or writing it
```

Ctrl+C stops the server.

## The page

- Session header: title, id, agent, model, project directory, start time, and
  session time (only assistant response time; idle gaps are excluded).
- Stats: message count (user prompts / assistant messages on separate lines),
  tool calls, cost, total input tokens split into uncached/cached with the cached
  percentage, and total output tokens with reasoning tokens. Token counts use
  compact units (K/M/B). Horizontal bars show user vs assistant messages,
  uncached vs cached input, and total output.
- User prompts rendered as Markdown, with attached files and mentions.
- Assistant messages with collapsible reasoning blocks and tool calls.
- Tool calls: input fields, output, error, exit code and completion status.
- Markdown rendering (tables, lists, code blocks…), with raw HTML stripped.
- Client-side filter (`/`), expand/collapse all, copy buttons for ids and code.
- Responsive layout, dark/light theme based on the OS preference, print styles.
- No external requests: the page is fully standalone.

## Notes

- Requires the `opencode` CLI in `PATH`.
- "All projects" mode reads the session index from OpenCode's local database
  (`opencode.db`) in read-only mode. Project mode uses `opencode session list`.
- Only data present in the export is shown; tool outputs are truncated at 120k
  characters per block to keep pages reasonable.
- The server binds to `127.0.0.1` only.

## License

MIT
