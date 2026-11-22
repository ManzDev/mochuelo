use std::fmt::Write as _;

const ICONS: [(&str, &str); 14] = [
    ("edit", include_str!("assets/edit.svg")),
    ("execute", include_str!("assets/execute.svg")),
    (
        "experimental_image_generate",
        include_str!("assets/experimental_image_generate.svg"),
    ),
    ("glob", include_str!("assets/glob.svg")),
    ("grep", include_str!("assets/grep.svg")),
    ("question", include_str!("assets/question.svg")),
    ("read", include_str!("assets/read.svg")),
    ("shell", include_str!("assets/shell.svg")),
    ("skill", include_str!("assets/skill.svg")),
    ("subagent", include_str!("assets/subagent.svg")),
    ("tool", include_str!("assets/tool.svg")),
    ("webfetch", include_str!("assets/webfetch.svg")),
    ("websearch", include_str!("assets/websearch.svg")),
    ("write", include_str!("assets/write.svg")),
];

/// Inline SVG sprite with one symbol per known tool. Every symbol reuses the
/// original icon paths, so the generated page never references external files.
pub fn sprite() -> String {
    let mut out = String::from(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" style=\"display:none\" aria-hidden=\"true\">",
    );

    for (name, raw) in ICONS {
        let inner_start = raw.find('>').map(|idx| idx + 1).unwrap_or(0);
        let inner_end = raw.rfind("</svg>").unwrap_or(raw.len());
        let view_box = view_box(raw).unwrap_or("0 0 24 24");
        let _ = write!(
            out,
            "<symbol id=\"icon-{name}\" viewBox=\"{view_box}\">{}</symbol>",
            &raw[inner_start..inner_end]
        );
    }

    out.push_str("</svg>");
    out
}

/// Tool name to symbol id. Unknown tools (MCP, plugins, bash…) use the default.
pub fn icon_id(tool: &str) -> &'static str {
    match tool {
        "edit" => "edit",
        "execute" => "execute",
        "experimental_image_generate" => "experimental_image_generate",
        "glob" => "glob",
        "grep" => "grep",
        "question" => "question",
        "read" => "read",
        "shell" => "shell",
        "skill" => "skill",
        "subagent" => "subagent",
        "webfetch" => "webfetch",
        "websearch" => "websearch",
        "write" => "write",
        _ => "tool",
    }
}

fn view_box(raw: &str) -> Option<&str> {
    let start = raw.find("viewBox=\"")? + "viewBox=\"".len();
    let end = raw[start..].find('"')? + start;
    Some(&raw[start..end])
}
