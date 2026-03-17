use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;

const INDENT: &str = "  ";

const KEY_STYLE: Style = Style::new().fg(Color::LightCyan);
const STRING_STYLE: Style = Style::new().fg(Color::Green);
const NUMBER_STYLE: Style = Style::new().fg(Color::Yellow);
const BOOL_NULL_STYLE: Style = Style::new().fg(Color::Magenta);
const PUNCT_STYLE: Style = Style::new().fg(Color::DarkGray);

pub fn json_to_lines(value: &Value) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    render_value(value, 0, false, &mut lines);
    lines
}

fn render_value(value: &Value, depth: usize, trailing_comma: bool, lines: &mut Vec<Line<'static>>) {
    let comma = if trailing_comma { "," } else { "" };
    let prefix = INDENT.repeat(depth);

    match value {
        Value::Null => {
            lines.push(Line::from(vec![
                Span::raw(prefix),
                Span::styled("null", BOOL_NULL_STYLE),
                Span::styled(comma, PUNCT_STYLE),
            ]));
        }
        Value::Bool(b) => {
            lines.push(Line::from(vec![
                Span::raw(prefix),
                Span::styled(b.to_string(), BOOL_NULL_STYLE),
                Span::styled(comma, PUNCT_STYLE),
            ]));
        }
        Value::Number(n) => {
            lines.push(Line::from(vec![
                Span::raw(prefix),
                Span::styled(n.to_string(), NUMBER_STYLE),
                Span::styled(comma, PUNCT_STYLE),
            ]));
        }
        Value::String(s) => {
            lines.push(Line::from(vec![
                Span::raw(prefix),
                Span::styled(format!("\"{}\"", escape_json_string(s)), STRING_STYLE),
                Span::styled(comma, PUNCT_STYLE),
            ]));
        }
        Value::Array(arr) => {
            if arr.is_empty() {
                lines.push(Line::from(vec![
                    Span::raw(prefix),
                    Span::styled(format!("[]{comma}"), PUNCT_STYLE),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::raw(prefix.clone()),
                    Span::styled("[", PUNCT_STYLE),
                ]));
                for (i, item) in arr.iter().enumerate() {
                    render_value(item, depth + 1, i + 1 < arr.len(), lines);
                }
                lines.push(Line::from(vec![
                    Span::raw(prefix),
                    Span::styled(format!("]{comma}"), PUNCT_STYLE),
                ]));
            }
        }
        Value::Object(obj) => {
            if obj.is_empty() {
                lines.push(Line::from(vec![
                    Span::raw(prefix),
                    Span::styled(format!("{{}}{comma}"), PUNCT_STYLE),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::raw(prefix.clone()),
                    Span::styled("{", PUNCT_STYLE),
                ]));
                let mut keys: Vec<&String> = obj.keys().collect();
                keys.sort();
                let count = keys.len();
                for (i, key) in keys.into_iter().enumerate() {
                    let val = &obj[key];
                    let has_comma = i + 1 < count;
                    let child_prefix = INDENT.repeat(depth + 1);
                    if matches!(val, Value::Object(_) | Value::Array(_)) && !is_empty_container(val)
                    {
                        lines.push(Line::from(vec![
                            Span::raw(child_prefix),
                            Span::styled(format!("\"{}\"", escape_json_string(key)), KEY_STYLE),
                            Span::styled(": ", PUNCT_STYLE),
                        ]));
                        render_value_inline_open(val, depth + 1, has_comma, lines);
                    } else {
                        let mut spans = vec![
                            Span::raw(child_prefix),
                            Span::styled(format!("\"{}\"", escape_json_string(key)), KEY_STYLE),
                            Span::styled(": ", PUNCT_STYLE),
                        ];
                        append_inline_value(val, has_comma, &mut spans);
                        lines.push(Line::from(spans));
                    }
                }
                lines.push(Line::from(vec![
                    Span::raw(prefix),
                    Span::styled(format!("}}{comma}"), PUNCT_STYLE),
                ]));
            }
        }
    }
}

