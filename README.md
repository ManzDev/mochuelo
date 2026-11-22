# mochuelo

Inspect [OpenCode](https://opencode.ai) sessions in your browser.

`mochuelo` calls `opencode session export <id>`, turns the JSON into a
self-contained HTML page (styles and scripts embedded), and serves it on
`127.0.0.1`. With no arguments it opens an interactive picker with the sessions
of the current project.

## Install

```sh
cargo install --path .
```

## Usage

```sh
mochuelo                  # interactive picker (↑/↓ or j/k, Enter, a, q)
mochuelo <session-id>     # serve the session and open the browser
mochuelo <session-id> -o report.html   # write a standalone HTML file only
```

### Modes

| Command | Behaviour |
| --- | --- |
| `mochuelo` | TUI with the project's sessions: id, title and date. `a` toggles between this project and all projects, `Enter` serves the selected one. |
| `mochuelo <id>` | Export, render, start a local server and open the browser. |
| `mochuelo <id> -o file.html` | Write `file.html`. No server, no browser. |
| `mochuelo export.json` | Same as an id, using a previously exported JSON file. |
| `opencode session export <id> \| mochuelo -` | Same as an id, reading the export from stdin. |

### Options

```
-o, --output <FILE>  Write a standalone HTML file instead of starting the server
    --no-open        Don't open the page in the default browser
    --port <PORT>    Port for the local server (0 = random free port)
    --stdout         Print the HTML to stdout instead of serving or writing it
```

Ctrl+C stops the server.

## The page

- Session header: title, id, model, project directory, start time, session time
  (only assistant response time; idle gaps are excluded), and the mochuelo
  mascot embedded as a WebP data URI (no external image reference).
- Stats: message count (user prompts / assistant messages on separate lines),
  tool calls with a per-tool count list, and a token fieldset with total/uncached
  input (value shows total / uncached, hints show cached percentage then
  uncached) and total output (total includes reasoning; hints show visible
  output and reasoning). Token counts use compact units (K/M/B). Horizontal bars
  show user vs assistant messages, cached vs uncached input, and output.
  Stat values use Science Gothic (Google Fonts, with system fallback).
- Cost card: total plus Input cost (Cached/Uncached with their token sizes) and
  Output cost (Uncached with its size; OpenCode has no output cache). The split is
  computed from per-message costs and the models.dev pricing catalog cached
  locally by OpenCode. When the export records zero cost but tokens exist, the
  total falls back to the catalog estimate.
- User prompts rendered as Markdown, with attached files and mentions.
- Assistant messages with collapsible reasoning blocks and tool calls.
- Tool calls: input fields, output, error, exit code and completion status. Each
  tool has an inline SVG icon (unknown tools use the default `tool.svg`); all
  icons live in a sprite embedded at the top of the page, with no external files.
  The tool-call card lists each tool with its count and approximate cost (the
  message cost is split evenly across its tool calls).
- Shell `command` inputs get token-level syntax highlighting: commands, flags,
  strings, variables, operators/pipes and comments.
- Markdown rendering (tables, lists, code blocks…), with raw HTML stripped.
- Client-side filter (`/`), expand/collapse all, copy buttons for ids and code.
- Responsive layout, dark/light theme based on the OS preference, print styles.
- Everything is embedded in the generated file except the Science Gothic font
  stylesheet loaded from Google Fonts; without network the page falls back to
  system fonts.

## Notes

- Requires the `opencode` CLI in `PATH`.
- "All projects" mode reads the session index from OpenCode's local database
  (`opencode.db`) in read-only mode. Project mode uses `opencode session list`.
- Cost breakdown reads the cached models.dev catalog from the same database. If
  pricing is missing for a model, the card shows the total without the split.
- Only data present in the export is shown; tool outputs are truncated at 120k
  characters per block to keep pages reasonable.
- The server binds to `127.0.0.1` only.

## License

MIT
