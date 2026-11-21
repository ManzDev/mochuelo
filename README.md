# oc-observe

Render an [OpenCode](https://opencode.ai) session as a beautiful, self-contained HTML page.

`oc-observe` runs `opencode session export <id>`, parses the JSON, and writes a single
`.html` file (styles and scripts embedded) that you can open, archive, share, or print.

## Install

```sh
cargo install --path .
```

## Usage

```sh
oc-observe ses_e09db3cc0ffeioobS2SsdTJN76
```

Writes `ses_e09db3cc0ffeioobS2SsdTJN76.html` in the current directory and prints its path.

```sh
oc-observe <session-id> -o report.html   # custom output path
oc-observe <session-id> --open           # open in the default browser
oc-observe <session-id> --stdout         # print HTML to stdout
oc-observe export.json                   # render a previously exported .json file
opencode session export <id> | oc-observe -   # read export JSON from stdin
```

## What the page shows

- Session header: title, id, agent, model, project directory, start time, wall time.
- Stats: message count, tool calls, cost, input/output/reasoning tokens, cache reads.
- User prompts rendered as Markdown, with attached files and mentions.
- Assistant messages with collapsible reasoning blocks and tool calls.
- Tool calls: input fields, output, error, exit code, completion status.
- Markdown rendering for messages (tables, lists, code blocks…), with raw HTML stripped.
- Client-side filter (`/`), expand/collapse all, and copy buttons for ids and code.
- Responsive layout, dark/light theme based on the OS preference, print styles.

## Notes

- Requires the `opencode` CLI in `PATH` when rendering by session id.
- Only data present in the export is shown; huge tool outputs are truncated at 120k
  characters per block to keep the file reasonable.
- No network requests: the generated page is fully standalone.

## License

MIT