fn render_value_inline_open(
    value: &Value,
    depth: usize,
    trailing_comma: bool,
    lines: &mut Vec<Line<'static>>,
) {
    let comma = if trailing_comma { "," } else { "" };
    let prefix = INDENT.repeat(depth);
    match value {
        Value::Array(arr) => {
            if let Some(last_line) = lines.last_mut() {
                last_line.spans.push(Span::styled("[", PUNCT_STYLE));
            }
            for (i, item) in arr.iter().enumerate() {
                render_value(item, depth + 1, i + 1 < arr.len(), lines);
            }
            lines.push(Line::from(vec![
                Span::raw(prefix),
                Span::styled(format!("]{comma}"), PUNCT_STYLE),
            ]));
        }
        Value::Object(obj) => {
            if let Some(last_line) = lines.last_mut() {
                last_line.spans.push(Span::styled("{", PUNCT_STYLE));
            }
            let mut keys: Vec<&String> = obj.keys().collect();
            keys.sort();
            let count = keys.len();
            for (i, key) in keys.into_iter().enumerate() {
                let val = &obj[key];
                let has_comma = i + 1 < count;
                let child_prefix = INDENT.repeat(depth + 1);
                if matches!(val, Value::Object(_) | Value::Array(_)) && !is_empty_container(val) {
                    lines.push(Line::from(vec![
                        Span::raw(child_prefix),
                        Span::styled(format!("\"{}\"", escape_json_string(key)), KEY_STYLE),
                        Span::styled(": ", PUNCT_STYLE),
                    ]));
                    render_value_inline_open(val, depth + 1, has_comma, lines);
                } else {
                    let mut spans = vec![
                        Span::raw(child_prefix),
                        Span::styled(format!("\"{}\"", escape_json_string(key)), KEY_STYLE),
                        Span::styled(": ", PUNCT_STYLE),
                    ];
                    append_inline_value(val, has_comma, &mut spans);
                    lines.push(Line::from(spans));
                }
            }
            lines.push(Line::from(vec![
                Span::raw(prefix),
                Span::styled(format!("}}{comma}"), PUNCT_STYLE),
            ]));
        }
        _ => {
            render_value(value, depth, trailing_comma, lines);
        }
    }
}

fn append_inline_value(value: &Value, trailing_comma: bool, spans: &mut Vec<Span<'static>>) {
    let comma = if trailing_comma { "," } else { "" };
    match value {
        Value::Null => {
            spans.push(Span::styled("null", BOOL_NULL_STYLE));
            spans.push(Span::styled(comma, PUNCT_STYLE));
        }
        Value::Bool(b) => {
            spans.push(Span::styled(b.to_string(), BOOL_NULL_STYLE));
            spans.push(Span::styled(comma, PUNCT_STYLE));
        }
        Value::Number(n) => {
            spans.push(Span::styled(n.to_string(), NUMBER_STYLE));
            spans.push(Span::styled(comma, PUNCT_STYLE));
        }
        Value::String(s) => {
            spans.push(Span::styled(
                format!("\"{}\"", escape_json_string(s)),
                STRING_STYLE,
            ));
            spans.push(Span::styled(comma, PUNCT_STYLE));
        }
        Value::Array(arr) if arr.is_empty() => {
            spans.push(Span::styled(format!("[]{comma}"), PUNCT_STYLE));
        }
        Value::Object(obj) if obj.is_empty() => {
            spans.push(Span::styled(format!("{{}}{comma}"), PUNCT_STYLE));
        }
        _ => {
            spans.push(Span::raw("..."));
        }
    }
}

fn is_empty_container(value: &Value) -> bool {
    matches!(value, Value::Array(a) if a.is_empty())
        || matches!(value, Value::Object(o) if o.is_empty())
}

fn escape_json_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}
